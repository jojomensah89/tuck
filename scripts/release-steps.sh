#!/usr/bin/env bash
# The git and GitHub steps of .github/workflows/release.yml. They live here so that
# tests/release-steps.test.ts runs these exact commands against a local bare remote.
#
#   is-current <branch> <sha>                  prints true when <sha> is still the branch tip
#   commit-bump <version> <source-sha> <dir>   commits a prepared version-only PR locally
#   prepare-pr <version> <source-sha> <notes> <dir>  pushes the version branch and opens checks
#   current-notes <version> <notes-file>       writes notes for an already merged version
#   verify-clean                               fails if the build changed a tracked file
#   finish-pending                             publishes a release left as a draft after its tag was pushed
#   publish <version> <source-sha> <dir>       draft, upload, atomic push, publish
#
# finish-pending and publish refuse to run unless RELEASE_LIVE=true.
set -euo pipefail

RETRY_DELAY="${RETRY_DELAY:-10}"
VERSION_FILES="CHANGELOG.md package.json src-tauri/Cargo.lock src-tauri/Cargo.toml src-tauri/tauri.conf.json"

die() {
  echo "::error::$*" >&2
  exit 1
}

require_live() {
  [ "${RELEASE_LIVE:-}" = "true" ] || die "Refusing to touch the remote: RELEASE_LIVE is not true."
}

retry() {
  local attempt=1
  until "$@"; do
    [ "$attempt" -lt 3 ] || return 1
    echo "Attempt $attempt failed, retrying: $1" >&2
    sleep $((attempt * RETRY_DELAY))
    attempt=$((attempt + 1))
  done
}

remote_head() {
  git ls-remote origin "refs/heads/$1" | cut -f1
}

# ls-remote exits 2 only when the tag is missing. Anything else is a failed read, which must not
# look like a missing tag, because that would delete a draft.
remote_tag_exists() {
  local status=0
  git ls-remote --exit-code origin "refs/tags/$1" >/dev/null || status=$?
  case "$status" in
    0) return 0 ;;
    2) return 1 ;;
    *) die "Could not read tags from origin (git exited $status)." ;;
  esac
}

remote_tag_sha() {
  local tag=$1 lines peeled
  lines=$(git ls-remote origin "refs/tags/$tag" "refs/tags/$tag^{}") ||
    die "Could not read tag $tag from origin."
  peeled=$(awk '$2 ~ /\^\{\}$/ { print $1; exit }' <<<"$lines")
  if [ -n "$peeled" ]; then
    echo "$peeled"
  else
    awk 'NR == 1 { print $1; exit }' <<<"$lines"
  fi
}

# Prints draft, published or none. A network failure stops the script instead of reading as none.
release_state() {
  local out
  if out=$(gh release view "$1" --json isDraft --jq .isDraft 2>&1); then
    if [ "$out" = "true" ]; then echo draft; else echo published; fi
  elif grep -qi "release not found" <<<"$out"; then
    echo none
  else
    echo "$out" >&2
    return 1
  fi
}

# A draft whose tag never reached the remote belongs to a run that stopped before its push.
delete_orphan_drafts() {
  local expected=${1:-} drafts tag
  drafts=$(gh release list --json tagName,isDraft --jq '.[] | select(.isDraft) | .tagName')
  for tag in $drafts; do
    [ -z "$expected" ] || [ "$tag" = "$expected" ] || continue
    if ! remote_tag_exists "$tag"; then
      echo "Deleting draft $tag: its tag was never pushed."
      gh release delete "$tag" --yes
    fi
  done
}

create_draft() {
  local tag=$1 dir=$2 installer=$3
  delete_orphan_drafts "$tag"
  gh release create "$tag" --draft --title "Tuck $tag" --notes-file "$dir/release-notes.md" \
    "$dir/$installer" "$dir/$installer.sig" "$dir/latest.json"
}

python_bin() {
  command -v python 2>/dev/null || command -v python3 2>/dev/null ||
    die "Python 3 is required to validate release metadata."
}

