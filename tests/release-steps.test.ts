import { afterAll, describe, expect, it, setDefaultTimeout } from 'bun:test';
import { spawnSync } from 'node:child_process';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';

/**
 * Runs scripts/release-steps.sh, the commands the release workflow runs, against a real local
 * bare remote. `gh` and `curl` are stand-ins that record state in files, so this proves the
 * ordering and recovery logic, not GitHub's own behaviour.
 */
// Each scenario starts many git and bash processes, which is slow on Windows
setDefaultTimeout(180_000);

const SCRIPT = path.resolve(import.meta.dir, '..', 'scripts', 'release-steps.sh');
const windowsGitBash = path.join(process.env.ProgramFiles ?? 'C:\\Program Files', 'Git', 'bin', 'bash.exe');
const BASH = process.platform === 'win32' && fs.existsSync(windowsGitBash) ? windowsGitBash : 'bash';

const GH_STUB = `#!/usr/bin/env bash
set -euo pipefail
state="$STUB_STATE"
if [ "$1" = pr ]; then
  sub=$2; shift 2
  echo "gh pr $sub $*" >> "$state/gh.log"
  case "$sub" in
    list) [ -f "$state/prs.json" ] && cat "$state/prs.json" || echo '[]' ;;
    create)
      echo 1 > "$state/pr"
      body_file=""
      head_ref=""
      while [ $# -gt 0 ]; do
        case "$1" in
          --body-file) body_file=$2; shift 2 ;;
          --head) head_ref=$2; shift 2 ;;
          *) shift ;;
        esac
      done
      [ -z "$body_file" ] || cp "$body_file" "$state/pr-body.md"
      if [ -n "$STUB_REMOTE" ] && [ -n "$head_ref" ]; then
        head_oid=$(git --git-dir "$STUB_REMOTE" rev-parse "refs/heads/$head_ref")
        python - "$state/prs.json" "$head_ref" "$head_oid" "$body_file" <<'PY'
import json
import pathlib
import sys

path, head, oid, body_file = sys.argv[1:]
body = pathlib.Path(body_file).read_text(encoding="utf-8") if body_file else ""
json.dump([{"number": 1, "headRefName": head, "headRefOid": oid, "headRepository": {"nameWithOwner": "owner/tuck"}, "body": body}], open(path, "w", encoding="utf-8"))
PY
      fi
      ;;
    close) echo "$1" >> "$state/closed-prs" ;;
    *) echo "stub gh: unknown pr $sub" >&2; exit 2 ;;
  esac
  exit 0
fi
if [ "$1" = repo ] && [ "$2" = view ]; then
  echo owner/tuck
  exit 0
fi
if [ "$1" = workflow ] && [ "$2" = run ]; then
  echo "gh workflow run \${*:3}" >> "$state/gh.log"
  echo 1 > "$state/workflow-dispatch"
  exit 0
fi
[ "$1" = release ] || { echo "stub gh: $*" >&2; exit 2; }
sub=$2; shift 2
tag_arg=\${1:-}
dir="$state/releases/$tag_arg"
echo "gh release $sub $*" >> "$state/gh.log"
case "$sub" in
  view)
    [ -d "$dir" ] || { echo "release not found" >&2; exit 1; }
    case "$*" in
      *isDraft*) cat "$dir/draft" ;;
      *assets*) ls "$dir/assets" ;;
    esac ;;
  list)
    for d in "$state"/releases/*/; do
      [ -d "$d" ] || continue
      [ "$(cat "$d/draft")" = true ] && basename "$d"
    done
    true ;;
  create)
    [ ! -d "$dir" ] || { echo "stub: duplicate draft for $tag_arg" >> "$state/errors"; }
    if [ -f "$state/fail-create" ]; then echo "stub: upload failed" >&2; exit 1; fi
    mkdir -p "$dir/assets"; echo true > "$dir/draft"; shift
    while [ $# -gt 0 ]; do
      case "$1" in
        --draft) shift ;;
        --title|--notes-file) shift 2 ;;
        *) cp "$1" "$dir/assets/"; shift ;;
      esac
    done
    if [ -x "$state/on-create" ]; then "$state/on-create"; fi ;;
  edit)
    if [ -f "$state/fail-edit" ]; then echo "stub: api down" >&2; exit 1; fi
    echo false > "$dir/draft"
    cp "$dir/assets/latest.json" "$state/live-latest.json" ;;
  download)
    target_dir="."
    patterns=()
    while [ $# -gt 0 ]; do
      case "$1" in
        --pattern) patterns+=("$2"); shift 2 ;;
        --dir) target_dir=$2; shift 2 ;;
        --clobber) shift ;;
        *) shift ;;
      esac
    done
    mkdir -p "$target_dir"
    for pattern in "\${patterns[@]}"; do cp "$dir/assets/$pattern" "$target_dir/$pattern"; done ;;
  delete) rm -rf "$dir" ;;
  *) echo "stub gh: unknown $sub" >&2; exit 2 ;;
esac
`;

