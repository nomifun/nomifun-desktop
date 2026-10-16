import { expect, test } from 'bun:test';
import { toolActivityLabel } from './toolActivityLabel';
test('shared activity labels preserve canonical action and target without retired Browser envelopes', () => {
  expect(toolActivityLabel({ name: 'browser_navigate', capability_id: 'browser', action_id: 'browser/navigate', args: { url: 'https://example.com/path' } }, 'zh-CN')).toBe('打开网页 · example.com');
  expect(toolActivityLabel({ name: 'ToolSearch', args: { query: 'browser/navigate' } }, 'en-US')).toBe('Find tools · browser/navigate');
  expect(toolActivityLabel({ update: { title: 'Browser', rawInput: { operation: 'navigate' } } }, 'en-US')).toBeNull();
});
