import { readFileSync } from 'node:fs';
import { expect, test } from 'bun:test';

test('ordinary Chat stays separate from Plugin preview and App surfaces', () => {
  const source = readFileSync(new URL('./GuidPage.tsx', import.meta.url), 'utf8');
  expect(source).toContain('send.sendMessageHandler');
  expect(source).not.toContain('PluginSurfacePanel');
  expect(source).not.toContain('pluginPlatform.surface');
  expect(source).not.toContain('pluginPreview');
  expect(source).not.toContain('new URLSearchParams(location.search)');
});