validate_draft_assets() {
  local tag=$1 version=${1#v} assets installer sig temp repo
  assets=$(gh release view "$tag" --json assets --jq '.assets[].name')
  installer="Tuck_${version}_Setup.exe"
  if ! grep -Fxq "$installer" <<<"$assets"; then
    installer="tuck_${version}_x64-setup.exe"
    grep -Fxq "$installer" <<<"$assets" || die "$tag draft is missing the exact installer for $version."
  fi
  sig="$installer.sig"
  grep -Fxq "$sig" <<<"$assets" || die "$tag draft is missing the exact signature $sig."
  grep -Fxq 'latest.json' <<<"$assets" || die "$tag draft is missing the exact latest.json asset."

  temp=$(mktemp -d "${TMPDIR:-/tmp}/tuck-release-assets.XXXXXX")
  gh release download "$tag" --pattern latest.json --pattern "$sig" --dir "$temp" --clobber
  repo=$(gh repo view --json nameWithOwner --jq .nameWithOwner)
  "$(python_bin)" - "$temp/latest.json" "$temp/$sig" "$version" "$tag" "$repo" "$installer" <<'PY'
import json
import pathlib
import sys

manifest_path, signature_path, version, tag, repo, installer = sys.argv[1:]
try:
    manifest = json.loads(pathlib.Path(manifest_path).read_text(encoding="utf-8"))
except Exception as error:
    raise SystemExit(f"latest.json is not valid JSON: {error}")
if manifest.get("version") != version:
    raise SystemExit(f"latest.json has version {manifest.get('version')!r}, expected {version!r}")
platforms = manifest.get("platforms")
if not isinstance(platforms, dict):
    raise SystemExit("latest.json has no platforms object")
expected_url = f"https://github.com/{repo}/releases/download/{tag}/{installer}"
expected_signature = pathlib.Path(signature_path).read_text(encoding="utf-8").strip()
for platform in ("windows-x86_64", "windows-x86_64-nsis"):
    entry = platforms.get(platform)
    if not isinstance(entry, dict):
        raise SystemExit(f"latest.json is missing {platform}")
    if entry.get("url") != expected_url:
        raise SystemExit(f"{platform} URL is {entry.get('url')!r}, expected {expected_url!r}")
    if not entry.get("signature") or entry["signature"].strip() != expected_signature:
        raise SystemExit(f"{platform} signature does not match {installer}.sig")
PY
  rm -rf "$temp"
  echo "$tag draft assets and updater manifest are valid."
}

publish_draft() {
  retry gh release edit "$1" --draft=false --latest ||
    die "$1 is tagged and uploaded but still a draft. Re-run this workflow: it finishes the publish without building."
}

feed_url() {
  grep -o 'https://[^"]*latest\.json' src-tauri/tauri.conf.json
}

# Whether the feed an installed Tuck fetches serves this version.
feed_serves() {
  local version=$1 url body attempt
  url=$(feed_url)
  # GitHub can serve the previous release for a moment after publishing
  for attempt in 1 2 3 4 5 6; do
    body=$("${RELEASE_CURL:-curl}" -fsSL "$url") || body=""
    if [[ "$body" == *"\"version\": \"$version\""* ]]; then
      echo "$url serves $version."
      return 0
    fi
    [ "$attempt" = 6 ] || sleep "$RETRY_DELAY"
  done
  return 1
}

# For the run that did the publishing: a feed that doesn't update means the release failed.
verify_live() {
  feed_serves "$1" || die "$(feed_url) does not serve version $1."
}

# whisper.cpp otherwise compiles for the build machine's CPU, and a release built on an AVX-512
# runner would crash on any PC without it. Fails the release instead of shipping that.
verify_cpu_baseline() {
  local caches=(src-tauri/target/release/build/whisper-rs-sys-*/out/build/CMakeCache.txt) cache setting
  [ -f "${caches[0]}" ] || die "No whisper.cpp build found to check the CPU baseline of."
  for cache in "${caches[@]}"; do
    for setting in GGML_NATIVE:BOOL=OFF GGML_AVX2:BOOL=ON GGML_AVX512:BOOL=OFF; do
      grep -qx "$setting" "$cache" || die "$cache does not have $setting."
    done
  done
  echo "whisper.cpp is built for the AVX2 baseline."
}

cmd_is_current() {
  local tip
  tip=$(remote_head "$1")
  if [ "$tip" = "$2" ]; then echo true; else echo false; fi
}

cmd_commit_bump() {
  local version=$1 source_sha=$2 dir=$3 file
  git config user.name "github-actions[bot]"
  git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
  # shellcheck disable=SC2086
  git add $VERSION_FILES
  git commit --quiet -m "chore(release): v$version"

  # The checks ran on source_sha, so the release commit may change nothing else
  for file in $(git diff --name-only "$source_sha" HEAD); do
    case " $VERSION_FILES " in
      *" $file "*) ;;
      *) die "The release commit changes $file, which is not a version file." ;;
    esac
  done
  [ -z "$(git status --porcelain)" ] || die "Uncommitted changes before the build: $(git status --porcelain)"

  mkdir -p "$dir"
  git branch --force release-candidate HEAD
  git bundle create "$dir/release.bundle" release-candidate "^$source_sha"
}