const CURL_STUB = `#!/usr/bin/env bash
cat "$STUB_STATE/live-latest.json"
`;

interface Rehearsal {
  root: string;
  remote: string;
  work: string;
  state: string;
  assets: string;
  source: string;
}

const roots: string[] = [];
afterAll(() => {
  for (const root of roots) fs.rmSync(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
});

function git(cwd: string, ...args: string[]): string {
  const result = spawnSync('git', args, { cwd, encoding: 'utf8' });
  if (result.status !== 0) throw new Error(`git ${args.join(' ')}: ${result.stderr}`);
  return result.stdout.trim();
}

function step(r: Rehearsal, live: boolean, ...args: string[]) {
  const env = {
    ...process.env,
    RELEASE_LIVE: live ? 'true' : 'false',
    RETRY_DELAY: '0',
    RELEASE_CURL: path.join(r.root, 'bin', 'curl').replaceAll('\\', '/'),
    STUB_STATE: r.state.replaceAll('\\', '/'),
    STUB_REMOTE: r.remote.replaceAll('\\', '/'),
    PATH: `${path.join(r.root, 'bin')}${path.delimiter}${process.env.PATH}`,
  };
  const result = spawnSync(BASH, [SCRIPT.replaceAll('\\', '/'), ...args], { cwd: r.work, env, encoding: 'utf8' });
  if (process.env.REHEARSAL_LOG) {
    fs.appendFileSync(process.env.REHEARSAL_LOG, `$ release-steps.sh ${args.join(' ')}\n[exit ${result.status}]\n${result.stdout}${result.stderr}\n`);
  }
  return result;
}

function commitAt(dir: string, message: string, ...body: string[]): string {
  const flags = [message, ...body].flatMap(part => ['-m', part]);
  git(dir, 'commit', '-q', '--allow-empty', ...flags);
  return git(dir, 'rev-parse', 'HEAD');
}

/** A repository whose main has one new merge on top of a baseline, as a run would see it. */
function setUp(): Rehearsal {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'tuck-rehearsal-'));
  roots.push(root);
  const remote = path.join(root, 'remote.git');
  const work = path.join(root, 'work');
  const state = path.join(root, 'state');
  const assets = path.join(root, 'assets');
  const bin = path.join(root, 'bin');
  fs.mkdirSync(path.join(state, 'releases'), { recursive: true });
  fs.mkdirSync(bin);
  fs.writeFileSync(path.join(bin, 'gh'), GH_STUB, { mode: 0o755 });
  fs.writeFileSync(path.join(bin, 'curl'), CURL_STUB, { mode: 0o755 });

  git(root, 'init', '-q', '--bare', '-b', 'main', remote);
  git(root, 'clone', '-q', remote, work);
  git(work, 'config', 'user.name', 'Test');
  git(work, 'config', 'user.email', 'test@example.com');
  git(work, 'config', 'commit.gpgsign', 'false');
  git(work, 'checkout', '-q', '-b', 'main');
  fs.mkdirSync(path.join(work, 'src-tauri'));
  fs.writeFileSync(path.join(work, 'CHANGELOG.md'), '# Changelog\n');
  fs.writeFileSync(path.join(work, 'package.json'), '{"version":"0.1.0"}\n');
  fs.writeFileSync(path.join(work, 'src-tauri', 'tauri.conf.json'), '{"endpoints":["https://example.test/latest.json"]}\n');
  fs.writeFileSync(path.join(work, 'src-tauri', 'Cargo.toml'), 'version = "0.1.0"\n');
  fs.writeFileSync(path.join(work, 'src-tauri', 'Cargo.lock'), 'version = "0.1.0"\n');
  git(work, 'add', '.');
  git(work, 'commit', '-q', '-m', 'chore: baseline');
  git(work, 'tag', 'v0.1.0');
  const source = commitAt(work, 'Merge pull request #22 from me/x', 'feat: the feature');
  git(work, 'push', '-q', 'origin', 'main', 'v0.1.0');
  // Simulate the repository rule after fixture setup. Any accidental release
  // push to protected main now fails; tests move main with update-ref only to
  // model an already merged PR or an independent race.
  fs.writeFileSync(
    path.join(remote, 'hooks', 'update'),
    '#!/usr/bin/env bash\nif [ "$1" = "refs/heads/main" ]; then echo "protected main: use a pull request" >&2; exit 1; fi\nexit 0\n',
    { mode: 0o755 },
  );
  return { root, remote, work, state, assets, source };
}

