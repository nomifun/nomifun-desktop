import { describe, expect, test } from 'bun:test';
import { browserNarrationFor } from './browserNarration';

describe('browserNarrationFor', () => {
  test('reads the v2 operation envelope', () => {
    expect(browserNarrationFor({ name: 'Browser', args: { operation: 'navigate', url: 'https://example.com/a' } }))
      .toEqual({ key: 'nomi.companion.browser.navigate', params: { host: 'example.com' } });
    expect(browserNarrationFor({ name: 'Browser', args: { operation: 'observe' } })?.key)
      .toBe('nomi.companion.browser.observe');
  });

  test('reads native actions only from the nested v2 action object', () => {
    expect(browserNarrationFor({ name: 'Browser', args: { operation: 'act', action: { action: 'click' } } })?.key)
      .toBe('nomi.companion.browser.click');
    expect(browserNarrationFor({ name: 'Browser', args: { operation: 'act', action: { action: 'type' } } })?.key)
      .toBe('nomi.companion.browser.type');
    expect(browserNarrationFor({ name: 'Browser', args: { action: 'click' } })?.key)
      .toBe('nomi.companion.browser.busy');
  });
});
