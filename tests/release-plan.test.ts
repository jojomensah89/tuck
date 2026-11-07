import { afterAll, describe, expect, it } from 'bun:test';
import { execFileSync } from 'node:child_process';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';

import { syncCargoLockVersion } from '../scripts/release-metadata';
import { compareVersions, planRelease, validateSynchronizedVersion } from '../scripts/release-plan';

const roots: string[] = [];
afterAll(() => {
  for (const root of roots) fs.rmSync(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
});

function git(cwd: string, ...args: string[]): string {
  return execFileSync('git', args, { cwd, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();
}

function makeRepo(): string {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'tuck-release-plan-'));
  roots.push(root);
  fs.mkdirSync(path.join(root, 'src-tauri'), { recursive: true });
  git(root, 'init', '-q', '-b', 'main');
  git(root, 'config', 'user.name', 'Test');
  git(root, 'config', 'user.email', 'test@example.com');
  git(root, 'config', 'commit.gpgsign', 'false');
  fs.writeFileSync(path.join(root, 'package.json'), '{"version":"0.2.0"}\n');
  fs.writeFileSync(path.join(root, 'src-tauri', 'tauri.conf.json'), '{"version":"0.2.0"}\n');
  fs.writeFileSync(path.join(root, 'src-tauri', 'Cargo.toml'), '[package]\nname = "tuck"\nversion = "0.2.0"\n\n[dependencies]\n');
  fs.writeFileSync(
    path.join(root, 'src-tauri', 'Cargo.lock'),
    '# generated\n\n[[package]]\nname = "dependency"\nversion = "1.0.0"\n\n[[package]]\nname = "tuck"\nversion = "0.2.0"\n',
  );
  fs.writeFileSync(
    path.join(root, 'CHANGELOG.md'),
    '# Changelog\n\n## [0.2.0] - 2026-11-05\n\n### Added\n- Existing release\n',
  );
  git(root, 'add', '.');
  git(root, 'commit', '-q', '-m', 'chore: baseline');
  git(root, 'tag', 'v0.2.0');
  return root;
}

describe('release planning', () => {
  it('uses strict semver ordering', () => {
    expect(compareVersions('0.2.1', '0.2.0')).toBe(1);
    expect(compareVersions('1.0.0', '0.9.9')).toBe(1);
    expect(compareVersions('0.2.0', '0.2.0')).toBe(0);
    expect(() => compareVersions('0.2', '0.2.0')).toThrow(/Invalid release version/);
  });

  it('plans a version-only PR when main is at the latest tag', () => {
    const root = makeRepo();
    git(root, 'commit', '-q', '--allow-empty', '-m', 'feat: add the next release');
    expect(planRelease(root)).toMatchObject({
      action: 'prepare',
      currentVersion: '0.2.0',
      latestTag: 'v0.2.0',
      nextVersion: '0.3.0',
    });
  });

  it('plans no PR for non-release history', () => {
    const root = makeRepo();
    git(root, 'commit', '-q', '--allow-empty', '-m', 'docs: clarify release steps');
    expect(planRelease(root)).toMatchObject({ action: 'none', currentVersion: '0.2.0' });
  });

  it('builds a merged version only when every version file and notes section agree', () => {
    const root = makeRepo();
    fs.writeFileSync(path.join(root, 'package.json'), '{"version":"0.2.1"}\n');
    fs.writeFileSync(path.join(root, 'src-tauri', 'tauri.conf.json'), '{"version":"0.2.1"}\n');
    fs.writeFileSync(path.join(root, 'src-tauri', 'Cargo.toml'), '[package]\nname = "tuck"\nversion = "0.2.1"\n');
    syncCargoLockVersion(root, '0.2.1');
    fs.writeFileSync(path.join(root, 'CHANGELOG.md'), '# Changelog\n\n## [0.2.1] - 2026-11-06\n\n### Fixed\n- Merged release fixes\n');
    git(root, 'add', '.');
    git(root, 'commit', '-q', '-m', 'chore(release): v0.2.1');
    expect(planRelease(root)).toMatchObject({ action: 'build', currentVersion: '0.2.1', latestTag: 'v0.2.0' });
  });

  it('rejects an arbitrary version bump with a mismatched lockfile', () => {
    const root = makeRepo();
    fs.writeFileSync(path.join(root, 'package.json'), '{"version":"0.9.0"}\n');
    fs.writeFileSync(path.join(root, 'src-tauri', 'tauri.conf.json'), '{"version":"0.9.0"}\n');
    fs.writeFileSync(path.join(root, 'src-tauri', 'Cargo.toml'), '[package]\nname = "tuck"\nversion = "0.9.0"\n');
    fs.writeFileSync(path.join(root, 'CHANGELOG.md'), '# Changelog\n\n## [0.9.0] - 2026-11-06\n\n### Fixed\n- Notes\n');
    git(root, 'add', '.');
    git(root, 'commit', '-q', '-m', 'chore: arbitrary bump');
    expect(() => planRelease(root)).toThrow(/lock version 0.2.0 does not match 0.9.0/);
  });

  it('updates only Tuck in Cargo.lock', () => {
    const root = makeRepo();
    const before = fs.readFileSync(path.join(root, 'src-tauri', 'Cargo.lock'), 'utf8');
    syncCargoLockVersion(root, '0.2.1');
    const after = fs.readFileSync(path.join(root, 'src-tauri', 'Cargo.lock'), 'utf8');
    expect(after).toContain('name = "dependency"\nversion = "1.0.0"');
    expect(after).toContain('name = "tuck"\nversion = "0.2.1"');
    expect(after.replace('name = "tuck"\nversion = "0.2.1"', 'name = "tuck"\nversion = "0.2.0"')).toBe(before);
  });

  it('rejects a merged version without a current changelog section', () => {
    const root = makeRepo();
    fs.writeFileSync(path.join(root, 'package.json'), '{"version":"0.2.1"}\n');
    fs.writeFileSync(path.join(root, 'src-tauri', 'tauri.conf.json'), '{"version":"0.2.1"}\n');
    fs.writeFileSync(path.join(root, 'src-tauri', 'Cargo.toml'), '[package]\nname = "tuck"\nversion = "0.2.1"\n');
    syncCargoLockVersion(root, '0.2.1');
    fs.writeFileSync(path.join(root, 'CHANGELOG.md'), '# Changelog\n\n## [0.2.0] - 2026-11-05\n\n### Added\n- Old notes\n');
    git(root, 'add', '.');
    git(root, 'commit', '-q', '-m', 'chore: incomplete bump');
    expect(() => validateSynchronizedVersion(root, '0.2.1')).toThrow(/starts at 0.2.0/);
  });
});
