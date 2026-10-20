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

test('JavaScript viewport media queries obey the same desktop minimum', () => {
  const file = 'ui/src/renderer/pages/conversation/execution/ExecutionTopPanel.tsx';
  for (const query of ['(max-width: 768px)', '(min-width: 600px)', '(max-width: 879.5px)']) {
    const failures = inspectRendererSource(file, `window.matchMedia(${JSON.stringify(query)}).matches`);
    assert.equal(failures.length, 1, query);
    assert.match(failures[0].reason, /below the supported 880px minimum/);
  }
  assert.deepEqual(inspectRendererSource(file, "matchMedia('(min-width: 880px)').matches"), []);
  assert.deepEqual(inspectRendererSource(file, "window.matchMedia('(prefers-reduced-motion: reduce)').matches"), []);
});

test('container queries, comments and quoted examples do not become viewport decisions', () => {
  const file = 'ui/src/renderer/pages/conversation/execution/example.tsx';
  assert.deepEqual(inspectRendererSource(file, "const css = '@container (max-width: 600px) { .pane {} }';"), []);
  assert.deepEqual(inspectRendererSource(file, '// window.matchMedia("(max-width: 768px)").matches'), []);
  assert.deepEqual(inspectRendererSource(file, 'const example = `window.matchMedia("(max-width: 768px)")`;'), []);
});
