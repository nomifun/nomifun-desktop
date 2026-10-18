/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { expect, mock, test } from 'bun:test';
import { MattingClient } from './mattingClient';
import type { MatteRequest, MatteResponse } from './mattingProtocol';

class TestWorker {
  onmessage: ((event: MessageEvent<MatteResponse>) => void) | null = null;
  onerror: ((event: ErrorEvent) => void) | null = null;
  terminate = mock(() => {});
  requests: MatteRequest[] = [];
  constructor(private respond = true) {}
  postMessage(request: MatteRequest, transfer: Transferable[]) {
    expect(transfer).toEqual([request.bitmap]);
    this.requests.push(request);
    if (this.respond) queueMicrotask(() => this.sendDone(request.id));
  }
  send(message: MatteResponse) { this.onmessage?.({ data: message } as MessageEvent<MatteResponse>); }
  sendDone(id: number) {
    this.send({ id, type: 'done', blob: new Blob(['cutout']), aspect: 1,
      headBox: { x: 0, y: 0, w: 0.3 }, method: 'heuristic' });
  }
}

const file = new File(['image'], 'source.png', { type: 'image/png' });
const bitmap = (): ImageBitmap => ({ width: 12, height: 12, close: mock(() => {}) } as unknown as ImageBitmap);
const tick = () => new Promise<void>(resolve => setTimeout(resolve, 0));
const job = (id: number, mode: 'auto' | 'heuristic' = 'auto') => ({ id, file, mode, tolerance: 11 });

test('passes the acquired Blob explicitly and reuses an idle successful worker', async () => {
  const model = new Blob(['approved model']);
  const worker = new TestWorker();
  const createWorker = mock(() => worker);
  const client = new MattingClient({ createWorker, acquireModel: async () => model, decode: async () => bitmap() });
  try {
    const receive = mock(() => {});
    client.run(job(1), receive);
    await tick();
    expect(worker.requests[0]?.model).toBe(model);
    client.run(job(2), receive);
    await tick();
    expect(createWorker).toHaveBeenCalledTimes(1);
    expect(worker.terminate).not.toHaveBeenCalled();
    expect(worker.requests.map(r => r.id)).toEqual([1, 2]);
  } finally { client.dispose(); }
});

test('model acquisition failure is observable and quick cutout receives no phantom model', async () => {
  const worker = new TestWorker();
  const client = new MattingClient({ createWorker: () => worker, decode: async () => bitmap(),
    acquireModel: async () => { throw new Error('offline'); } });
  const messages: MatteResponse[] = [];
  try {
    client.run(job(1), message => messages.push(message));
    await tick();
    expect(messages).toContainEqual({ id: 1, type: 'fallback', reason: 'model_unavailable', detail: 'Error: offline' });
    expect(worker.requests[0]?.mode).toBe('heuristic');
    expect(worker.requests[0]?.model).toBeUndefined();
    expect(messages.at(-1)?.type).toBe('done');
  } finally { client.dispose(); }
});

test('a stalled inference is terminated before a fresh bitmap and worker run quick cutout', async () => {
  const hung = new TestWorker(false);
  const quick = new TestWorker();
  const workers = [hung, quick];
  const decode = mock(async () => bitmap());
  const client = new MattingClient({ createWorker: () => workers.shift()!, acquireModel: async () => new Blob(['model']),
    decode, timeoutMs: 10 });
  const messages: MatteResponse[] = [];
  try {
    await new Promise<void>((resolve, reject) => {
      const watchdog = setTimeout(() => reject(new Error('quick cutout never completed')), 1000);
      client.run(job(1), message => {
        messages.push(message);
        if (message.type === 'done') { clearTimeout(watchdog); resolve(); }
      });
    });
    expect(hung.terminate).toHaveBeenCalledTimes(1);
    expect(quick.requests[0]?.mode).toBe('heuristic');
    expect(quick.requests[0]?.model).toBeUndefined();
    expect(decode).toHaveBeenCalledTimes(2);
    expect(messages).toContainEqual({ id: 1, type: 'fallback', reason: 'inference_timeout' });
    expect(messages.at(-1)?.type).toBe('done');
  } finally { client.dispose(); }
});

