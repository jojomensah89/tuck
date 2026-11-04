import { describe, expect, it } from 'bun:test';
import { createHash, generateKeyPairSync, sign, type KeyObject } from 'node:crypto';
import { assetUrl, buildLatestJson, verifyUpdaterSignature } from '../scripts/release-assets';

const KEY_ID = Buffer.from('0123456789abcdef', 'hex');

function makeKey(): { pubkey: string; privateKey: KeyObject } {
  const { publicKey, privateKey } = generateKeyPairSync('ed25519');
  const raw = publicKey.export({ format: 'der', type: 'spki' }).subarray(-32);
  const body = Buffer.concat([Buffer.from('Ed', 'latin1'), KEY_ID, raw]).toString('base64');
  return { pubkey: Buffer.from(`untrusted comment: test key\n${body}\n`).toString('base64'), privateKey };
}

/** Signs the way `tauri signer` does: prehashed minisign, base64-wrapped. */
function makeSig(file: Uint8Array, privateKey: KeyObject, keyId = KEY_ID, trusted = 'timestamp:1\tfile:x'): string {
  const signature = sign(null, createHash('blake2b512').update(file).digest(), privateKey);
  const line = Buffer.concat([Buffer.from('ED', 'latin1'), keyId, signature]).toString('base64');
  const global = sign(null, Buffer.concat([signature, Buffer.from(trusted)]), privateKey).toString('base64');
  return Buffer.from(`untrusted comment: sig\n${line}\ntrusted comment: ${trusted}\n${global}\n`).toString('base64');
}

describe('updater signature check', () => {
  const file = Buffer.from('installer bytes');

  it('accepts a signature from the configured key', () => {
    const { pubkey, privateKey } = makeKey();
    expect(() => verifyUpdaterSignature(file, makeSig(file, privateKey), pubkey)).not.toThrow();
  });

  it('rejects a changed installer', () => {
    const { pubkey, privateKey } = makeKey();
    expect(() => verifyUpdaterSignature(Buffer.from('tampered'), makeSig(file, privateKey), pubkey)).toThrow(/does not match/);
  });

  it('rejects a different key with the same id', () => {
    const { pubkey } = makeKey();
    const other = makeKey();
    expect(() => verifyUpdaterSignature(file, makeSig(file, other.privateKey), pubkey)).toThrow(/does not match/);
  });

  it('rejects a different key id', () => {
    const { pubkey, privateKey } = makeKey();
    const sig = makeSig(file, privateKey, Buffer.from('ffffffffffffffff', 'hex'));
    expect(() => verifyUpdaterSignature(file, sig, pubkey)).toThrow(/different key/);
  });

  it('rejects an edited trusted comment', () => {
    const { pubkey, privateKey } = makeKey();
    const decoded = Buffer.from(makeSig(file, privateKey), 'base64').toString().replace('timestamp:1', 'timestamp:2');
    expect(() => verifyUpdaterSignature(file, Buffer.from(decoded).toString('base64'), pubkey)).toThrow(/trusted comment/);
  });
});

describe('latest.json', () => {
  it('points both Windows keys at the installer with the signature contents', () => {
    const latest = buildLatestJson({
      version: '0.2.0',
      notes: '### Added\n- Thing',
      pubDate: new Date('2026-11-04T12:00:00.123Z'),
      signature: 'c2ln\n',
      url: 'https://example.test/tuck_0.2.0_x64-setup.exe',
    });
    const platform = { signature: 'c2ln', url: 'https://example.test/tuck_0.2.0_x64-setup.exe' };
    expect(latest).toEqual({
      version: '0.2.0',
      notes: '### Added\n- Thing',
      pub_date: '2026-11-04T12:00:00Z',
      platforms: { 'windows-x86_64': platform, 'windows-x86_64-nsis': platform },
    });
  });

  it('builds the versioned asset URL from the configured endpoint', () => {
    expect(assetUrl('https://github.com/o/r/releases/latest/download/latest.json', 'v0.2.0', 'tuck_0.2.0_x64-setup.exe'))
      .toBe('https://github.com/o/r/releases/download/v0.2.0/tuck_0.2.0_x64-setup.exe');
    expect(() => assetUrl('https://example.test/other.json', 'v1.0.0', 'x')).toThrow(/Unexpected updater endpoint/);
  });
});
