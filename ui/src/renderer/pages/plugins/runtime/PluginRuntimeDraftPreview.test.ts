import '../../../../../test/setup-dom.ts';
import { describe, expect, test } from 'bun:test';
import { runInNewContext } from 'node:vm';
import { previewDocument } from './PluginRuntimeDraftPreview';

function previewRuntime(html: string) {
  const events = new Map<string, () => void>();
  const messages: unknown[] = [];
  const window = {
    addEventListener(name: string, callback: () => void) {
      events.set(name, callback);
    },
  } as unknown as Window & {
    nomi: {
      storage: {
        get(key: string): Promise<unknown>;
        set(key: string, value: unknown): Promise<void>;
      };
    };
  };
  const document = previewDocument(html, 'preview-test');
  const scripts = [
    ...document.matchAll(/<script(?:\s[^>]*)?>([\s\S]*?)<\/script>/gi),
  ];
  const context = {
    window,
    parent: {
      postMessage(value: unknown) {
        messages.push(value);
      },
    },
    structuredClone,
    setTimeout(callback: () => void) {
      callback();
    },
    Map,
  };
  for (const script of scripts) runInNewContext(script[1], context);
  events.get('DOMContentLoaded')?.();
  return { storage: window.nomi.storage, messages };
}

describe('PluginRuntime draft preview isolation', () => {
  test('exported production bootstraps cannot replace preview storage or await a real capability', async () => {
    const html =
      '<script data-nomifun-miniapp-bridge="nomifun-miniapp-bridge-bootstrap-v1">throw new Error("production bridge ran")</script><script data-nomifun-product-sdk="1">throw new Error("production SDK ran")</script><!doctype html><html><head></head><body>Imported app</body></html>';
    const first = previewRuntime(html);
    await first.storage.set('tasks', [{ title: 'preview only' }]);
    expect(await first.storage.get('tasks')).toEqual([
      { title: 'preview only' },
    ]);
    const second = previewRuntime(html);
    expect(await second.storage.get('tasks')).toBe(null);
    expect(second.messages).toEqual([
      { type: 'nomifun-plugin-preview', token: 'preview-test', error: null },
    ]);
  });
  test('storage returns copies so app mutation cannot alter stored values without a write', async () => {
    const runtime = previewRuntime(
      '<!doctype html><html><head></head><body>A preview</body></html>',
    );
    const tasks = [{ done: false }];
    await runtime.storage.set('tasks', tasks);
    tasks[0].done = true;
    expect(await runtime.storage.get('tasks')).toEqual([{ done: false }]);
    const read = (await runtime.storage.get('tasks')) as Array<{
      done: boolean;
    }>;
    read[0].done = true;
    expect(await runtime.storage.get('tasks')).toEqual([{ done: false }]);
  });
});
