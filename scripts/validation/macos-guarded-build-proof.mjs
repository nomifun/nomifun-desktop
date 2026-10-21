// Read-only admission for the disposable lifecycle driver. An external build
// receipt associates reviewed guard/main inputs with this exact signed Host.
// The runtime nonce receipt remains a separate, required second check.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { readFile, realpath } from 'node:fs/promises';
import { isAbsolute, join } from 'node:path';
import { spawn } from 'node:child_process';

export const GUARDED_BUILD_PROOF_SCHEMA = 'nomifun.macos-lifecycle-guarded-build.v1';
const GUARD_SOURCE = 'apps/desktop/src/macos_lifecycle_fixture.rs';
const MAIN_SOURCE = 'apps/desktop/src/main.rs';
// These are compiled main/guard diagnostics. The external source/build link
// cannot accidentally authorize a pre-guard debug Host merely by hashing it.
const GUARD_MARKERS = ['MACOS_LIFECYCLE_FIXTURE_REFUSED: ', 'fixture root and token must be provided together'];
const digest = bytes => createHash('sha256').update(bytes).digest('hex');
const requireHash = value => assert.match(value ?? '', /^[0-9a-f]{64}$/, 'A canonical SHA-256 is required');

async function checkedRecord(reference, label) {
  assert(isAbsolute(reference?.path ?? ''), `${label} must identify an absolute existing evidence file`);
  requireHash(reference.sha256);
  const bytes = await readFile(reference.path);
  assert.equal(digest(bytes), reference.sha256, `${label} bytes differ from the external proof`);
  try { return JSON.parse(bytes); }
  catch { throw new Error(`${label} must contain valid JSON`); }
}

async function inspectHost(path) {
  const hash = createHash('sha256');
  const found = new Set();
  const carryLength = Math.max(...GUARD_MARKERS.map(value => Buffer.byteLength(value))) - 1;
  let carry = Buffer.alloc(0);
  for await (const bytes of createReadStream(path)) {
    hash.update(bytes);
    const searchable = Buffer.concat([carry, bytes]);
    for (const marker of GUARD_MARKERS) if (searchable.includes(marker)) found.add(marker);
    carry = searchable.subarray(Math.max(0, searchable.length - carryLength));
  }
  return { sha256: hash.digest('hex'), guard_markers_present: found.size === GUARD_MARKERS.length };
}

async function strictSignature(app) {
  await new Promise((accept, reject) => {
    const child = spawn('/usr/bin/codesign', ['--verify', '--deep', '--strict', app], { stdio: 'ignore' });
    child.once('error', () => reject(new Error('Strict App signature verification could not start')));
    child.once('close', code => code === 0 ? accept() : reject(new Error('Strict App signature verification failed')));
  });
}

export async function verifyGuardedMacosBuild({ proofPath, appPath, repoRoot }, { verifySignature = strictSignature } = {}) {
  assert(isAbsolute(proofPath ?? ''), 'Pass --guarded-build-proof with an absolute external build/source proof before launch');
  const proofBytes = await readFile(proofPath);
  let proof;
  try { proof = JSON.parse(proofBytes); }
  catch { throw new Error('The guarded build proof must contain valid JSON'); }
  assert.equal(proof.schema, GUARDED_BUILD_PROOF_SCHEMA);
  assert.equal(proof.guard_contract, 'debug-direct-temp-v1');
  const app = await realpath(appPath);
  assert.equal(await realpath(proof.app_path), app, 'The proof must name this exact App');
  requireHash(proof.signed_host_sha256);
  const [build, source] = await Promise.all([
    checkedRecord(proof.build_record, 'Build artifact record'),
    checkedRecord(proof.source_record, 'Reviewed source record'),
  ]);
  assert.equal(build.profile, 'debug', 'Only the existing debug guard build can enter this fixture');
  assert.equal(build.bundle_identifier, 'com.nomifun.desktop.dev');
  assert.equal(await realpath(build.app), app);
  assert.equal(build.inspection?.status, 'pass');
  assert.match(build.source_head ?? '', /^[0-9a-f]{40}(?:[0-9a-f]{24})?$/, 'Build evidence must name its actual source baseline');
  assert.equal(build.source_head, source.source_head, 'Build and source records must share their source baseline');
  const hostBuild = build.identities?.['MacOS/nomifun-desktop'];
  requireHash(hostBuild?.source_sha256);
  assert.equal(hostBuild.before_signing_sha256, hostBuild.source_sha256, 'The build record must prove exact Host supply before signing');
  assert.equal(hostBuild.signed_sha256, proof.signed_host_sha256, 'Signed Host identity must come from the build record');
  for (const name of [GUARD_SOURCE, MAIN_SOURCE]) {
    requireHash(proof.source_sha256?.[name]);
    assert.equal(source.modified_source_sha256?.[name], proof.source_sha256[name], 'Reviewed source identity is not associated with this proof');
    assert.equal(digest(await readFile(join(repoRoot, name))), proof.source_sha256[name], 'Reviewed guard/main source changed; supply its corresponding build proof');
  }
  const host = await inspectHost(join(app, 'Contents/MacOS/nomifun-desktop'));
  assert.equal(host.sha256, proof.signed_host_sha256, 'The selected App Host differs from the guarded build proof');
  assert(host.guard_markers_present, 'The selected Host predates the compiled lifecycle guard');
  await verifySignature(app);
  return { proof_path: await realpath(proofPath), proof_sha256: digest(proofBytes), app_path: app,
    signed_host_sha256: host.sha256, guard_contract: proof.guard_contract,
    source_sha256: proof.source_sha256, build_record: proof.build_record, source_record: proof.source_record,
    compiled_guard_markers_present: true, strict_signature_verified: true,
    scope: 'Prelaunch test-build admission only; the matching runtime nonce receipt remains mandatory' };
}