/** What the build job does for a version, including the assets the build would produce. */
function build(r: Rehearsal, version: string): void {
  fs.rmSync(r.assets, { recursive: true, force: true });
  fs.mkdirSync(r.assets, { recursive: true });
  const installer = `Tuck_${version}_Setup.exe`;
  fs.writeFileSync(path.join(r.assets, installer), 'installer');
  fs.writeFileSync(path.join(r.assets, `${installer}.sig`), 'signature');
  fs.writeFileSync(path.join(r.assets, 'release-notes.md'), 'notes');
  const url = `https://github.com/owner/tuck/releases/download/v${version}/${installer}`;
  fs.writeFileSync(
    path.join(r.assets, 'latest.json'),
    JSON.stringify({ version, platforms: { 'windows-x86_64': { signature: 'signature', url }, 'windows-x86_64-nsis': { signature: 'signature', url } } }, null, 2),
  );
  for (const file of ['CHANGELOG.md', 'package.json', 'src-tauri/tauri.conf.json', 'src-tauri/Cargo.toml', 'src-tauri/Cargo.lock']) {
    fs.appendFileSync(path.join(r.work, file), `\n${version}\n`);
  }
  for (const file of ['CHANGELOG.md', 'package.json', 'src-tauri/tauri.conf.json', 'src-tauri/Cargo.toml', 'src-tauri/Cargo.lock']) git(r.work, 'add', file);
  git(r.work, 'commit', '-q', '-m', `chore(release): v${version}`);
  const built = git(r.work, 'rev-parse', 'HEAD');
  // Transfer the object without updating protected main, then model an already
  // merged version PR with a bare-repository ref update.
  git(r.work, 'push', '-q', 'origin', `HEAD:refs/heads/_test-build-${version.replaceAll('.', '-')}`);
  git(r.remote, 'update-ref', 'refs/heads/main', built);
}

/** Puts the working copy back on the commit a fresh publish job would check out. */
function freshCheckout(r: Rehearsal, sha: string): void {
  git(r.work, 'checkout', '-q', '--detach', sha);
}

function publish(r: Rehearsal, version: string, source = remoteRef(r, 'refs/heads/main')) {
  return step(r, true, 'publish', version, source, r.assets.replaceAll('\\', '/'));
}

function remoteRef(r: Rehearsal, ref: string): string {
  return git(r.remote, 'for-each-ref', '--format=%(objectname)', ref);
}

/** Another clone pushes a merge to main, as a second PR landing would. */
function landAnotherMerge(r: Rehearsal, title: string): string {
  const other = path.join(r.root, `other-${Date.now()}-${Math.random().toString(36).slice(2)}`);
  git(r.root, 'clone', '-q', '-b', 'main', r.remote, other);
  git(other, 'config', 'user.name', 'Other');
  git(other, 'config', 'user.email', 'other@example.com');
  git(other, 'config', 'commit.gpgsign', 'false');
  const sha = commitAt(other, 'Merge pull request #23 from me/y', title);
  git(other, 'push', '-q', 'origin', 'HEAD:refs/heads/_test-merge');
  git(r.remote, 'update-ref', 'refs/heads/main', sha);
  return sha;
}

function releaseDraftFlag(r: Rehearsal, tag: string): string | null {
  const file = path.join(r.state, 'releases', tag, 'draft');
  return fs.existsSync(file) ? fs.readFileSync(file, 'utf8').trim() : null;
}

function writePreparedVersion(work: string, version: string): void {
  fs.writeFileSync(path.join(work, 'CHANGELOG.md'), `# Changelog\n\n## [${version}]\n\n- Release notes\n`);
  fs.writeFileSync(path.join(work, 'package.json'), `{"version":"${version}"}\n`);
  fs.writeFileSync(path.join(work, 'src-tauri', 'tauri.conf.json'), `{"version":"${version}"}\n`);
  fs.writeFileSync(path.join(work, 'src-tauri', 'Cargo.toml'), `version = "${version}"\n`);
  fs.writeFileSync(path.join(work, 'src-tauri', 'Cargo.lock'), `version = "${version}"\n`);
}

