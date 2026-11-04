import { describe, expect, it } from 'bun:test';
import {
  calculateNextVersion,
  cleanCommitSubject,
  determineBumpType,
  formatUserFacingSubject,
  generateChangelogSection,
  parseCommit,
  updateChangelogContent,
  type ParsedCommit,
} from '../scripts/release-metadata';

describe('release metadata & conventional commit parser', () => {
  it('cleans and parses conventional commit subjects to plain user-facing text', () => {
    const feat = cleanCommitSubject('feat(tray): add left-click toggle (#21).');
    expect(feat.type).toBe('feat');
    expect(feat.scope).toBe('tray');
    expect(feat.isBreaking).toBe(false);
    expect(feat.subject).toBe('Add left-click toggle'); // stripped scope/type, sentence case, no trailing period

    const fix = cleanCommitSubject('fix: address clippy warnings.');
    expect(fix.type).toBe('fix');
    expect(fix.scope).toBeUndefined();
    expect(fix.isBreaking).toBe(false);
    expect(fix.subject).toBe('Address clippy warnings');

    const breaking = cleanCommitSubject('feat(api)!: redesign state store.');
    expect(breaking.type).toBe('feat');
    expect(breaking.scope).toBe('api');
    expect(breaking.isBreaking).toBe(true);
    expect(breaking.subject).toBe('Redesign state store');

    const nonConventional = cleanCommitSubject('merge remote branch.');
    expect(nonConventional.type).toBe('chore');
    expect(nonConventional.subject).toBe('Merge remote branch');
  });

  it('handles revert commits and does not trigger release', () => {
    const revertCommit = cleanCommitSubject('Revert "feat: add experimental feature"');
    expect(revertCommit.type).toBe('revert');
    expect(revertCommit.subject).toBe('Revert "feat: add experimental feature"');

    const parsedRevert = parseCommit('123', 'Revert "feat: add experimental feature"');
    expect(determineBumpType([parsedRevert], '0.1.0')).toBe('none');
  });

  it('handles docs!: breaking changes sensibly with a non-empty section', () => {
    const docsBreaking = parseCommit('1', 'docs!: rewrite entire user manual and architecture specs');
    expect(docsBreaking.isBreaking).toBe(true);
    expect(determineBumpType([docsBreaking], '0.1.0')).toBe('minor');

    const section = generateChangelogSection('0.2.0', [docsBreaking], '2026-11-04');
    expect(section).toContain('### Changed');
    expect(section).toContain('- Rewrite entire user manual and architecture specs');
    expect(section.trim()).not.toBe('## [0.2.0] - 2026-11-04');
  });

  it('strictly adheres to BREAKING CHANGE footer rule (uppercase line start only)', () => {
    // Valid breaking change in footer
    const validBreaking = parseCommit(
      'abc1234',
      'fix: change config format',
      'BREAKING CHANGE: the config file syntax has changed'
    );
    expect(validBreaking.isBreaking).toBe(true);

    const validHyphen = parseCommit(
      'abc1234',
      'fix: change config format',
      'BREAKING-CHANGE: the config file syntax has changed'
    );
    expect(validHyphen.isBreaking).toBe(true);

    // Mid-sentence or lowercase does NOT trigger breaking change
    const lowercaseInline = parseCommit(
      'def5678',
      'docs: update guide',
      'This documentation commit mentions a breaking change in passing'
    );
    expect(lowercaseInline.isBreaking).toBe(false);
    expect(determineBumpType([lowercaseInline], '0.1.0')).toBe('none');
  });

  it('skips chore(release) commits from triggering bumps and changelogs', () => {
    const releaseChore = parseCommit('abc', 'chore(release): v0.1.0 [skip ci]');
    expect(determineBumpType([releaseChore], '0.1.0')).toBe('none');

    const section = generateChangelogSection('0.2.0', [releaseChore], '2026-11-04');
    expect(section).not.toContain('v0.1.0');
    expect(section).not.toContain('Chore');
  });

  it('handles this repo real commit and PR shapes', () => {
    // Real PR title from repo: "Harden offline dictation and clipboard history"
    const prCommit1 = parseCommit('25eeddd', 'feat: harden offline dictation and clipboard history (#14)');
    expect(prCommit1.type).toBe('feat');
    expect(prCommit1.subject).toBe('Harden offline dictation and clipboard history');

    // Real PR title from repo: "Collapse instantly when the pointer leaves"
    const prCommit2 = parseCommit('a855430', 'fix: collapse instantly when the pointer leaves (#15)');
    expect(prCommit2.type).toBe('fix');
    expect(prCommit2.subject).toBe('Collapse instantly when the pointer leaves');

    // Real direct commit: "docs(research): resource use, lyrics matching and dictation model catalog (#21)"
    const docCommit = parseCommit('31248d0', 'docs(research): resource use, lyrics matching and dictation model catalog (#21)');
    expect(docCommit.type).toBe('docs');
    expect(docCommit.subject).toBe('Resource use, lyrics matching and dictation model catalog');

    // Real PR 22: "feat: release pipeline, in-app updater and full tray menu"
    const pr22 = parseCommit('cc72787', 'feat: release pipeline, in-app updater and full tray menu');
    expect(pr22.type).toBe('feat');
    expect(pr22.subject).toBe('Release pipeline, in-app updater and full tray menu');

    const bump = determineBumpType([docCommit, pr22], '0.1.0');
    expect(bump).toBe('minor');

    const section = generateChangelogSection('0.2.0', [docCommit, pr22], '2026-11-04');
    expect(section).toContain('### Added');
    expect(section).toContain('- Release pipeline, in-app updater and full tray menu');
    expect(section).not.toContain('Resource use'); // docs skipped from user-facing changelog
  });

  it('determines bump type according to release rules', () => {
    const docsOnly: ParsedCommit[] = [
      parseCommit('1', 'docs: update readme'),
      parseCommit('2', 'chore: update deps'),
      parseCommit('3', 'test: add unit tests'),
      parseCommit('4', 'ci: pin actions'),
    ];
    expect(determineBumpType(docsOnly, '0.1.0')).toBe('none');

    const patchCommits: ParsedCommit[] = [
      parseCommit('1', 'docs: update readme'),
      parseCommit('2', 'fix(native): handle null pointer'),
    ];
    expect(determineBumpType(patchCommits, '0.1.0')).toBe('patch');

    const perfCommits: ParsedCommit[] = [
      parseCommit('1', 'perf: optimize hit testing loop'),
    ];
    expect(determineBumpType(perfCommits, '0.1.0')).toBe('patch');

    const featCommits: ParsedCommit[] = [
      parseCommit('1', 'fix: fix minor glitch'),
      parseCommit('2', 'feat(tray): add fuller tray menu'),
    ];
    expect(determineBumpType(featCommits, '0.1.0')).toBe('minor');

    // Breaking while below 1.0 -> minor
    const breakingBelowOne: ParsedCommit[] = [
      parseCommit('1', 'feat!: completely rewrite IPC protocol'),
    ];
    expect(determineBumpType(breakingBelowOne, '0.1.0')).toBe('minor');

    // Breaking at or above 1.0 -> major
    const breakingAboveOne: ParsedCommit[] = [
      parseCommit('1', 'feat!: rewrite public API'),
    ];
    expect(determineBumpType(breakingAboveOne, '1.2.0')).toBe('major');
  });

  it('calculates the next semver version correctly', () => {
    expect(calculateNextVersion('0.1.0', 'none')).toBe('0.1.0');
    expect(calculateNextVersion('0.1.0', 'patch')).toBe('0.1.1');
    expect(calculateNextVersion('0.1.0', 'minor')).toBe('0.2.0');
    expect(calculateNextVersion('1.0.0', 'minor')).toBe('1.1.0');
    expect(calculateNextVersion('1.2.3', 'major')).toBe('2.0.0');
  });

  it('generates a Keep a Changelog section', () => {
    const commits: ParsedCommit[] = [
      parseCommit('1', 'feat(tray): add left-click toggle'),
      parseCommit('2', 'feat(updater): add in-app updater'),
      parseCommit('3', 'fix: address clippy warning'),
      parseCommit('4', 'perf: reduce CPU idle usage'),
      parseCommit('5', 'docs: add releasing guide'),
    ];

    const section = generateChangelogSection('0.2.0', commits, '2026-11-04');
    expect(section).toContain('## [0.2.0] - 2026-11-04');
    expect(section).toContain('### Added');
    expect(section).toContain('- Add left-click toggle');
    expect(section).toContain('- Add in-app updater');
    expect(section).toContain('### Fixed');
    expect(section).toContain('- Address clippy warning');
    expect(section).toContain('### Changed');
    expect(section).toContain('- Reduce CPU idle usage');
    // docs should not appear in release notes
    expect(section).not.toContain('Add releasing guide');
  });

  it('prepends a new section into CHANGELOG.md properly', () => {
    const initialContent = '';
    const section1 = '## [0.2.0] - 2026-11-04\n\n### Added\n- Feature A';
    const updated1 = updateChangelogContent(initialContent, section1);
    expect(updated1).toContain('# Changelog');
    expect(updated1).toContain('## [0.2.0]');

    const section2 = '## [0.3.0] - 2026-11-10\n\n### Added\n- Feature B';
    const updated2 = updateChangelogContent(updated1, section2);
    expect(updated2.indexOf('## [0.3.0]')).toBeLessThan(updated2.indexOf('## [0.2.0]'));
  });
});
