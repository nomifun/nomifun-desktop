import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, writeFile, rm } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { GUARDED_BUILD_PROOF_SCHEMA, verifyGuardedMacosBuild } from './macos-guarded-build-proof.mjs';

const sha = bytes => createHash('sha256').update(bytes).digest('hex');
async function fixture() {
  const root = await mkdtemp(join(tmpdir(), 'nomifun-guarded-build-proof-'));
  const app = join(root, 'NomiFun Dev.app'), repo = join(root, 'source');
  await mkdir(join(app, 'Contents/MacOS'), { recursive: true });
  await mkdir(join(repo, 'apps/desktop/src'), { recursive: true });
  const host = join(app, 'Contents/MacOS/nomifun-desktop');
  const bytes = Buffer.from('owned test bytes: MACOS_LIFECYCLE_FIXTURE_REFUSED: fixture root and token must be provided together');
  await writeFile(host, bytes);
  const source_sha256 = {};
  for (const name of ['apps/desktop/src/main.rs', 'apps/desktop/src/macos_lifecycle_fixture.rs']) {
    await writeFile(join(repo, name), `reviewed ${name}`);
    source_sha256[name] = sha(await readFile(join(repo, name)));
  }
  const build = { app, profile: 'debug', bundle_identifier: 'com.nomifun.desktop.dev', source_head: 'a'.repeat(40), inspection: { status: 'pass' },
    identities: { 'MacOS/nomifun-desktop': { source_sha256: 'a'.repeat(64), before_signing_sha256: 'a'.repeat(64), signed_sha256: sha(bytes) } } };
  const source = { source_head: 'a'.repeat(40), modified_source_sha256: source_sha256 };
  const buildPath = join(root, 'build.json'), sourcePath = join(root, 'source.json'), proofPath = join(root, 'proof.json');
  await writeFile(buildPath, JSON.stringify(build)); await writeFile(sourcePath, JSON.stringify(source));
  const proof = { schema: GUARDED_BUILD_PROOF_SCHEMA, guard_contract: 'debug-direct-temp-v1', app_path: app,
    signed_host_sha256: sha(bytes), source_sha256, build_record: { path: buildPath, sha256: sha(await readFile(buildPath)) },
    source_record: { path: sourcePath, sha256: sha(await readFile(sourcePath)) } };
  const save = () => writeFile(proofPath, JSON.stringify(proof)); await save();
  return { root, app, repo, host, proof, proofPath, save };
}

test('read-only prelaunch proof binds exact signed Host to reviewed source/build records', async () => {
  const f = await fixture(); let signatureChecks = 0;
  try {
    const result = await verifyGuardedMacosBuild({ proofPath: f.proofPath, appPath: f.app, repoRoot: f.repo }, {
      verifySignature: async path => { signatureChecks++; assert.equal(path, await import('node:fs/promises').then(fs => fs.realpath(f.app))); },
    });
    assert.equal(signatureChecks, 1); assert.equal(result.signed_host_sha256, f.proof.signed_host_sha256);
    assert.equal(result.compiled_guard_markers_present, true); assert.equal(result.strict_signature_verified, true);
  } finally { await rm(f.root, { recursive: true, force: true }); }
});

test('wrong Host or changed source/build evidence refuses before signature or native launch', async () => {
  const f = await fixture(); let nativeCalls = 0;
  const verify = () => verifyGuardedMacosBuild({ proofPath: f.proofPath, appPath: f.app, repoRoot: f.repo }, { verifySignature: async () => { nativeCalls++; } });
  try {
    await writeFile(f.host, 'stale selected Host'); await assert.rejects(verify, /Host differs/);
    await writeFile(f.host, 'owned test bytes: MACOS_LIFECYCLE_FIXTURE_REFUSED: fixture root and token must be provided together');
    await writeFile(join(f.repo, 'apps/desktop/src/main.rs'), 'changed main'); await assert.rejects(verify, /source changed/);
    await writeFile(join(f.repo, 'apps/desktop/src/main.rs'), 'reviewed apps/desktop/src/main.rs');
    await writeFile(f.proof.build_record.path, `${await readFile(f.proof.build_record.path, 'utf8')}\n`);
    await assert.rejects(verify, /Build artifact record bytes differ/);
    assert.equal(nativeCalls, 0);
  } finally { await rm(f.root, { recursive: true, force: true }); }
});

test('missing proof, pre-guard debug bytes, and strict-signature failures remain rejected', async () => {
  const f = await fixture(); let nativeCalls = 0;
  try {
    await assert.rejects(verifyGuardedMacosBuild({ appPath: f.app, repoRoot: f.repo }), /guarded-build-proof/);
    const stale = Buffer.from('old descriptor-compatible debug Host without the fixture guard');
    await writeFile(f.host, stale); f.proof.signed_host_sha256 = sha(stale);
    const build = JSON.parse(await readFile(f.proof.build_record.path)); build.identities['MacOS/nomifun-desktop'].signed_sha256 = sha(stale);
    await writeFile(f.proof.build_record.path, JSON.stringify(build)); f.proof.build_record.sha256 = sha(await readFile(f.proof.build_record.path)); await f.save();
    await assert.rejects(verifyGuardedMacosBuild({ proofPath: f.proofPath, appPath: f.app, repoRoot: f.repo }, { verifySignature: async () => { nativeCalls++; } }), /predates/);
    assert.equal(nativeCalls, 0);
    await writeFile(f.host, 'owned test bytes: MACOS_LIFECYCLE_FIXTURE_REFUSED: fixture root and token must be provided together');
    f.proof.signed_host_sha256 = sha(await readFile(f.host)); build.identities['MacOS/nomifun-desktop'].signed_sha256 = f.proof.signed_host_sha256;
    await writeFile(f.proof.build_record.path, JSON.stringify(build)); f.proof.build_record.sha256 = sha(await readFile(f.proof.build_record.path)); await f.save();
    await assert.rejects(verifyGuardedMacosBuild({ proofPath: f.proofPath, appPath: f.app, repoRoot: f.repo }, { verifySignature: async () => { throw Error('strict signature invalid'); } }), /strict signature/);
  } finally { await rm(f.root, { recursive: true, force: true }); }
});
