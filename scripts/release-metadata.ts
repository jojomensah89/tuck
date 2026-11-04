import { execSync } from 'node:child_process';
import * as fs from 'node:fs';
import * as path from 'node:path';

export type BumpType = 'major' | 'minor' | 'patch' | 'none';

export interface ParsedCommit {
  hash: string;
  type: string;
  scope?: string;
  isBreaking: boolean;
  subject: string;
  rawSubject: string;
  body?: string;
}

export interface ReleaseDecision {
  shouldRelease: boolean;
  reason?: string;
  bumpType: BumpType;
  currentVersion: string;
  nextVersion: string;
  tagName: string;
  commits: ParsedCommit[];
  changelogSection: string;
}

const CONVENTIONAL_REGEX =
  /^(?<type>[a-z]+)(?:\((?<scope>[^)]+)\))?(?<breaking>!)?:\s*(?<subject>.+)$/i;
const PR_SUFFIX_REGEX = /\s*\([#\d\s,]+\)$/;

/**
 * Clean up a conventional commit subject into human-readable user-facing text.
 */
export function cleanCommitSubject(rawSubject: string): { type: string; scope?: string; isBreaking: boolean; subject: string } {
  const match = CONVENTIONAL_REGEX.exec(rawSubject.trim());
  if (!match || !match.groups) {
    let cleaned = rawSubject.trim().replace(PR_SUFFIX_REGEX, '').trim();
    if (cleaned.length > 0) {
      cleaned = cleaned.charAt(0).toUpperCase() + cleaned.slice(1);
    }
    return { type: 'chore', isBreaking: false, subject: cleaned };
  }

  const { type, scope, breaking, subject } = match.groups;
  let cleaned = subject.replace(PR_SUFFIX_REGEX, '').trim();
  if (cleaned.length > 0) {
    cleaned = cleaned.charAt(0).toUpperCase() + cleaned.slice(1);
  }

  return {
    type: type.toLowerCase(),
    scope: scope?.trim(),
    isBreaking: Boolean(breaking),
    subject: cleaned,
  };
}

/**
 * Parse raw git commit lines into structured commit metadata.
 */
export function parseCommit(hash: string, rawSubject: string, body = ''): ParsedCommit {
  const { type, scope, isBreaking, subject } = cleanCommitSubject(rawSubject);
  const bodyHasBreaking =
    /BREAKING CHANGE:/i.test(body) || /BREAKING-CHANGE:/i.test(body);

  return {
    hash,
    type,
    scope,
    isBreaking: isBreaking || bodyHasBreaking,
    subject,
    rawSubject,
    body,
  };
}

/**
 * Determine the bump type from a list of parsed commits according to the release specification:
 * - feat -> minor
 * - fix/perf -> patch
 * - breaking -> minor while below 1.0 (major if >= 1.0)
 * - if only docs/test/chore/ci/style/refactor/build landed, do not release (none)
 */
export function determineBumpType(commits: readonly ParsedCommit[], currentVersion: string): BumpType {
  const isBelowOne = currentVersion.startsWith('0.');
  let hasFeat = false;
  let hasPatch = false;
  let hasBreaking = false;

  for (const c of commits) {
    if (c.isBreaking) {
      hasBreaking = true;
    } else if (c.type === 'feat') {
      hasFeat = true;
    } else if (c.type === 'fix' || c.type === 'perf') {
      hasPatch = true;
    }
  }

  if (hasBreaking) {
    return isBelowOne ? 'minor' : 'major';
  }
  if (hasFeat) {
    return 'minor';
  }
  if (hasPatch) {
    return 'patch';
  }
  return 'none';
}

/**
 * Calculate the next semver string from current version and bump type.
 */
export function calculateNextVersion(currentVersion: string, bump: BumpType): string {
  if (bump === 'none') return currentVersion;

  const parts = currentVersion.split('.').map(n => Number.parseInt(n, 10));
  if (parts.length !== 3 || parts.some(Number.isNaN)) {
    throw new Error(`Invalid semver version: ${currentVersion}`);
  }

  let [major, minor, patch] = parts;
  if (bump === 'major') {
    major += 1;
    minor = 0;
    patch = 0;
  } else if (bump === 'minor') {
    minor += 1;
    patch = 0;
  } else if (bump === 'patch') {
    patch += 1;
  }

  return `${major}.${minor}.${patch}`;
}

/**
 * Generate a Keep a Changelog section for a new version from commits.
 */
export function generateChangelogSection(version: string, commits: readonly ParsedCommit[], releaseDate?: string): string {
  const dateStr = releaseDate ?? new Date().toISOString().split('T')[0];
  const added: string[] = [];
  const fixed: string[] = [];
  const changed: string[] = [];

  for (const c of commits) {
    if (!c.subject) continue;
    if (c.type === 'feat') {
      added.push(c.subject);
    } else if (c.type === 'fix') {
      fixed.push(c.subject);
    } else if (c.type === 'perf' || c.type === 'refactor') {
      changed.push(c.subject);
    }
  }

  // Deduplicate entries preserving order
  const dedup = (items: string[]) => [...new Set(items)];
  const addedUnique = dedup(added);
  const fixedUnique = dedup(fixed);
  const changedUnique = dedup(changed);

  const lines: string[] = [`## [${version}] - ${dateStr}`, ''];

  if (addedUnique.length > 0) {
    lines.push('### Added');
    for (const item of addedUnique) {
      lines.push(`- ${item}`);
    }
    lines.push('');
  }

  if (fixedUnique.length > 0) {
    lines.push('### Fixed');
    for (const item of fixedUnique) {
      lines.push(`- ${item}`);
    }
    lines.push('');
  }

  if (changedUnique.length > 0) {
    lines.push('### Changed');
    for (const item of changedUnique) {
      lines.push(`- ${item}`);
    }
    lines.push('');
  }

  return lines.join('\n').trimEnd();
}

/**
 * Prepend a new version section into existing CHANGELOG.md content.
 */
export function updateChangelogContent(existingContent: string, newSection: string): string {
  const headerPrefix = '# Changelog\n\nAll notable changes to this project will be documented in this file.\n\nThe format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),\nand this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).\n\n';

  if (!existingContent.trim()) {
    return `${headerPrefix}${newSection}\n`;
  }

  if (existingContent.startsWith('# Changelog')) {
    // Find the first version header `## [`
    const firstVersionIndex = existingContent.indexOf('\n## [');
    if (firstVersionIndex !== -1) {
      const headerPart = existingContent.slice(0, firstVersionIndex + 1);
      const rest = existingContent.slice(firstVersionIndex + 1);
      return `${headerPart}${newSection}\n\n${rest}`;
    }
    // No version headers found yet
    return `${existingContent.trimEnd()}\n\n${newSection}\n`;
  }

  return `${headerPrefix}${newSection}\n\n${existingContent.trimStart()}`;
}

/**
 * Read the current version from package.json.
 */
export function readCurrentVersion(projectRoot: string): string {
  const pkgPath = path.join(projectRoot, 'package.json');
  const pkg = JSON.parse(fs.readFileSync(pkgPath, 'utf8'));
  return pkg.version;
}

/**
 * Get the latest git tag matching `v*`. Returns null if none exist.
 */
export function getLatestTag(repoDir: string): string | null {
  try {
    const stdout = execSync('git describe --tags --abbrev=0 --match "v*"', {
      cwd: repoDir,
      encoding: 'utf8',
      stdio: ['pipe', 'pipe', 'ignore'],
    }).trim();
    return stdout || null;
  } catch {
    return null;
  }
}

/**
 * Get commit list since the given tag (or all commits if tag is null).
 */
export function getCommitsSinceTag(repoDir: string, tag: string | null): ParsedCommit[] {
  const range = tag ? `${tag}..HEAD` : 'HEAD';
  // Use format: hash%x1fsubject%x1fbody%x1e
  let stdout = '';
  try {
    stdout = execSync(`git log ${range} --format="%H%x1f%s%x1f%b%x1e"`, {
      cwd: repoDir,
      encoding: 'utf8',
    });
  } catch (e) {
    console.error(`Failed to get git log for range ${range}:`, e);
    return [];
  }

  const rawCommits = stdout.split('\x1e').filter(c => c.trim().length > 0);
  const parsed: ParsedCommit[] = [];

  for (const raw of rawCommits) {
    const parts = raw.split('\x1f');
    if (parts.length >= 2) {
      const [hash, subject, body = ''] = parts;
      parsed.push(parseCommit(hash.trim(), subject.trim(), body.trim()));
    }
  }

  return parsed;
}

/**
 * Update version strings across package.json, tauri.conf.json, and Cargo.toml.
 */
export function syncVersionFiles(projectRoot: string, newVersion: string): void {
  // 1. package.json
  const pkgPath = path.join(projectRoot, 'package.json');
  const pkg = JSON.parse(fs.readFileSync(pkgPath, 'utf8'));
  pkg.version = newVersion;
  fs.writeFileSync(pkgPath, `${JSON.stringify(pkg, null, 2)}\n`, 'utf8');

  // 2. src-tauri/tauri.conf.json
  const tauriPath = path.join(projectRoot, 'src-tauri', 'tauri.conf.json');
  const tauri = JSON.parse(fs.readFileSync(tauriPath, 'utf8'));
  tauri.version = newVersion;
  fs.writeFileSync(tauriPath, `${JSON.stringify(tauri, null, 2)}\n`, 'utf8');

  // 3. src-tauri/Cargo.toml
  const cargoPath = path.join(projectRoot, 'src-tauri', 'Cargo.toml');
  let cargoContent = fs.readFileSync(cargoPath, 'utf8');
  cargoContent = cargoContent.replace(
    /(^\[package\][\s\S]*?^version\s*=\s*)"[^"]+"/m,
    `$1"${newVersion}"`
  );
  fs.writeFileSync(cargoPath, cargoContent, 'utf8');
}

/**
 * Main evaluation logic.
 */
export function evaluateRelease(projectRoot: string): ReleaseDecision {
  const currentVersion = readCurrentVersion(projectRoot);
  const latestTag = getLatestTag(projectRoot);
  const commits = getCommitsSinceTag(projectRoot, latestTag);
  const bumpType = determineBumpType(commits, currentVersion);
  const shouldRelease = bumpType !== 'none';
  const nextVersion = calculateNextVersion(currentVersion, bumpType);
  const tagName = `v${nextVersion}`;
  const changelogSection = generateChangelogSection(nextVersion, commits);

  return {
    shouldRelease,
    reason: shouldRelease ? undefined : 'No release-triggering commits found (only docs/chore/ci/test)',
    bumpType,
    currentVersion,
    nextVersion,
    tagName,
    commits,
    changelogSection,
  };
}

// CLI execution
if (import.meta.main) {
  const args = process.argv.slice(2);
  const projectRoot = path.resolve(import.meta.dir, '..');
  const decision = evaluateRelease(projectRoot);

  if (args.includes('--json')) {
    console.log(JSON.stringify(decision, null, 2));
    process.exit(0);
  }

  const outputBodyIdx = args.indexOf('--output-body');
  if (outputBodyIdx !== -1 && args[outputBodyIdx + 1]) {
    const targetFile = path.resolve(args[outputBodyIdx + 1]);
    fs.writeFileSync(targetFile, decision.changelogSection, 'utf8');
  }

  if (args.includes('--apply')) {
    if (!decision.shouldRelease) {
      console.log(`[release-metadata] ${decision.reason}. No release needed.`);
      if (process.env.GITHUB_OUTPUT) {
        fs.appendFileSync(process.env.GITHUB_OUTPUT, 'should_release=false\n');
      }
      process.exit(0);
    }

    console.log(`[release-metadata] Bumping version ${decision.currentVersion} -> ${decision.nextVersion} (${decision.tagName})`);
    syncVersionFiles(projectRoot, decision.nextVersion);

    // Update CHANGELOG.md
    const changelogPath = path.join(projectRoot, 'CHANGELOG.md');
    const existing = fs.existsSync(changelogPath) ? fs.readFileSync(changelogPath, 'utf8') : '';
    const updated = updateChangelogContent(existing, decision.changelogSection);
    fs.writeFileSync(changelogPath, updated, 'utf8');

    // Update Cargo.lock
    try {
      execSync('cargo check --manifest-path src-tauri/Cargo.toml', {
        cwd: projectRoot,
        stdio: 'inherit',
      });
    } catch (e) {
      console.warn('[release-metadata] Note: cargo check exited with an issue while updating Cargo.lock:', e);
    }

    if (process.env.GITHUB_OUTPUT) {
      const bodyPath = path.join(projectRoot, 'release_body.md');
      fs.writeFileSync(bodyPath, decision.changelogSection, 'utf8');

      fs.appendFileSync(process.env.GITHUB_OUTPUT, `should_release=true\n`);
      fs.appendFileSync(process.env.GITHUB_OUTPUT, `version=${decision.nextVersion}\n`);
      fs.appendFileSync(process.env.GITHUB_OUTPUT, `tag_name=${decision.tagName}\n`);
      fs.appendFileSync(process.env.GITHUB_OUTPUT, `release_body_path=${bodyPath}\n`);
    }
  } else {
    console.log(`Current version: ${decision.currentVersion}`);
    console.log(`Should release:  ${decision.shouldRelease}`);
    if (decision.shouldRelease) {
      console.log(`Bump type:       ${decision.bumpType}`);
      console.log(`Next version:    ${decision.nextVersion}`);
      console.log(`Tag name:        ${decision.tagName}`);
      console.log(`\nChangelog preview:\n------------------\n${decision.changelogSection}`);
    } else {
      console.log(`Reason:          ${decision.reason}`);
    }
  }
}
