import * as fs from 'node:fs';
import * as path from 'node:path';

import {
  evaluateRelease,
  getLatestTag,
  readCurrentVersion,
  releaseNotesBody,
  syncCargoLockVersion,
  syncVersionFiles,
  updateChangelogContent,
} from './release-metadata';

export type ReleaseAction = 'none' | 'prepare' | 'build';

export interface ReleasePlan {
  action: ReleaseAction;
  currentVersion: string;
  latestTag: string;
  nextVersion?: string;
  reason?: string;
}

const SEMVER = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;

function parseVersion(version: string): [number, number, number] {
  const match = SEMVER.exec(version);
  if (!match) throw new Error(`Invalid release version: ${version}`);
  return [Number(match[1]), Number(match[2]), Number(match[3])];
}

export function compareVersions(left: string, right: string): number {
  const a = parseVersion(left);
  const b = parseVersion(right);
  for (let index = 0; index < a.length; index += 1) {
    if (a[index] !== b[index]) return a[index] > b[index] ? 1 : -1;
  }
  return 0;
}

export function isNewerVersion(version: string, baseline: string): boolean {
  return compareVersions(version, baseline) > 0;
}

function cargoPackageVersion(cargoToml: string): string {
  const header = /^\[package\]\s*\r?\n/m.exec(cargoToml);
  const bodyStart = header ? (header.index ?? 0) + header[0].length : -1;
  const rest = bodyStart < 0 ? '' : cargoToml.slice(bodyStart);
  const nextSection = /^\[/m.exec(rest)?.index;
  const packageBlock = nextSection === undefined ? rest : rest.slice(0, nextSection);
  const version = /^version\s*=\s*"([^"]+)"/m.exec(packageBlock)?.[1];
  if (!version) throw new Error('src-tauri/Cargo.toml has no root package version.');
  return version;
}

function cargoLockPackageVersion(lock: string): string {
  const block = lock
    .split(/(?=^\[\[package\]\]\s*$)/m)
    .find(entry => /^name\s*=\s*"tuck"\s*$/m.test(entry));
  const version = block && /^version\s*=\s*"([^"]+)"/m.exec(block)?.[1];
  if (!version) throw new Error('src-tauri/Cargo.lock has no root tuck package version.');
  return version;
}

function changelogHeaderAndNotes(projectRoot: string): { version: string; notes: string } {
  const changelog = fs.readFileSync(path.join(projectRoot, 'CHANGELOG.md'), 'utf8');
  const match = /^## \[([^\]]+)\][^\r\n]*\r?\n/m.exec(changelog);
  if (!match) throw new Error('CHANGELOG.md has no version section.');
  const bodyStart = (match.index ?? 0) + match[0].length;
  const rest = changelog.slice(bodyStart);
  const nextHeader = /^## \[/m.exec(rest)?.index;
  const body = nextHeader === undefined ? rest : rest.slice(0, nextHeader);
  return { version: match[1], notes: body.replace(/^\s+|\s+$/g, '') };
}

/**
 * Validate the files that must agree before a merged version can be built.
 * This intentionally does not run Cargo, which could update unrelated locks.
 */
export function validateSynchronizedVersion(projectRoot: string, expectedVersion: string): void {
  parseVersion(expectedVersion);
  const packageVersion = readCurrentVersion(projectRoot);
  const tauri = JSON.parse(fs.readFileSync(path.join(projectRoot, 'src-tauri', 'tauri.conf.json'), 'utf8')) as { version?: string };
  const cargo = cargoPackageVersion(fs.readFileSync(path.join(projectRoot, 'src-tauri', 'Cargo.toml'), 'utf8'));
  const lock = cargoLockPackageVersion(fs.readFileSync(path.join(projectRoot, 'src-tauri', 'Cargo.lock'), 'utf8'));
  const versions = { package: packageVersion, tauri: tauri.version, cargo, lock };
  for (const [file, version] of Object.entries(versions)) {
    if (version !== expectedVersion) {
      throw new Error(`${file} version ${version ?? '<missing>'} does not match ${expectedVersion}.`);
    }
  }
  const changelog = changelogHeaderAndNotes(projectRoot);
  if (changelog.version !== expectedVersion) {
    throw new Error(`CHANGELOG.md starts at ${changelog.version}, not ${expectedVersion}.`);
  }
  if (!changelog.notes) throw new Error(`CHANGELOG.md has no notes for ${expectedVersion}.`);
}

export function planRelease(projectRoot: string): ReleasePlan {
  const currentVersion = readCurrentVersion(projectRoot);
  const latestTag = getLatestTag(projectRoot);
  if (!latestTag) throw new Error('No vX.Y.Z tag is reachable from HEAD.');
  const latestVersion = latestTag.slice(1);
  const comparison = compareVersions(currentVersion, latestVersion);

  if (comparison < 0) {
    throw new Error(`Package version ${currentVersion} is behind reachable tag ${latestTag}.`);
  }
  if (comparison > 0) {
    validateSynchronizedVersion(projectRoot, currentVersion);
    return { action: 'build', currentVersion, latestTag };
  }

  const decision = evaluateRelease(projectRoot);
  if (!decision.shouldRelease) {
    return { action: 'none', currentVersion, latestTag, reason: decision.reason };
  }
  if (!isNewerVersion(decision.nextVersion, currentVersion)) {
    throw new Error(`Release metadata produced a non-increasing version ${decision.nextVersion}.`);
  }
  return { action: 'prepare', currentVersion, latestTag, nextVersion: decision.nextVersion };
}

function applyPreparation(projectRoot: string, notesFile: string): void {
  const decision = evaluateRelease(projectRoot);
  if (!decision.shouldRelease) throw new Error(decision.reason ?? 'No release needed.');
  const notes = releaseNotesBody(decision.changelogSection);
  if (!notes) throw new Error(`No user-facing notes for ${decision.tagName}.`);
  syncVersionFiles(projectRoot, decision.nextVersion);
  syncCargoLockVersion(projectRoot, decision.nextVersion);
  const changelogPath = path.join(projectRoot, 'CHANGELOG.md');
  const existing = fs.existsSync(changelogPath) ? fs.readFileSync(changelogPath, 'utf8') : '';
  fs.writeFileSync(changelogPath, updateChangelogContent(existing, decision.changelogSection), 'utf8');
  fs.writeFileSync(path.resolve(projectRoot, notesFile), `${notes}\n`, 'utf8');
}

if (import.meta.main) {
  const args = process.argv.slice(2);
  const projectRoot = path.resolve(import.meta.dir, '..');
  if (args.includes('--json')) {
    console.log(JSON.stringify(planRelease(projectRoot)));
  } else if (args.includes('--validate-current')) {
    const version = args[args.indexOf('--validate-current') + 1] ?? readCurrentVersion(projectRoot);
    validateSynchronizedVersion(projectRoot, version);
    console.log(`Validated synchronized release version ${version}.`);
  } else if (args.includes('--apply')) {
    const notesIndex = args.indexOf('--notes-file');
    const notesFile = notesIndex === -1 ? undefined : args[notesIndex + 1];
    if (!notesFile) throw new Error('--apply needs --notes-file <path>');
    applyPreparation(projectRoot, notesFile);
  } else {
    throw new Error('Usage: release-plan.ts --json | --validate-current [version] | --apply --notes-file <path>');
  }
}
