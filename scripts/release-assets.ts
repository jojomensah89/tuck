import { createHash, createPublicKey, verify } from 'node:crypto';
import * as fs from 'node:fs';
import * as path from 'node:path';

/** Prefix that turns a raw 32-byte Ed25519 key into DER SubjectPublicKeyInfo. */
const ED25519_SPKI_PREFIX = Buffer.from('302a300506032b6570032100', 'hex');

function base64Lines(encoded: string, label: string): string[] {
  const lines = Buffer.from(encoded.trim(), 'base64').toString('utf8').split(/\r?\n/);
  if (lines.length < 2) throw new Error(`${label} is not a minisign file`);
  return lines;
}

/**
 * Checks a Tauri `.sig` against the `pubkey` value from tauri.conf.json, the same way the
 * updater does: the file signature, the key id, and the signature over the trusted comment.
 * Throws on any mismatch. Publishing a release the app would reject strands every install.
 */
export function verifyUpdaterSignature(file: Uint8Array, sigFileContents: string, pubkey: string): void {
  const keyBytes = Buffer.from(base64Lines(pubkey, 'Public key')[1], 'base64');
  if (keyBytes.length !== 42 || keyBytes.subarray(0, 2).toString('latin1') !== 'Ed') {
    throw new Error('Public key is not a minisign Ed25519 key');
  }
  const keyId = keyBytes.subarray(2, 10);
  const key = createPublicKey({
    key: Buffer.concat([ED25519_SPKI_PREFIX, keyBytes.subarray(10)]),
    format: 'der',
    type: 'spki',
  });

  const lines = base64Lines(sigFileContents, 'Signature');
  const sigBytes = Buffer.from(lines[1], 'base64');
  const algorithm = sigBytes.subarray(0, 2).toString('latin1');
  if (sigBytes.length !== 74 || (algorithm !== 'ED' && algorithm !== 'Ed')) {
    throw new Error('Signature is not a minisign Ed25519 signature');
  }
  if (!sigBytes.subarray(2, 10).equals(keyId)) {
    throw new Error('Signature was made by a different key than the one in tauri.conf.json');
  }
  const signature = sigBytes.subarray(10);
  const signed = algorithm === 'ED' ? createHash('blake2b512').update(file).digest() : file;
  if (!verify(null, signed, key, signature)) {
    throw new Error('Signature does not match the installer');
  }

  const trustedLine = lines[2] ?? '';
  const prefix = 'trusted comment: ';
  if (!trustedLine.startsWith(prefix)) throw new Error('Signature has no trusted comment');
  const globalSignature = Buffer.from(lines[3] ?? '', 'base64');
  const globalSigned = Buffer.concat([signature, Buffer.from(trustedLine.slice(prefix.length), 'utf8')]);
  if (globalSignature.length !== 64 || !verify(null, globalSigned, key, globalSignature)) {
    throw new Error('Signature has an invalid trusted comment');
  }
}

export interface LatestJsonInput {
  version: string;
  notes: string;
  pubDate: Date;
  signature: string;
  url: string;
}

/** The updater manifest. Both Windows keys point at the single NSIS installer. */
export function buildLatestJson({ version, notes, pubDate, signature, url }: LatestJsonInput): object {
  const platform = { signature: signature.trim(), url };
  return {
    version,
    notes,
    pub_date: pubDate.toISOString().replace(/\.\d{3}Z$/, 'Z'),
    platforms: {
      'windows-x86_64': platform,
      'windows-x86_64-nsis': platform,
    },
  };
}

/** Turns the configured `.../releases/latest/download/latest.json` endpoint into a versioned asset URL. */
export function assetUrl(endpoint: string, tag: string, asset: string): string {
  const suffix = '/latest/download/latest.json';
  if (!endpoint.endsWith(suffix)) throw new Error(`Unexpected updater endpoint: ${endpoint}`);
  return `${endpoint.slice(0, -suffix.length)}/download/${tag}/${encodeURIComponent(asset)}`;
}

function argument(args: readonly string[], name: string): string {
  const index = args.indexOf(name);
  const value = index === -1 ? undefined : args[index + 1];
  if (!value) throw new Error(`Missing ${name}`);
  return value;
}

/**
 * Collects the output of one build into a directory: installer, signature, latest.json and
 * notes. Run once after `tauri build`, so what gets published is that exact build.
 */
if (import.meta.main) {
  const args = process.argv.slice(2);
  const version = argument(args, '--version');
  const notesFile = argument(args, '--notes-file');
  const outDir = argument(args, '--out');
  const projectRoot = path.resolve(import.meta.dir, '..');

  const config = JSON.parse(fs.readFileSync(path.join(projectRoot, 'src-tauri', 'tauri.conf.json'), 'utf8'));
  if (config.version !== version) throw new Error(`tauri.conf.json has ${config.version}, expected ${version}`);

  const tag = `v${version}`;
  const installer = `${config.productName}_${version}_x64-setup.exe`;
  const bundleDir = path.join(projectRoot, 'src-tauri', 'target', 'release', 'bundle', 'nsis');
  const installerBytes = fs.readFileSync(path.join(bundleDir, installer));
  const signature = fs.readFileSync(path.join(bundleDir, `${installer}.sig`), 'utf8');

  verifyUpdaterSignature(installerBytes, signature, config.plugins.updater.pubkey);
  console.log(`[release-assets] ${installer}: signature verified against tauri.conf.json`);

  fs.mkdirSync(outDir, { recursive: true });
  fs.copyFileSync(path.join(bundleDir, installer), path.join(outDir, installer));
  fs.writeFileSync(path.join(outDir, `${installer}.sig`), signature, 'utf8');
  const notes = fs.readFileSync(notesFile, 'utf8').trim();
  fs.writeFileSync(path.join(outDir, 'release-notes.md'), `${notes}\n`, 'utf8');
  const latest = buildLatestJson({
    version,
    notes,
    pubDate: new Date(),
    signature,
    url: assetUrl(config.plugins.updater.endpoints[0], tag, installer),
  });
  fs.writeFileSync(path.join(outDir, 'latest.json'), `${JSON.stringify(latest, null, 2)}\n`, 'utf8');
}
