import { describe, expect, it } from 'bun:test';
import { readFileSync } from 'node:fs';

type Job = { if?: string; needs?: string | string[]; permissions?: Record<string, string>; steps?: unknown[] };
const workflow = Bun.YAML.parse(readFileSync(new URL('../.github/workflows/release.yml', import.meta.url), 'utf8')) as { jobs: Record<string, Job> };
const jobs = workflow.jobs;

// Evaluate the small boolean gate subset used by this workflow, with fixture
// GitHub contexts. This checks recovery/signing behavior when jobs are skipped.
function runs(job: string, overrides: Record<string, unknown> = {}): boolean {
  const context: Record<string, unknown> = {
    'github.event_name': 'push', 'github.ref': 'refs/heads/main',
    'inputs.dry_run': false, 'needs.prepare.result': 'success',
    'needs.check.result': 'success', 'needs.build.result': 'skipped',
    'needs.recover.result': 'success', 'needs.prepare.outputs.build': 'false',
    ...overrides,
  };
  const expression = (jobs[job].if ?? 'true').replace(/always\(\)/g, 'true')
    .replace(/\b(?:github|inputs|needs)(?:\.[A-Za-z_]+)+\b/g, key => {
      if (!(key in context)) throw new Error(`Unmodelled workflow context: ${key}`);
      return JSON.stringify(context[key]);
    });
  return Boolean(Function(`"use strict"; return (${expression});`)());
}

describe('release workflow safety gates', () => {
  it('recovers a tagged draft on no-release merges or full reruns without rebuilding', () => {
    expect(jobs.recover.needs).toEqual(['prepare', 'check']);
    expect(runs('recover')).toBe(true);
    expect(runs('build')).toBe(false);
    expect(runs('publish')).toBe(false);
  });
  it('never signs after failed or cancelled checks', () => {
    for (const result of ['failure', 'cancelled', 'skipped']) {
      expect(runs('build', { 'needs.check.result': result, 'needs.prepare.outputs.build': 'true' })).toBe(false);
      expect(runs('recover', { 'needs.check.result': result })).toBe(false);
    }
  });
  it('dry runs build checked code but cannot prepare PRs, recover or publish', () => {
    const context = { 'github.event_name': 'workflow_dispatch', 'inputs.dry_run': true };
    expect(runs('prepare', context)).toBe(false);
    expect(runs('build', context)).toBe(true);
    expect(runs('recover', context)).toBe(false);
    expect(runs('publish', { ...context, 'needs.build.result': 'success' })).toBe(false);
  });
  it('never publishes from a feature branch or after recovery failure', () => {
    expect(runs('publish', { 'needs.build.result': 'success', 'github.ref': 'refs/heads/feature' })).toBe(false);
    expect(runs('publish', { 'needs.build.result': 'success', 'needs.recover.result': 'failure' })).toBe(false);
    expect(runs('publish', { 'needs.build.result': 'success' })).toBe(true);
  });
  it('checks provenance after the build and before uploading signed assets', () => {
    const steps = jobs.build.steps as { run?: string; uses?: string }[];
    const build = steps.findIndex(step => step.run === 'bun run tauri build');
    const clean = steps.findIndex(step => step.run?.includes('release-steps.sh verify-clean'));
    const upload = steps.findIndex(step => step.uses?.startsWith('actions/upload-artifact@'));
    expect(clean).toBeGreaterThan(build);
    expect(upload).toBeGreaterThan(clean);
    expect(jobs.build.permissions?.contents).toBe('read');
    for (const job of ['prepare', 'recover', 'publish']) {
      expect(JSON.stringify(jobs[job])).not.toContain('TAURI_SIGNING_PRIVATE_KEY');
    }
  });
});
