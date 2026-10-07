import { afterEach, expect, spyOn, test } from 'bun:test';
import EventEmitter from 'eventemitter3';
import * as bridge from './bridge';

type Adapter = Parameters<typeof bridge.adapter>[0];
let inbound: Parameters<Adapter['on']>[0];

function install(emit: Adapter['emit']) {
  bridge.adapter({ emit, on: (emitter) => { inbound = emitter; } });
}

afterEach(() => {
  install((name, ...args) => inbound.emit(name, ...args));
});

test('invoke removes its response listener when sending throws', async () => {
  const failure = new Error('transport unavailable');
  let responseEvent = '';
  install((name, body) => {
    responseEvent = name.replace('subscribe-', 'subscribe.callback-') + body.id;
    throw failure;
  });

  await expect(bridge.invoke('audit-failure')).rejects.toBe(failure);
  const emission = spyOn(EventEmitter.prototype, 'emit');
  try {
    inbound.emit(responseEvent, 'late result');
    expect(emission.mock.results.at(-1)?.value).toBe(false);
  } finally {
    emission.mockRestore();
  }
});

test('invoke accepts synchronous falsy responses and consumes each callback once', async () => {
  const seenIds = new Set<string>();
  const emission = spyOn(EventEmitter.prototype, 'emit');
  try {
    for (const value of [false, 0, undefined]) {
      install((name, body) => {
        expect(name).toBe('subscribe-audit-success');
        expect(seenIds.has(body.id)).toBe(false);
        seenIds.add(body.id);
        const responseEvent = 'subscribe.callback-audit-success' + body.id;
        inbound.emit(responseEvent, body.data);
        inbound.emit(responseEvent, 'duplicate');
        expect(emission.mock.results.at(-1)?.value).toBe(false);
      });
      expect(await bridge.invoke('audit-success', value)).toBe(value);
    }
  } finally {
    emission.mockRestore();
  }
});
