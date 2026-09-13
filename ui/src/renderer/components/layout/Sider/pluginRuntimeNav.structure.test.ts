import { readFileSync } from 'node:fs';
import { expect, test } from 'bun:test';

test('one plugin entry owns the library, creation and running pages', () => {
  const sider = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');
  const router = readFileSync(new URL('../Router.tsx', import.meta.url), 'utf8');
  expect(sider.match(/<SiderPluginEntry\b/g)).toHaveLength(1);
  expect(sider).not.toContain('SiderPluginRuntimesEntry');
  expect(router.match(/path='\/plugins'/g)).toHaveLength(1);
  expect(router).toContain("path='/plugins/new'");
  expect(router).toContain("path='/plugins/create/:draftId'");
  expect(router).toContain("path='/plugins/run/:id'");
  expect(router).not.toContain('/mini-apps');
});