cmd_prepare_pr() {
  require_live
  local version=$1 source_sha=$2 notes_file=$3 dir=$4 branch pr tip remote_candidate local_candidate local_tree remote_tree
  [ -n "$version" ] && [ -n "$source_sha" ] && [ -n "$notes_file" ] && [ -n "$dir" ] ||
    die "prepare-pr needs <version> <source-sha> <notes-file> <dir>"
  branch="release/v${version}-${source_sha:0:12}"
  tip=$(remote_head main)
  [ "$tip" = "$source_sha" ] || die "main moved to $tip while preparing $branch; no PR was created."
  cmd_commit_bump "$version" "$source_sha" "$dir"
  local_candidate=$(git rev-parse HEAD)
  local_tree=$(git rev-parse HEAD^{tree})
  remote_candidate=$(git ls-remote origin "refs/heads/$branch" | cut -f1)
  if [ -n "$remote_candidate" ]; then
    git fetch --quiet origin "refs/heads/$branch"
    remote_candidate=$(git rev-parse FETCH_HEAD)
    [ "$(git rev-parse "$remote_candidate^")" = "$source_sha" ] ||
      die "Existing $branch does not have $source_sha as its parent; refusing to force or replace it."
    remote_tree=$(git rev-parse "$remote_candidate^{tree}")
    [ "$remote_tree" = "$local_tree" ] ||
      die "Existing $branch has a different prepared tree; refusing to force or replace it."
    local_candidate="$remote_candidate"
  else
    tip=$(remote_head main)
    [ "$tip" = "$source_sha" ] || die "main moved to $tip while preparing $branch; no PR was created."
    git push origin "HEAD:refs/heads/$branch"
    remote_candidate=$(git ls-remote origin "refs/heads/$branch" | cut -f1)
    [ "$remote_candidate" = "$local_candidate" ] || die "Prepared branch $branch did not retain its expected commit."
  fi

  local body_file repo pr_file candidate_number candidate_oid candidate_body stale_number
  body_file=$(mktemp "${TMPDIR:-/tmp}/tuck-release-pr.XXXXXX")
  printf '<!-- tuck-release source=%s version=%s -->\n\n' "$source_sha" "$version" > "$body_file"
  cat "$notes_file" >> "$body_file"
  repo=$(gh repo view --json nameWithOwner --jq .nameWithOwner)
  pr_file=$(mktemp "${TMPDIR:-/tmp}/tuck-release-prs.XXXXXX")
  gh pr list --base main --state open --limit 100 --json number,headRefName,headRefOid,headRepository,body > "$pr_file"
  while IFS=$'\t' read -r kind number oid marker; do
    [ -n "$kind" ] || continue
    if [ "$kind" = candidate ]; then
      candidate_number=$number
      candidate_oid=$oid
      candidate_body=${marker%$'\r'}
    elif [ "$kind" = stale ]; then
      tip=$(remote_head main)
      [ "$tip" = "$source_sha" ] || die "main moved to $tip while closing stale release PRs; no PR was changed."
      gh pr close "$number"
    fi
  done < <("$(python_bin)" - "$pr_file" "$repo" "$branch" "$source_sha" "$version" <<'PY'
import json
import re
import sys

path, repo, branch, source, version = sys.argv[1:]
prs = json.load(open(path, encoding="utf-8"))
marker = f"<!-- tuck-release source={source} version={version} -->"
for pr in prs:
    head = pr.get("headRepository") or {}
    if head.get("nameWithOwner") != repo:
        continue
    name = pr.get("headRefName") or ""
    body = pr.get("body") or ""
    number = str(pr.get("number"))
    oid = pr.get("headRefOid") or ""
    if name == branch:
        print("candidate\t{}\t{}\t{}".format(number, oid, "1" if marker in body else "0"))
    elif re.fullmatch(r"release/v\d+\.\d+\.\d+-[0-9a-fA-F]{12}", name) and "<!-- tuck-release" in body:
        print("stale\t{}\t{}\t1".format(number, oid))
PY
)
  rm -f "$pr_file"
  if [ -n "${candidate_number:-}" ]; then
    [ "${candidate_oid:-}" = "$remote_candidate" ] ||
      die "Existing release PR #$candidate_number points at ${candidate_oid:-<missing>}, expected $remote_candidate."
    [ "${candidate_body:-}" = 1 ] ||
      die "Existing release PR #$candidate_number is not a Tuck release PR; refusing to reuse it."
  else
    tip=$(remote_head main)
    [ "$tip" = "$source_sha" ] || die "main moved to $tip before release PR creation; no PR was created."
    gh pr create --base main --head "$branch" --title "chore(release): v$version" --body-file "$body_file"
  fi
  rm -f "$body_file"

  # Re-read the PR head after creation/reuse, then dispatch checks only for the
  # verified branch commit. A provider-side update can still race this read.
  pr_file=$(mktemp "${TMPDIR:-/tmp}/tuck-release-prs.XXXXXX")
  gh pr list --base main --state open --limit 100 --json number,headRefName,headRefOid,headRepository,body > "$pr_file"
  candidate_oid=$("$(python_bin)" - "$pr_file" "$repo" "$branch" <<'PY'
import json
import sys

path, repo, branch = sys.argv[1:]
for pr in json.load(open(path, encoding="utf-8")):
    if pr.get("headRefName") == branch and (pr.get("headRepository") or {}).get("nameWithOwner") == repo:
        print(pr.get("headRefOid") or "")
        break
PY
)
  rm -f "$pr_file"
  [ "$candidate_oid" = "$remote_candidate" ] || die "Release PR $branch does not point at verified commit $remote_candidate."
  tip=$(remote_head main)
  [ "$tip" = "$source_sha" ] || die "main moved to $tip before check dispatch; no check was dispatched."
  # GITHUB_TOKEN-created PRs may not emit pull_request workflows; dispatch the
  # read-only Windows checks explicitly on the exact prepared branch.
  gh workflow run check.yml --ref "$branch"
}

