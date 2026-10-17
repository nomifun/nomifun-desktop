#!/usr/bin/env node
// Log in only the marked isolated Linux fixture. Credentials and JWT remain
// in its private parent; stdout contains public fixture metadata only.
import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { readNativeAnnouncement, readNativeOwnedFixtureBytes, validateNativeDataset } from './global-os-native.mjs';
import { verifyLinuxNativeApp } from './global-os-linux-cases.mjs';

assert.equal(process.platform, 'linux'); assert(process.getuid() > 0);
assert(process.argv.length === 4 && process.argv[2] === '--native-data-root', 'Usage: node scripts/validation/global-os-linux-fixture-login.mjs --native-data-root /tmp/nomifun-global-os-owner-UNIQUE/native-data');
const dataset = await validateNativeDataset(process.argv[3]);
const announcement = await readNativeAnnouncement(dataset);
assert.equal(announcement.host, '127.0.0.1');
await verifyLinuxNativeApp({ pid: announcement.pid, port: announcement.port, data: dataset.root, uid: process.getuid() });
const credential = JSON.parse((await readNativeOwnedFixtureBytes(join(dataset.parent, 'credential.json'), 4096)).toString('utf8'));
assert.equal(credential.username, 'global-os-fixture'); assert(typeof credential.password === 'string' && credential.password.length >= 32);
const response = await fetch(`http://127.0.0.1:${announcement.port}/login`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(credential), signal: AbortSignal.timeout(15000) });
assert(response.ok, `Real native App login failed (HTTP ${response.status}); response body/credentials are not logged`);
const value = await response.json();
assert(value.success === true && value.user?.username === credential.username && typeof value.token === 'string' && value.token.length > 32, 'Native App did not issue a successful fixture-owner JWT');
const tokenPath = join(dataset.parent, 'owner-jwt');
await writeFile(tokenPath, `${value.token}\n`, { flag: 'wx', mode: 0o600 });
console.log(JSON.stringify({ status: 'prepared', owner_jwt_file: tokenPath, verified_app_pid: announcement.pid, evidence_scope: 'Isolated owner login fixture preparation; not Agent acceptance' }));
