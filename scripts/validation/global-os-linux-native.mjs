#!/usr/bin/env node
// Linux-only public ingress. Historical macOS continuation options stay pinned
// in their original runner and are deliberately unavailable here.
import assert from 'node:assert/strict';
import { readNativeOwnedFixtureBytes, runNativeRegression } from './global-os-native.mjs';

assert.equal(process.platform, 'linux', 'Run on the actual Linux native App host');
const option = key => { const i = process.argv.indexOf(key); return i < 0 ? undefined : process.argv[i + 1]; };
const required = key => { const value = option(key); assert(value && !value.startsWith('--'), `${key} is required`); return value; };
if (process.argv.includes('--help')) {
  console.log('Linux isolated public Agent ingress: --base-url LOOPBACK --native-data-root PRIVATE/native-data --token-file PRIVATE_OWNER_JWT --report-dir EVIDENCE [--scenario files-process|system|missing-default] [--administrator | --administrator-read-matrix | --administrator-system-service] [--retain-service-for-app-exit --app-pid PID]. Native packaging, signing and live-model acceptance are separate evidence.');
} else {
  const values = ['--base-url', '--native-data-root', '--token-file', '--report-dir', '--template', '--scenario', '--app-pid'];
  const flags = ['--administrator', '--administrator-read-matrix', '--administrator-system-service', '--retain-service-for-app-exit'];
  for (let i = 2; i < process.argv.length; i++) {
    const argument = process.argv[i]; assert([...values, ...flags].includes(argument), `Unsupported Linux option ${argument}`);
    assert.equal(process.argv.filter(value => value === argument).length, 1, `Duplicate option ${argument}`);
    if (values.includes(argument)) { assert(process.argv[i + 1] && !process.argv[i + 1].startsWith('--'), `${argument} needs a value`); i++; }
  }
  const retained = process.argv.includes('--retain-service-for-app-exit');
  assert(retained || !process.argv.includes('--app-pid'), '--app-pid requires the retained-service scenario');
  const scenario = option('--scenario') ?? (retained ? 'system' : 'files-process');
  assert(['files-process', 'system', 'missing-default'].includes(scenario), 'Unsupported Linux validation scenario');
  const tokenFile = required('--token-file');
  const token = (await readNativeOwnedFixtureBytes(tokenFile, 4096)).toString('utf8').trim();
  await runNativeRegression({ baseUrl: required('--base-url'), token, tokenFile, nativeDataRoot: required('--native-data-root'), reportDirectory: required('--report-dir'), template: option('--template') ?? 'chat.minimal', scenario, administrator: process.argv.includes('--administrator') || process.argv.includes('--administrator-read-matrix') || process.argv.includes('--administrator-system-service'), administratorSystemService: process.argv.includes('--administrator-system-service'), administratorReadMatrix: process.argv.includes('--administrator-read-matrix'), retainServiceForAppExit: retained, appPid: retained ? Number(required('--app-pid')) : undefined });
}