cmd_current_notes() {
  local version=$1 notes_file=$2 line started=0
  [ -n "$version" ] && [ -n "$notes_file" ] || die "current-notes needs <version> <notes-file>"
  [ -f CHANGELOG.md ] || die "CHANGELOG.md is missing"
  : > "$notes_file"
  while IFS= read -r line || [ -n "$line" ]; do
    if [[ "$line" == "## [$version]"* ]]; then
      started=1
      continue
    fi
    if [ "$started" = 1 ] && [[ "$line" == '## ['* ]]; then
      break
    fi
    if [ "$started" = 1 ]; then
      printf '%s\n' "$line" >> "$notes_file"
    fi
  done < CHANGELOG.md
  sed -i '/^[[:space:]]*$/d' "$notes_file"
  [ -s "$notes_file" ] || die "No changelog notes found for v$version"
}

cmd_verify_clean() {
  local dirty
  dirty=$(git status --porcelain)
  [ -z "$dirty" ] || die "The build changed tracked files, so the tag would not match what was built: $dirty"
}

cmd_finish_pending() {
  require_live
  git fetch --quiet --tags origin main

  local tag state subject assets missing
  tag=$(git tag --list 'v*' --merged FETCH_HEAD --sort=-v:refname | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' | head -n 1 || true)
  [ -n "$tag" ] || { echo "No version tag yet."; return 0; }

  state=$(release_state "$tag")
  case "$state" in
    published)
      echo "$tag is already published."
      # An earlier run may have stopped before confirming this. Only a warning: pulling a bad
      # release legitimately changes what the feed serves, and must not block the fix.
      feed_serves "${tag#v}" ||
        echo "::warning::$(feed_url) does not serve ${tag#v}. Expected only if that release was pulled. See Recovery in docs/releasing.md."
      ;;
    none)
      subject=$(git log -1 --format=%s "$tag")
      case "$subject" in
        "chore(release): "*) die "$tag was released from CI but has no GitHub release. See Recovery in docs/releasing.md." ;;
      esac
      echo "$tag is a baseline tag with no release."
      ;;
    draft)
      validate_draft_assets "$tag"
      echo "Finishing $tag: tagged and uploaded by an earlier run, still a draft."
      publish_draft "$tag"
      verify_live "${tag#v}"
      ;;
  esac
}

