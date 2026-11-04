#!/usr/bin/env bash
# The git and GitHub steps of .github/workflows/release.yml. They live here so that
# tests/release-steps.test.ts runs these exact commands against a local bare remote.
#
#   is-current <branch> <sha>                  prints true when <sha> is still the branch tip
#   commit-bump <version> <source-sha> <dir>   commits the bump locally, bundles it into <dir>
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

remote_tag_exists() {
  git ls-remote --exit-code origin "refs/tags/$1" >/dev/null
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
  local drafts tag
  drafts=$(gh release list --json tagName,isDraft --jq '.[] | select(.isDraft) | .tagName')
  for tag in $drafts; do
    if ! remote_tag_exists "$tag"; then
      echo "Deleting draft $tag: its tag was never pushed."
      gh release delete "$tag" --yes
    fi
  done
}

create_draft() {
  local tag=$1 dir=$2 installer=$3
  delete_orphan_drafts
  gh release create "$tag" --draft --title "Tuck $tag" --notes-file "$dir/release-notes.md" \
    "$dir/$installer" "$dir/$installer.sig" "$dir/latest.json"
}

publish_draft() {
  retry gh release edit "$1" --draft=false --latest ||
    die "$1 is tagged and uploaded but still a draft. Re-run this workflow: it finishes the publish without building."
}

# Confirms what an installed Tuck will actually fetch.
verify_live() {
  local version=$1 url
  url=$(grep -o 'https://[^"]*latest\.json' src-tauri/tauri.conf.json)
  "${RELEASE_CURL:-curl}" -fsSL --retry 5 --retry-delay "$RETRY_DELAY" --retry-all-errors "$url" | grep -q "\"version\": \"$version\"" ||
    die "$url does not serve version $version."
  echo "$url serves $version."
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
  git commit --quiet -m "chore(release): v$version [skip ci]"

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
      ;;
    none)
      subject=$(git log -1 --format=%s "$tag")
      case "$subject" in
        "chore(release): "*) die "$tag was released from CI but has no GitHub release. See Recovery in docs/releasing.md." ;;
      esac
      echo "$tag is a baseline tag with no release."
      ;;
    draft)
      assets=$(gh release view "$tag" --json assets --jq '.assets[].name')
      missing=""
      grep -q '_x64-setup\.exe$' <<<"$assets" || missing="$missing installer"
      grep -q '_x64-setup\.exe\.sig$' <<<"$assets" || missing="$missing signature"
      grep -q '^latest\.json$' <<<"$assets" || missing="$missing latest.json"
      [ -z "$missing" ] || die "The $tag draft is missing:$missing. See Recovery in docs/releasing.md."
      echo "Finishing $tag: tagged and uploaded by an earlier run, still a draft."
      publish_draft "$tag"
      verify_live "${tag#v}"
      ;;
  esac
}

cmd_publish() {
  require_live
  local version=$1 source_sha=$2 dir=$3
  local tag="v$version" installer="tuck_${version}_x64-setup.exe" tip release_sha

  tip=$(remote_head main)
  if [ "$tip" != "$source_sha" ]; then
    echo "::notice::main moved to $tip while this run built $source_sha. The run for the newer commit releases everything since the last tag."
    return 0
  fi
  if remote_tag_exists "$tag"; then
    die "Tag $tag already exists but main has no release commit for it. See Recovery in docs/releasing.md."
  fi

  git fetch --quiet "$dir/release.bundle" release-candidate
  git checkout --quiet --detach FETCH_HEAD
  release_sha=$(git rev-parse HEAD)
  [ "$(git rev-parse HEAD^)" = "$source_sha" ] || die "The built commit is not a child of $source_sha."
  [ "$(git log -1 --format=%s)" = "chore(release): $tag [skip ci]" ] || die "The built commit is not the $tag release commit."

  retry create_draft "$tag" "$dir" "$installer" || die "Could not upload the $tag draft. Nothing was pushed. Re-run this workflow."

  git tag --force "$tag" >/dev/null
  if ! git push --atomic origin HEAD:main "$tag"; then
    tip=$(remote_head main)
    if [ "$tip" = "$source_sha" ]; then
      die "Push failed and main did not move. Nothing was pushed. Re-run this workflow."
    elif [ "$tip" != "$release_sha" ]; then
      echo "::notice::main moved to $tip during the push, so neither main nor $tag changed. The run for the newer commit releases everything since the last tag."
      delete_orphan_drafts
      return 0
    fi
    echo "The push reported an error but main is at the release commit. Continuing."
  fi

  publish_draft "$tag"
  verify_live "$version"
}

command=${1:-}
[ $# -gt 0 ] && shift
case "$command" in
  is-current) cmd_is_current "$@" ;;
  commit-bump) cmd_commit_bump "$@" ;;
  verify-clean) cmd_verify_clean ;;
  finish-pending) cmd_finish_pending ;;
  publish) cmd_publish "$@" ;;
  *) die "Unknown command: $command" ;;
esac
