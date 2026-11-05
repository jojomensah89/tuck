import { expect, test } from 'bun:test';
import { UsageSnapshotGate } from '../usage-snapshot.js';
test('an event arriving during a pull wins, even if the pull returns last', () => {
  const gate = new UsageSnapshotGate();
  const old = gate.begin(); gate.changed();
  expect(gate.accepts(old)).toBe(false);
  expect(gate.accepts(gate.begin())).toBe(true);
});
test('newer open requests supersede older pulls without an event', () => {
  const gate = new UsageSnapshotGate(); const first = gate.begin(); const second = gate.begin();
  expect(gate.accepts(first)).toBe(false); expect(gate.accepts(second)).toBe(true);
  gate.changed(); expect(gate.accepts(second)).toBe(false);
});