cmd_publish() {
  require_live
  local version=$1 source_sha=$2 dir=$3
  local tag="v$version" installer="Tuck_${version}_Setup.exe" tip release_sha tag_sha

  tip=$(remote_head main)
  if [ "$tip" != "$source_sha" ]; then
    echo "::notice::main moved to $tip while this run built $source_sha. The run for the newer commit releases everything since the last tag."
    return 0
  fi
  if remote_tag_exists "$tag"; then
    tag_sha=$(remote_tag_sha "$tag")
    [ "$tag_sha" = "$source_sha" ] ||
      die "$tag points at $tag_sha, but this build is $source_sha. Refusing to reuse the wrong tag."
    case "$(release_state "$tag")" in
      published) verify_live "${tag#v}"; return 0 ;;
      draft) validate_draft_assets "$tag"; publish_draft "$tag"; verify_live "${tag#v}"; return 0 ;;
      *) die "Tag $tag already exists but has no GitHub release. See Recovery in docs/releasing.md." ;;
    esac
  fi

  git fetch --quiet origin main
  git checkout --quiet --detach FETCH_HEAD
  release_sha=$(git rev-parse HEAD)
  [ "$release_sha" = "$source_sha" ] || die "The built commit is not the current main commit."

  retry create_draft "$tag" "$dir" "$installer" || die "Could not upload the $tag draft. Nothing was pushed. Re-run this workflow."
  validate_draft_assets "$tag"

  tip=$(remote_head main)
  if [ "$tip" != "$source_sha" ]; then
    echo "::notice::main moved during release preparation, so the draft was removed and no tag was pushed."
    delete_orphan_drafts "$tag"
    return 0
  fi
  git tag "$tag"
  if ! git push --atomic origin "$tag"; then
    die "The release draft exists but the tag was not pushed. Re-run this workflow to finish it."
  fi

  publish_draft "$tag"
  verify_live "$version"
}

command=${1:-}
[ $# -gt 0 ] && shift
case "$command" in
  is-current) cmd_is_current "$@" ;;
  commit-bump) cmd_commit_bump "$@" ;;
  prepare-pr) cmd_prepare_pr "$@" ;;
  current-notes) cmd_current_notes "$@" ;;
  verify-clean) cmd_verify_clean ;;
  verify-cpu-baseline) verify_cpu_baseline ;;
  finish-pending) cmd_finish_pending ;;
  publish) cmd_publish "$@" ;;
  *) die "Unknown command: $command" ;;
esac