describe('release steps against a local bare remote', () => {
  it('prepare-pr pushes only a version branch, creates a PR, and dispatches checks', () => {
    const r = setUp();
    const notes = path.join(r.root, 'notes.md');
    fs.writeFileSync(notes, 'Release notes\n');
    writePreparedVersion(r.work, '0.2.1');

    const result = step(r, true, 'prepare-pr', '0.2.1', r.source, notes, r.assets.replaceAll('\\', '/'));
    const branch = `release/v0.2.1-${r.source.slice(0, 12)}`;

    expect(result.status).toBe(0);
    expect(remoteRef(r, `refs/heads/${branch}`)).not.toBe('');
    expect(remoteRef(r, 'refs/heads/main')).toBe(r.source);
    const blockedMainPush = spawnSync('git', ['push', '-q', 'origin', 'HEAD:refs/heads/main'], { cwd: r.work, encoding: 'utf8' });
    expect(blockedMainPush.status).not.toBe(0);
    expect(blockedMainPush.stderr).toContain('protected main');
    const log = fs.readFileSync(path.join(r.state, 'gh.log'), 'utf8');
    expect(log).toContain('gh pr create');
    expect(log).toContain('gh workflow run check.yml --ref');
    expect(fs.existsSync(path.join(r.state, 'workflow-dispatch'))).toBe(true);
    expect(fs.readFileSync(path.join(r.state, 'pr-body.md'), 'utf8')).toContain(`<!-- tuck-release source=${r.source} version=0.2.1 -->`);
  });

  it('reuses an identical existing candidate branch and PR without force-pushing', () => {
    const r = setUp();
    const notes = path.join(r.root, 'notes.md');
    const candidateWork = path.join(r.root, 'candidate');
    const branch = `release/v0.2.1-${r.source.slice(0, 12)}`;
    fs.writeFileSync(notes, 'Release notes\n');
    git(r.root, 'clone', '-q', '-b', 'main', r.remote, candidateWork);
    git(candidateWork, 'config', 'user.name', 'Candidate');
    git(candidateWork, 'config', 'user.email', 'candidate@example.com');
    writePreparedVersion(candidateWork, '0.2.1');
    git(candidateWork, 'add', '.');
    git(candidateWork, 'commit', '-q', '-m', 'chore(release): v0.2.1');
    const candidateSha = git(candidateWork, 'rev-parse', 'HEAD');
    git(candidateWork, 'push', '-q', 'origin', `HEAD:refs/heads/${branch}`);
    fs.writeFileSync(
      path.join(r.state, 'prs.json'),
      JSON.stringify([{ number: 41, headRefName: branch, headRefOid: candidateSha, headRepository: { nameWithOwner: 'owner/tuck' }, body: `<!-- tuck-release source=${r.source} version=0.2.1 -->\n\nRelease notes` }]),
    );
    writePreparedVersion(r.work, '0.2.1');

    const result = step(r, true, 'prepare-pr', '0.2.1', r.source, notes, r.assets.replaceAll('\\', '/'));
    expect(result.status).toBe(0);
    expect(remoteRef(r, `refs/heads/${branch}`)).toBe(candidateSha);
    const log = fs.readFileSync(path.join(r.state, 'gh.log'), 'utf8');
    expect(log).not.toContain('gh pr create');
    expect(log).toContain('gh workflow run check.yml --ref');
  });

  it('rejects an existing candidate branch with the wrong source parent', () => {
    const r = setUp();
    const notes = path.join(r.root, 'notes.md');
    const candidateWork = path.join(r.root, 'bad-candidate');
    const branch = `release/v0.2.1-${r.source.slice(0, 12)}`;
    fs.writeFileSync(notes, 'Release notes\n');
    git(r.root, 'clone', '-q', r.remote, candidateWork);
    git(candidateWork, 'config', 'user.name', 'Candidate');
    git(candidateWork, 'config', 'user.email', 'candidate@example.com');
    git(candidateWork, 'checkout', '-q', 'v0.1.0');
    writePreparedVersion(candidateWork, '0.2.1');
    git(candidateWork, 'add', '.');
    git(candidateWork, 'commit', '-q', '-m', 'chore(release): v0.2.1');
    const badSha = git(candidateWork, 'rev-parse', 'HEAD');
    git(candidateWork, 'push', '-q', 'origin', `HEAD:refs/heads/${branch}`);
    writePreparedVersion(r.work, '0.2.1');

    const result = step(r, true, 'prepare-pr', '0.2.1', r.source, notes, r.assets.replaceAll('\\', '/'));

    expect(result.status).toBe(1);
    expect(result.stderr).toContain('does not have');
    expect(remoteRef(r, `refs/heads/${branch}`)).toBe(badSha);
    expect(fs.existsSync(path.join(r.state, 'gh.log'))).toBe(false);
  });

  it('closes only stale generated same-repository release PRs', () => {
    const r = setUp();
    const notes = path.join(r.root, 'notes.md');
    fs.writeFileSync(notes, 'Release notes\n');
    writePreparedVersion(r.work, '0.2.1');
    fs.writeFileSync(
      path.join(r.state, 'prs.json'),
      JSON.stringify([
        { number: 51, headRefName: 'release/v0.2.0-aaaaaaaaaaaa', headRefOid: r.source, headRepository: { nameWithOwner: 'owner/tuck' }, body: '<!-- tuck-release source=old version=0.2.0 -->' },
        { number: 52, headRefName: 'release/v0.2.0-bbbbbbbbbbbb', headRefOid: r.source, headRepository: { nameWithOwner: 'other/tuck' }, body: '<!-- tuck-release source=old version=0.2.0 -->' },
        { number: 53, headRefName: 'release/v0.2.0-cccccccccccc', headRefOid: r.source, headRepository: { nameWithOwner: 'owner/tuck' }, body: 'human PR' },
      ]),
    );

    const result = step(r, true, 'prepare-pr', '0.2.1', r.source, notes, r.assets.replaceAll('\\', '/'));

    expect(result.status).toBe(0);
    expect(fs.readFileSync(path.join(r.state, 'closed-prs'), 'utf8').trim()).toBe('51');
  });

  it('prepare-pr is read-only in dry-run mode', () => {
    const r = setUp();
    const notes = path.join(r.root, 'notes.md');
    fs.writeFileSync(notes, 'Release notes\n');
    const result = step(r, false, 'prepare-pr', '0.2.1', r.source, notes, r.assets.replaceAll('\\', '/'));

    expect(result.status).toBe(1);
    expect(result.stderr).toContain('RELEASE_LIVE');
    expect(remoteRef(r, 'refs/heads/main')).toBe(r.source);
    expect(fs.existsSync(path.join(r.state, 'gh.log'))).toBe(false);
  });

  it('happy path: main and the tag move together to the commit that was built', () => {
    const r = setUp();
    build(r, '0.2.0');
    const built = git(r.work, 'rev-parse', 'HEAD');
    freshCheckout(r, r.source);

    const result = publish(r, '0.2.0');

    expect(result.status).toBe(0);
    expect(remoteRef(r, 'refs/heads/main')).toBe(built);
    expect(remoteRef(r, 'refs/tags/v0.2.0')).toBe(built);
    expect(releaseDraftFlag(r, 'v0.2.0')).toBe('false');
    expect(fs.readdirSync(path.join(r.state, 'releases', 'v0.2.0', 'assets')).sort()).toEqual([
      'Tuck_0.2.0_Setup.exe',
      'Tuck_0.2.0_Setup.exe.sig',
      'latest.json',
    ]);
    // Uploaded while still a draft, published only after the push
    const log = fs.readFileSync(path.join(r.state, 'gh.log'), 'utf8');
    expect(log.indexOf('release create')).toBeLessThan(log.indexOf('release edit'));
  });

  it('dry run: no step can touch the remote', () => {
    const r = setUp();
    build(r, '0.2.0');
    const before = remoteRef(r, 'refs/heads/main');
    freshCheckout(r, r.source);

    expect(step(r, false, 'publish', '0.2.0', r.source, r.assets.replaceAll('\\', '/')).status).toBe(1);
    expect(step(r, false, 'finish-pending').status).toBe(1);

    expect(remoteRef(r, 'refs/heads/main')).toBe(before);
    expect(remoteRef(r, 'refs/tags/v0.2.0')).toBe('');
    expect(fs.existsSync(path.join(r.state, 'gh.log'))).toBe(false);
  });

  it('moved main before publish: exits green, changes nothing, and the newest run covers both merges', () => {
    const r = setUp();
    build(r, '0.2.0');
    freshCheckout(r, r.source);
    const newest = landAnotherMerge(r, 'fix: a later fix');

    const result = publish(r, '0.2.0', r.source);

    expect(result.status).toBe(0);
    expect(result.stdout).toContain('main moved');
    expect(remoteRef(r, 'refs/heads/main')).toBe(newest);
    expect(remoteRef(r, 'refs/tags/v0.2.0')).toBe('');
    expect(releaseDraftFlag(r, 'v0.2.0')).toBeNull();
    expect(step(r, false, 'is-current', 'main', r.source).stdout.trim()).toBe('false');
    expect(step(r, false, 'is-current', 'main', newest).stdout.trim()).toBe('true');
    // The range the newest run reads still starts at the last tag
    git(r.work, 'fetch', '-q', 'origin', 'main');
    const range = git(r.work, 'log', '--first-parent', '--format=%b', 'v0.1.0..FETCH_HEAD');
    expect(range).toContain('feat: the feature');
    expect(range).toContain('fix: a later fix');
  });

  it('main moves during the push: the atomic push changes neither ref and the draft is removed', () => {
    const r = setUp();
    build(r, '0.2.0');
    freshCheckout(r, r.source);
    const other = path.join(r.root, 'race');
    git(r.root, 'clone', '-q', '-b', 'main', r.remote, other);
    git(other, 'config', 'user.name', 'Other');
    git(other, 'config', 'user.email', 'other@example.com');
    git(other, 'config', 'commit.gpgsign', 'false');
    const raced = commitAt(other, 'Merge pull request #23 from me/y', 'fix: raced');
    git(other, 'push', '-q', 'origin', 'HEAD:refs/heads/_test-race');
    fs.writeFileSync(
      path.join(r.state, 'on-create'),
      `#!/usr/bin/env bash\ngit --git-dir "${r.remote.replaceAll('\\', '/')}" update-ref refs/heads/main "$(git -C "${other.replaceAll('\\', '/')}" rev-parse HEAD)"\n`,
      { mode: 0o755 },
    );

    const result = publish(r, '0.2.0');

    expect(result.status).toBe(0);
    expect(result.stdout).toContain('during release preparation');
    expect(remoteRef(r, 'refs/heads/main')).toBe(raced);
    expect(remoteRef(r, 'refs/tags/v0.2.0')).toBe('');
    expect(releaseDraftFlag(r, 'v0.2.0')).toBeNull();
  });

  it('re-run after a completed release: nothing is built, pushed or published twice', () => {
    const r = setUp();
    build(r, '0.2.0');
    const built = git(r.work, 'rev-parse', 'HEAD');
    freshCheckout(r, r.source);
    expect(publish(r, '0.2.0').status).toBe(0);
    const logBefore = fs.readFileSync(path.join(r.state, 'gh.log'), 'utf8');
    freshCheckout(r, r.source);

    expect(step(r, false, 'is-current', 'main', r.source).stdout.trim()).toBe('false');
    const finish = step(r, true, 'finish-pending');
    const again = publish(r, '0.2.0');

    expect(finish.status).toBe(0);
    expect(finish.stdout).toContain('already published');
    expect(again.status).toBe(0);
    expect(remoteRef(r, 'refs/heads/main')).toBe(built);
    expect(remoteRef(r, 'refs/tags/v0.2.0')).toBe(built);
    const log = fs.readFileSync(path.join(r.state, 'gh.log'), 'utf8').slice(logBefore.length);
    expect(log).not.toContain('create');
    expect(log).not.toContain('edit');
  });

  it('a re-run of a published release warns if the update feed serves another version, without blocking the next release', () => {
    const r = setUp();
    build(r, '0.2.0');
    freshCheckout(r, r.source);
    expect(publish(r, '0.2.0').status).toBe(0);
    fs.writeFileSync(path.join(r.state, 'live-latest.json'), '{"version": "0.1.0"}');
    freshCheckout(r, r.source);

    const finish = step(r, true, 'finish-pending');

    expect(finish.status).toBe(0);
    expect(finish.stdout).toContain('::warning::');
    expect(finish.stdout).toContain('does not serve 0.2.0');
  });

  it('publish fails after the push: the job is red and a re-run finishes without building', () => {
    const r = setUp();
    build(r, '0.2.0');
    const built = git(r.work, 'rev-parse', 'HEAD');
    freshCheckout(r, r.source);
    fs.writeFileSync(path.join(r.state, 'fail-edit'), '');

    const first = publish(r, '0.2.0');
    expect(first.status).toBe(1);
    expect(first.stderr).toContain('Re-run this workflow');
    expect(remoteRef(r, 'refs/tags/v0.2.0')).toBe(built);
    expect(releaseDraftFlag(r, 'v0.2.0')).toBe('true');

    fs.rmSync(path.join(r.state, 'fail-edit'));
    freshCheckout(r, r.source);
    const finish = step(r, true, 'finish-pending');
    expect(finish.status).toBe(0);
    expect(finish.stdout).toContain('Finishing v0.2.0');
    expect(releaseDraftFlag(r, 'v0.2.0')).toBe('false');
    expect(publish(r, '0.2.0').status).toBe(0);
    expect(fs.existsSync(path.join(r.state, 'errors'))).toBe(false);
  });

  it('publish failed and nobody re-ran: the next merge publishes the old draft first, so no version is skipped', () => {
    const r = setUp();
    build(r, '0.2.0');
    freshCheckout(r, r.source);
    fs.writeFileSync(path.join(r.state, 'fail-edit'), '');
    expect(publish(r, '0.2.0').status).toBe(1);
    fs.rmSync(path.join(r.state, 'fail-edit'));

    // Older interrupted releases keep their original public asset names.
    const oldAssets = path.join(r.state, 'releases', 'v0.2.0', 'assets');
    for (const suffix of ['', '.sig']) {
      fs.renameSync(
        path.join(oldAssets, `Tuck_0.2.0_Setup.exe${suffix}`),
        path.join(oldAssets, `tuck_0.2.0_x64-setup.exe${suffix}`),
      );
    }
    const oldManifestPath = path.join(oldAssets, 'latest.json');
    const oldManifest = JSON.parse(fs.readFileSync(oldManifestPath, 'utf8')) as {
      platforms: Record<string, { url: string }>;
    };
    for (const platform of Object.values(oldManifest.platforms)) {
      platform.url = platform.url.replace('Tuck_0.2.0_Setup.exe', 'tuck_0.2.0_x64-setup.exe');
    }
    fs.writeFileSync(oldManifestPath, JSON.stringify(oldManifest, null, 2));

    const next = landAnotherMerge(r, 'fix: next merge');
    git(r.work, 'fetch', '-q', 'origin', 'main');
    git(r.work, 'checkout', '-q', '--detach', next);
    build(r, '0.2.1');
    freshCheckout(r, remoteRef(r, 'refs/heads/main'));

    expect(step(r, true, 'finish-pending').status).toBe(0);
    expect(releaseDraftFlag(r, 'v0.2.0')).toBe('false');
    expect(publish(r, '0.2.1').status).toBe(0);
    expect(releaseDraftFlag(r, 'v0.2.1')).toBe('false');
  });

  it('upload fails before the push: nothing reaches main and a re-run leaves one draft', () => {
    const r = setUp();
    build(r, '0.2.0');
    const before = remoteRef(r, 'refs/heads/main');
    freshCheckout(r, r.source);
    fs.writeFileSync(path.join(r.state, 'fail-create'), '');

    expect(publish(r, '0.2.0').status).toBe(1);
    expect(remoteRef(r, 'refs/heads/main')).toBe(before);
    expect(remoteRef(r, 'refs/tags/v0.2.0')).toBe('');

    fs.rmSync(path.join(r.state, 'fail-create'));
    // A stale draft from an interrupted upload
    fs.mkdirSync(path.join(r.state, 'releases', 'v0.2.0', 'assets'), { recursive: true });
    fs.writeFileSync(path.join(r.state, 'releases', 'v0.2.0', 'draft'), 'true\n');
    freshCheckout(r, r.source);
    expect(publish(r, '0.2.0').status).toBe(0);
    expect(fs.existsSync(path.join(r.state, 'errors'))).toBe(false);
    expect(releaseDraftFlag(r, 'v0.2.0')).toBe('false');
  });

  it('rejects a pre-existing tag that points at the wrong commit', () => {
    const r = setUp();
    build(r, '0.2.0');
    const built = remoteRef(r, 'refs/heads/main');
    git(r.remote, 'update-ref', 'refs/tags/v0.2.0', r.source);
    freshCheckout(r, built);

    const result = publish(r, '0.2.0');

    expect(result.status).toBe(1);
    expect(result.stderr).toContain('Refusing to reuse the wrong tag');
    expect(remoteRef(r, 'refs/tags/v0.2.0')).toBe(r.source);
  });

  it('replaces an orphan draft for the version being safely retried', () => {
    const r = setUp();
    build(r, '0.2.1');
    const built = remoteRef(r, 'refs/heads/main');
    const orphan = path.join(r.state, 'releases', 'v0.2.1', 'assets');
    fs.mkdirSync(orphan, { recursive: true });
    fs.writeFileSync(path.join(r.state, 'releases', 'v0.2.1', 'draft'), 'true\n');
    fs.writeFileSync(path.join(orphan, 'stale.txt'), 'old upload');
    freshCheckout(r, built);

    const result = publish(r, '0.2.1');

    expect(result.status).toBe(0);
    expect(releaseDraftFlag(r, 'v0.2.1')).toBe('false');
    expect(fs.existsSync(path.join(r.state, 'releases', 'v0.2.1', 'assets', 'stale.txt'))).toBe(false);
    expect(remoteRef(r, 'refs/tags/v0.2.1')).toBe(built);
  });

  it('rejects a newly uploaded draft corrupted before the tag push', () => {
    const r = setUp();
    build(r, '0.2.0');
    fs.writeFileSync(
      path.join(r.state, 'on-create'),
      '#!/usr/bin/env bash\nprintf \'%s\\n\' \'{"version":"9.9.9"}\' > "$STUB_STATE/releases/v0.2.0/assets/latest.json"\n',
      { mode: 0o755 },
    );
    const result = publish(r, '0.2.0');
    expect(result.status).toBe(1);
    expect(result.stderr).toContain('latest.json has version');
    expect(remoteRef(r, 'refs/tags/v0.2.0')).toBe('');
    expect(releaseDraftFlag(r, 'v0.2.0')).toBe('true');
    expect(fs.readFileSync(path.join(r.state, 'gh.log'), 'utf8')).not.toContain('release edit');
  });

  it('rejects a tagged draft whose updater manifest has the wrong version', () => {
    const r = setUp();
    build(r, '0.2.0');
    const built = remoteRef(r, 'refs/heads/main');
    freshCheckout(r, built);
    fs.writeFileSync(path.join(r.state, 'fail-edit'), '');
    expect(publish(r, '0.2.0').status).toBe(1);
    fs.rmSync(path.join(r.state, 'fail-edit'));
    const manifestPath = path.join(r.state, 'releases', 'v0.2.0', 'assets', 'latest.json');
    const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8')) as { version: string };
    manifest.version = '9.9.9';
    fs.writeFileSync(manifestPath, JSON.stringify(manifest, null, 2));

    const result = step(r, true, 'finish-pending');

    expect(result.status).toBe(1);
    expect(result.stderr).toContain('latest.json has version');
    expect(releaseDraftFlag(r, 'v0.2.0')).toBe('true');
  });

  it('a released tag whose release was deleted stops the pipeline, but the baseline tag does not', () => {
    const r = setUp();
    expect(step(r, true, 'finish-pending').stdout).toContain('baseline tag');

    build(r, '0.2.0');
    freshCheckout(r, r.source);
    expect(publish(r, '0.2.0').status).toBe(0);
    fs.rmSync(path.join(r.state, 'releases', 'v0.2.0'), { recursive: true });

    const result = step(r, true, 'finish-pending');
    expect(result.status).toBe(1);
    expect(result.stderr).toContain('no GitHub release');
  });

  it('refuses a release commit that changes more than version files', () => {
    const r = setUp();
    fs.writeFileSync(path.join(r.work, 'sneaky.txt'), 'x');
    git(r.work, 'add', 'sneaky.txt');
    fs.appendFileSync(path.join(r.work, 'package.json'), '\n');
    git(r.work, 'commit', '-q', '-m', 'wip');
    const result = step(r, false, 'commit-bump', '0.2.0', r.source, r.assets.replaceAll('\\', '/'));
    expect(result.status).toBe(1);
    expect(result.stderr).toContain('sneaky.txt');
  });

  it('verify-clean fails when the build changed a tracked file', () => {
    const r = setUp();
    expect(step(r, false, 'verify-clean').status).toBe(0);
    fs.appendFileSync(path.join(r.work, 'package.json'), '\nchanged\n');
    expect(step(r, false, 'verify-clean').status).toBe(1);
  });
});
