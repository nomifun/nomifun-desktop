import { test } from 'node:test';
import assert from 'node:assert/strict';
import { inspectRendererSource } from './desktop-ui-boundary-policy.mjs';

const connectionPage = 'ui/src/renderer/pages/connections/index.tsx';

test('desktop connection copy can name the separate phone client', () => {
  assert.deepEqual(inspectRendererSource(connectionPage, "const title = '连接手机'; const help = 'Open NomiFun Mobile on Android or iOS';"), []);
  assert.deepEqual(inspectRendererSource('ui/src/renderer/services/i18n/locales/zh-CN/settings.json', '{"deviceConnections":{"phoneTask":"连接手机"}}'), []);
});

test('device connection locale masking is scoped to the deviceConnections namespace', () => {
  assert.ok(inspectRendererSource('ui/src/renderer/services/i18n/locales/zh-CN/settings.json', '{"webui":{"x":"连接手机"}}').length > 0);
  assert.ok(inspectRendererSource('ui/src/renderer/services/i18n/locales/zh-CN/nomi.json', '{"x":"连接手机"}').length > 0);
});

test('connection-copy permission does not permit device or touch layout selection', () => {
  for (const source of [
    'const isMobile = width < 600;',
    "const phone = navigator.userAgent.includes('Android');",
    'const touch = navigator.maxTouchPoints > 0;',
    "const cls = 'mobile:flex';",
    "const style = 'padding-bottom: env(safe-area-inset-bottom)';",
    "const style = '@media(max-width: 600px) { .panel {display:none} }';",
    "const style = '@media(pointer: coarse) { .button {padding:30px} }';",
  ]) assert.ok(inspectRendererSource(connectionPage, source).length > 0, source);
});

test('other renderer surfaces retain the existing mobile terminology boundary', () => {
  assert.ok(inspectRendererSource('ui/src/renderer/pages/conversation/index.tsx', "const label = '手机布局';").length > 0);
  assert.ok(inspectRendererSource('ui/src/renderer/pages/connections/index.css', '.mobile { display: block; }').length > 0);
  assert.deepEqual(inspectRendererSource(connectionPage, "const style = '@media(max-width: 880px) { .panel {display:block} }';"), []);
});
