import { readFileSync } from 'node:fs';
import { expect, test } from 'bun:test';

test('one Applications entry owns the current hosting route', () => {
  const sider = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');
  const router = readFileSync(new URL('../Router.tsx', import.meta.url), 'utf8');
  expect(sider.match(/<SiderAppEntry\b/g)).toHaveLength(1);
  expect(router.match(/path='\/apps'/g)).toHaveLength(1);
  expect(router).not.toContain("path='/plugins");
  expect(router).not.toMatch(/PluginAuthoring|PluginSurface|PluginRunPage/);
});
