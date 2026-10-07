import { readFileSync } from 'node:fs';
import { expect, test } from 'bun:test';
import { PLUGIN_FEATURE_VISIBLE } from '../../../utils/plugins/pluginFeatureAvailability';

test('one Plugin entry owns library, creator, and App/detail routes', () => {
  const sider = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');
  const router = readFileSync(new URL('../Router.tsx', import.meta.url), 'utf8');
  const library = readFileSync(new URL('../../../pages/plugins/PluginLibraryPage.tsx', import.meta.url), 'utf8');
  expect(sider.match(/<SiderPluginEntry\b/g)).toHaveLength(1);
  expect(router.match(/path='\/plugins'/g)).toHaveLength(1);
  expect(router).not.toContain("path='/plugins/new'");
  expect(router).not.toContain("path='/plugins/create/:draftId'");
  expect(library).toContain('launchPluginConversation(navigate');
  expect(router).toContain("path='/plugins/run/:id'");
  expect(router).not.toMatch(/PluginRuntime|PluginProduct|PluginMount/);
});

test('hidden plugins keep navigation and direct routes behind the shared gate', () => {
  const sider = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');
  const router = readFileSync(new URL('../Router.tsx', import.meta.url), 'utf8');
  expect(PLUGIN_FEATURE_VISIBLE).toBe(false);
  expect(sider).toMatch(/\{PLUGIN_FEATURE_VISIBLE && <>\s*<SiderPluginEntry\b[^]*?<PluginPinnedEntries\b[^]*?<\/>\}/);
  for (const route of ['/plugins', '/plugins/run/:id']) {
    expect(router).toContain(`path='${route}' element={PLUGIN_FEATURE_VISIBLE ? withRouteFallback(`);
  }
  expect(router.match(/PLUGIN_FEATURE_VISIBLE \? withRouteFallback\(Plugin(?:Library|Run)Page\) : <Navigate to='\/guid' replace/g)).toHaveLength(2);
});
