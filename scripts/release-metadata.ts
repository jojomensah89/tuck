import { execFileSync } from 'node:child_process';
import * as fs from 'node:fs';
import * as path from 'node:path';

export type BumpType = 'major' | 'minor' | 'patch' | 'none';

export interface ParsedCommit {
  hash: string;
  type: string;
  scope?: string;
  isBreaking: boolean;
  subject: string;
  /** The conventional-commit line: the PR title for a merge commit, otherwise the subject. */
  rawSubject: string;
  /** The subject as stored in git, which a `git revert` of a merge commit quotes. */
  gitSubject?: string;
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
  /** Titles that do not follow Conventional Commits, so they can never trigger a release. */
  warnings: string[];
}

const CONVENTIONAL_REGEX =
  /^(?<type>[a-z]+)(?:\((?<scope>[^)]+)\))?(?<breaking>!)?:\s*(?<subject>.+)$/i;
const PR_SUFFIX_REGEX = /\s*\([#\d\s,]+\)$/;
const MERGE_PR_REGEX = /^Merge pull request #(?<prNumber>\d+)/i;
const REVERT_REGEX = /^revert(?:\([^)]*\))?!?:\s*|^revert\s+"/i;
const REVERT_TARGET_REGEX = /^revert(?:\([^)]*\))?!?:\s*(?<plain>.+)$|^revert\s+"(?<quoted>.+)"(?:\s*\([#\d\s,]+\))?$/i;
const REVERTED_HASH_REGEX = /This reverts commit (?<hash>[0-9a-f]{7,40})/i;
const FOOTER_START_REGEX = /^(?:BREAKING[- ]CHANGE|[A-Za-z][A-Za-z0-9-]*)(?:: | #)/;
const BREAKING_FOOTER_REGEX = /^BREAKING[- ]CHANGE: \S/;
const VERSION_TAG_REGEX = /^v\d+\.\d+\.\d+$/;

/**
 * Format a string to sentence case and strip any trailing periods.
 */
export function formatUserFacingSubject(raw: string): string {
  let cleaned = raw.trim().replace(/\.+$/, '').trim();
  cleaned = cleaned.replace(PR_SUFFIX_REGEX, '').trim();
  cleaned = cleaned.replace(/\.+$/, '').trim();
  if (cleaned.length > 0) {
    cleaned = cleaned.charAt(0).toUpperCase() + cleaned.slice(1);
  }
  return cleaned;
}

/**
 * Clean up a conventional commit subject into human-readable user-facing text.
 */
export function cleanCommitSubject(rawSubject: string): { type: string; scope?: string; isBreaking: boolean; subject: string } {
  const trimmed = rawSubject.trim();

  // Check for Revert commits
  if (REVERT_REGEX.test(trimmed)) {
    return {
      type: 'revert',
      isBreaking: false,
      subject: formatUserFacingSubject(trimmed),
    };
  }

  const match = CONVENTIONAL_REGEX.exec(trimmed);
  if (!match || !match.groups) {
    return {
      type: 'chore',
      isBreaking: false,
      subject: formatUserFacingSubject(trimmed),
    };
  }

  const { type, scope, breaking, subject } = match.groups;
  return {
    type: type.toLowerCase(),
    scope: scope?.trim(),
    isBreaking: Boolean(breaking),
    subject: formatUserFacingSubject(subject),
  };
}

/**
 * True when the body ends in a footer section containing BREAKING CHANGE. Footers are the
 * last paragraph, and that paragraph must start with a footer token. A prose line that
 * happens to begin with the words does not count.
 */
export function hasBreakingFooter(body: string): boolean {
  const paragraphs = body.trim().split(/\n\s*\n/);
  const lines = (paragraphs[paragraphs.length - 1] ?? '').split('\n');
  if (!FOOTER_START_REGEX.test(lines[0] ?? '')) return false;
  return lines.some(line => BREAKING_FOOTER_REGEX.test(line));
}

/**
 * Parse raw git commit lines into structured commit metadata.
 */
export function parseCommit(hash: string, rawSubject: string, body = ''): ParsedCommit {
  const { type, scope, isBreaking, subject } = cleanCommitSubject(rawSubject);
  const bodyHasBreaking = hasBreakingFooter(body);

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

function stripPrSuffix(subject: string): string {
  return subject.replace(PR_SUFFIX_REGEX, '').trim();
}

function revertsCommit(revert: ParsedCommit, candidate: ParsedCommit): boolean {
  const revertedHash = REVERTED_HASH_REGEX.exec(revert.body ?? '')?.groups?.hash;
  if (revertedHash && candidate.hash.startsWith(revertedHash)) return true;
  const groups = REVERT_TARGET_REGEX.exec(stripPrSuffix(revert.rawSubject))?.groups;
  const target = groups?.quoted ?? groups?.plain;
  if (!target) return false;
  return [candidate.rawSubject, candidate.gitSubject].some(
    subject => subject !== undefined && stripPrSuffix(subject) === target,
  );
}

/**
 * Removes commits undone by a revert in the same range, so a feature that never shipped
 * neither bumps the version nor appears in the notes. Reverting a revert brings the
 * original back. Takes and returns commits newest first, as `git log` gives them.
 */
export function dropRevertedCommits(commits: readonly ParsedCommit[]): ParsedCommit[] {
  const cancelled = new Set<ParsedCommit>();
  const undoneBy = new Map<ParsedCommit, ParsedCommit>();
  const seen: ParsedCommit[] = [];

  for (const commit of [...commits].reverse()) {
    if (commit.type === 'revert') {
      const target = seen.findLast(candidate => !cancelled.has(candidate) && revertsCommit(commit, candidate));
      if (target) {
        cancelled.add(target);
        const original = undoneBy.get(target);
        if (original) cancelled.delete(original);
        else undoneBy.set(commit, target);
      }
    }
    seen.push(commit);
  }

  return commits.filter(commit => !cancelled.has(commit));
}

/**
 * Determine the bump type from a list of parsed commits according to the release specification:
 * - feat -> minor
 * - fix/perf -> patch
 * - breaking -> minor while below 1.0 (major if >= 1.0)
 * - if only docs/test/chore/ci/style/refactor/build/revert landed, do not release (none)
 */
export function determineBumpType(commits: readonly ParsedCommit[], currentVersion: string): BumpType {
  const isBelowOne = currentVersion.startsWith('0.');
  let hasFeat = false;
  let hasPatch = false;
  let hasBreaking = false;

  for (const c of commits) {
    // Skip release chores
    if (c.type === 'chore' && c.scope === 'release') continue;
    if (c.type === 'revert') continue;

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
    // Skip release chore commits
    if (c.type === 'chore' && c.scope === 'release') continue;
    if (c.type === 'revert') continue;

    if (c.type === 'feat') {
      added.push(c.subject);
    } else if (c.type === 'fix') {
      fixed.push(c.subject);
    } else if (c.type === 'perf' || c.type === 'refactor' || c.isBreaking) {
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

function git(repoDir: string, args: readonly string[]): string {
  return execFileSync('git', args, { cwd: repoDir, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
}

/**
 * The newest `vX.Y.Z` tag reachable from HEAD, or null if there is none.
 * A failing git command throws, so it can never read as "no tags".
 */
export function getLatestTag(repoDir: string): string | null {
  const tags = git(repoDir, ['tag', '--list', 'v*', '--merged', 'HEAD', '--sort=-v:refname']);
  return tags.split(/\r?\n/).find(tag => VERSION_TAG_REGEX.test(tag.trim()))?.trim() ?? null;
}

/**
 * The conventional-commit title of a first-parent commit. This repository's merge commits
 * carry the PR title in the body (merge_commit_message = PR_TITLE). Squash commits carry it
 * in the subject. A PR merge without a title stops the release rather than skipping it.
 */
export function resolveCommitTitle(hash: string, subject: string, body: string): { title: string; body: string } {
  const merge = MERGE_PR_REGEX.exec(subject);
  if (!merge) return { title: subject, body };

  const [title = '', ...rest] = body.trim().split('\n');
  if (!title.trim()) {
    throw new Error(
      `Merge commit ${hash} for PR #${merge.groups?.prNumber} has no PR title in its body, ` +
        'so its release type is unknown. See "Recovery" in docs/releasing.md.',
    );
  }
  return { title: title.trim(), body: rest.join('\n').trim() };
}

/**
 * Get first-parent commits since the given tag, newest first. Throws if git fails, because
 * an empty answer would read as "nothing to release".
 */
export function getCommitsSinceTag(repoDir: string, tag: string): ParsedCommit[] {
  const stdout = git(repoDir, ['log', `${tag}..HEAD`, '--first-parent', '--format=%H%x1f%s%x1f%b%x1e']);
  const parsed: ParsedCommit[] = [];

  for (const raw of stdout.split('\x1e')) {
    if (!raw.trim()) continue;
    const [hash, subject, rawBody = ''] = raw.split('\x1f');
    if (subject === undefined) throw new Error(`Unexpected git log record: ${raw}`);
    const gitSubject = subject.trim();
    const { title, body } = resolveCommitTitle(hash.trim(), gitSubject, rawBody.trim());
    parsed.push({ ...parseCommit(hash.trim(), title, body), gitSubject });
  }

  return parsed;
}

/**
 * Update only the package-owned version fields.  Cargo's resolver can rewrite
 * unrelated dependency entries, which makes a version-only release PR noisy
 * and can accidentally change the dependency graph.  The lockfile helper below
 * updates the root `tuck` package entry only.
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

/** Update the root package entry in Cargo.lock without running Cargo's resolver. */
export function syncCargoLockVersion(projectRoot: string, newVersion: string): void {
  const lockPath = path.join(projectRoot, 'src-tauri', 'Cargo.lock');
  const lockContent = fs.readFileSync(lockPath, 'utf8');
  const blocks = lockContent.split(/(?=^\[\[package\]\]\s*$)/m);
  const rootIndex = blocks.findIndex(block => /^name\s*=\s*"tuck"\s*$/m.test(block));
  if (rootIndex === -1) {
    throw new Error('Cargo.lock has no root tuck package entry; refusing to rewrite the lockfile.');
  }
  const updated = blocks[rootIndex].replace(/(^version\s*=\s*)"[^"]+"/m, `$1"${newVersion}"`);
  if (updated === blocks[rootIndex]) {
    throw new Error('Cargo.lock root tuck package has no version field; refusing to rewrite the lockfile.');
  }
  blocks[rootIndex] = updated;
  fs.writeFileSync(lockPath, blocks.join(''), 'utf8');
}

/**
 * Main evaluation logic.
 */
export function evaluateRelease(projectRoot: string): ReleaseDecision {
  const currentVersion = readCurrentVersion(projectRoot);
  const latestTag = getLatestTag(projectRoot);
  if (!latestTag) {
    throw new Error('No vX.Y.Z tag is reachable from HEAD. Push the baseline tag first: see "First release" in docs/releasing.md.');
  }
  if (latestTag !== `v${currentVersion}`) {
    throw new Error(`Latest tag ${latestTag} does not match package.json version ${currentVersion}. See "Recovery" in docs/releasing.md.`);
  }
  const allCommits = getCommitsSinceTag(projectRoot, latestTag);
  const warnings = allCommits
    .filter(commit => !CONVENTIONAL_REGEX.test(commit.rawSubject) && !REVERT_REGEX.test(commit.rawSubject))
    .map(commit => commit.rawSubject);
  const commits = dropRevertedCommits(allCommits);
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
    warnings,
  };
}

/** The notes shown on the release page and in Settings: the section without its version heading. */
export function releaseNotesBody(changelogSection: string): string {
  return changelogSection.replace(/^## .*\r?\n?/, '').trim();
}

// CLI execution
if (import.meta.main) {
  const args = process.argv.slice(2);
  const projectRoot = path.resolve(import.meta.dir, '..');
  const decision = evaluateRelease(projectRoot);

  for (const title of decision.warnings) {
    console.log(`::warning::Not a Conventional Commit, so it cannot trigger a release: ${title}`);
  }

  if (args.includes('--json')) {
    console.log(JSON.stringify(decision, null, 2));
    process.exit(0);
  }

  if (args.includes('--apply')) {
    const output = process.env.GITHUB_OUTPUT;
    if (!decision.shouldRelease) {
      console.log(`[release-metadata] ${decision.reason}. No release needed.`);
      if (output) fs.appendFileSync(output, 'should_release=false\n');
      process.exit(0);
    }

    const notesIdx = args.indexOf('--notes-file');
    const notesFile = notesIdx === -1 ? undefined : args[notesIdx + 1];
    if (!notesFile) throw new Error('--apply needs --notes-file <path>');
    const notes = releaseNotesBody(decision.changelogSection);
    if (!notes) throw new Error(`No user-facing notes for ${decision.tagName}. See "Recovery" in docs/releasing.md.`);

    console.log(`[release-metadata] Bumping version ${decision.currentVersion} -> ${decision.nextVersion} (${decision.tagName})`);
    syncVersionFiles(projectRoot, decision.nextVersion);

    const changelogPath = path.join(projectRoot, 'CHANGELOG.md');
    const existing = fs.existsSync(changelogPath) ? fs.readFileSync(changelogPath, 'utf8') : '';
    fs.writeFileSync(changelogPath, updateChangelogContent(existing, decision.changelogSection), 'utf8');

    // Keep this PR version-only. Running `cargo update` can change unrelated
    // dependency versions, so synchronize only Tuck's lockfile entry.
    syncCargoLockVersion(projectRoot, decision.nextVersion);

    fs.writeFileSync(path.resolve(notesFile), `${notes}\n`, 'utf8');
    if (output) {
      fs.appendFileSync(output, `should_release=true\nversion=${decision.nextVersion}\n`, 'utf8');
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
