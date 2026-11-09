import { afterEach, expect, test } from 'bun:test';
import { mockIPC, clearMocks } from '@tauri-apps/api/mocks';
import { checkForUpdates, startBackgroundUpdateScheduler, subscribeUpdate, updateAction } from '../src/updater';
afterEach(() => { clearMocks(); Reflect.deleteProperty(globalThis, 'window'); });
test('Store identity suppresses checks, actions, subscriptions and timers', async () => {
  const calls: string[] = [];
  Object.assign(globalThis, {window: {setTimeout: () => { throw new Error('Store scheduled an update'); }}});
  mockIPC(cmd => { calls.push(cmd); if (cmd === 'is_store_build') return true; throw new Error(`Unexpected updater IPC: ${cmd}`); });
  expect(await checkForUpdates(true)).toBeNull();
  await startBackgroundUpdateScheduler(0);
  await updateAction();
  await subscribeUpdate(() => { throw new Error('Store offered an update'); });
  expect(calls).toEqual(['is_store_build','is_store_build','is_store_build','is_store_build']);
});
test('unpackaged identity retains update checks', async () => {
  Object.assign(globalThis, {window: {}});
  mockIPC(cmd => cmd === 'is_store_build' ? false : {version:'1.0.0',currentVersion:'0.3.0'});
  expect((await checkForUpdates(true))?.version).toBe('1.0.0');
});
