#!/usr/bin/env node
// Linux-only public ingress. Historical macOS continuation options stay pinned
// in their original runner and are deliberately unavailable here.
import assert from 'node:assert/strict';
import { readNativeOwnedFixtureBytes, runNativeRegression } from './global-os-native.mjs';

assert.equal(process.platform, 'linux', 'Run on the actual Linux native App host');
const option = key => { const i = process.argv.indexOf(key); return i < 0 ? undefined : process.argv[i + 1]; };
const required = key => { const value = option(key); assert(value && !value.startsWith('--'), `${key} is required`); return value; };
if (process.argv.includes('--help')) {
  console.log('node scripts/validation/global-os-linux-native.mjs --base-url http://127.0.0.1:PORT --native-data-root /tmp/nomifun-global-os-owner-UNIQUE/native-data --token-file /ABS/PRIVATE/owner-jwt --report-dir /ABS/EVIDENCE [--template chat.minimal|assistant.general] [--scenario files-process|system|desktop|missing-default] [--administrator | --administrator-read-matrix | --administrator-system-service] [--retain-service-for-app-exit --app-pid PID] [--x11-session-root /tmp/nomifun-global-os-x11-UUID]. Requires the ordinary-user real native App, its fresh marked dataset and actual owner login JWT. Every effect uses public canonical Agent ingress; host observations are read-only. Desktop defaults to assistant.general and requires the private genuine Xvfb/Openbox launcher metadata, matching real App/driver display and Xauthority. It proves isolated X11 full product ingress rather than real WSLg or Wayland grants. Run systemd service and administrator scenarios independently. The retained-service mode waits for explicit native App exit/restart, then continues the same Session solely for cleanup. No macOS historical Session continuation or arbitrary root paths are accepted.');
} else {
  const values = ['--base-url', '--native-data-root', '--token-file', '--report-dir', '--template', '--scenario', '--app-pid', '--x11-session-root'];
  const flags = ['--administrator', '--administrator-read-matrix', '--administrator-system-service', '--retain-service-for-app-exit'];
  for (let i = 2; i < process.argv.length; i++) {
    const argument = process.argv[i]; assert([...values, ...flags].includes(argument), `Unsupported Linux option ${argument}`);
    assert.equal(process.argv.filter(value => value === argument).length, 1, `Duplicate option ${argument}`);
    if (values.includes(argument)) { assert(process.argv[i + 1] && !process.argv[i + 1].startsWith('--'), `${argument} needs a value`); i++; }
  }
  const retained = process.argv.includes('--retain-service-for-app-exit');
  assert(retained || !process.argv.includes('--app-pid'), '--app-pid requires the retained-service scenario');
  const scenario = option('--scenario') ?? (retained ? 'system' : 'files-process');
  assert(scenario !== 'desktop' || option('--x11-session-root'), 'Desktop acceptance requires its private --x11-session-root');
  assert(scenario !== 'desktop' || !option('--template') || option('--template') === 'assistant.general', 'Linux desktop acceptance uses the unmodified default General Agent');
  assert(scenario === 'desktop' || !option('--x11-session-root'), '--x11-session-root requires --scenario desktop');
  const tokenFile = required('--token-file');
  const token = (await readNativeOwnedFixtureBytes(tokenFile, 4096)).toString('utf8').trim();
  await runNativeRegression({ baseUrl: required('--base-url'), token, tokenFile, nativeDataRoot: required('--native-data-root'), reportDirectory: required('--report-dir'), template: option('--template') ?? (scenario === 'desktop' ? 'assistant.general' : 'chat.minimal'), scenario, administrator: process.argv.includes('--administrator') || process.argv.includes('--administrator-read-matrix') || process.argv.includes('--administrator-system-service'), administratorSystemService: process.argv.includes('--administrator-system-service'), administratorReadMatrix: process.argv.includes('--administrator-read-matrix'), retainServiceForAppExit: retained, appPid: retained ? Number(required('--app-pid')) : undefined, x11SessionRoot: option('--x11-session-root') });
}