test('closing a preparing job aborts its subscription and suppresses stale messages', async () => {
  let cancellation: AbortSignal | undefined;
  const client = new MattingClient({ acquireModel: (_progress, signal) => {
    cancellation = signal;
    return new Promise<Blob>((_resolve, reject) => signal?.addEventListener('abort', () => reject(new Error('cancelled'))));
  } });
  const receive = mock(() => {});
  client.run(job(1), receive);
  client.dispose();
  await tick();
  expect(cancellation?.aborted).toBe(true);
  expect(receive).toHaveBeenCalledTimes(1); // Initial preparation progress only.
});

test('a replaced decode closes its bitmap and cannot reach the active worker', async () => {
  let finishDecode!: (value: ImageBitmap) => void;
  const stale = bitmap();
  const worker = new TestWorker();
  const decode = mock(() => new Promise<ImageBitmap>(resolve => { finishDecode = resolve; }));
  const client = new MattingClient({ createWorker: () => worker, decode });
  try {
    client.run(job(1, 'heuristic'), () => {});
    const finishStale = finishDecode;
    decode.mockImplementation(async () => bitmap());
    client.run(job(2, 'heuristic'), () => {});
    finishStale(stale);
    await tick();
    expect(stale.close).toHaveBeenCalledTimes(1);
    expect(worker.requests.map(r => r.id)).toEqual([2]);
  } finally { client.dispose(); }
});

test('an inference failure retires the poisoned worker after its fallback finishes', async () => {
  const failed = new TestWorker(false);
  const fresh = new TestWorker();
  const workers = [failed, fresh];
  const client = new MattingClient({ createWorker: () => workers.shift()!, decode: async () => bitmap(),
    acquireModel: async () => new Blob(['model']) });
  try {
    client.run(job(1), () => {});
    await tick();
    failed.send({ id: 1, type: 'fallback', reason: 'inference_failed' });
    failed.sendDone(1);
    expect(failed.terminate).toHaveBeenCalledTimes(1);
    client.run(job(2), () => {});
    await tick();
    expect(fresh.requests).toHaveLength(1);
  } finally { client.dispose(); }
});

test('a terminal callback can immediately start its successor without losing that job', async () => {
  const worker = new TestWorker();
  const client = new MattingClient({ createWorker: () => worker, decode: async () => bitmap() });
  try {
    const completed: number[] = [];
    client.run(job(1, 'heuristic'), message => {
      if (message.type !== 'done') return;
      completed.push(message.id);
      client.run(job(2, 'heuristic'), next => { if (next.type === 'done') completed.push(next.id); });
    });
    await tick();
    expect(completed).toEqual([1, 2]);
    expect(worker.terminate).not.toHaveBeenCalled();
  } finally { client.dispose(); }
});

test('queued messages from a terminated worker cannot finish its quick-cutout replacement', async () => {
  const hung = new TestWorker(false);
  const quick = new TestWorker(false);
  const workers = [hung, quick];
  const messages: MatteResponse[] = [];
  const client = new MattingClient({ createWorker: () => workers.shift()!, decode: async () => bitmap(),
    acquireModel: async () => new Blob(['model']), timeoutMs: 10 });
  try {
    client.run(job(1), message => messages.push(message));
    for (let attempts = 0; quick.requests.length === 0 && attempts < 30; attempts++) await tick();
    expect(quick.requests).toHaveLength(1);
    hung.sendDone(1);
    expect(messages.some(message => message.type === 'done')).toBe(false);
    quick.sendDone(1);
    expect(messages.at(-1)?.type).toBe('done');
  } finally { client.dispose(); }
});
