import { expect, spyOn, test } from 'bun:test';
import { ipcBridge } from '@/common';
import { isBackendRequestError } from './httpBridge';

// Exercise real bridge deadlines and AbortSignal wiring. No component mocks.
test('Office start/status/stop abort an unresponsive backend with eight-second start and five-second status/stop deadlines', async () => {
  const paths: string[] = [];
  const fetch = spyOn(globalThis, 'fetch').mockImplementation((input, init) => {
    paths.push(String(input));
    return new Promise<Response>((_resolve, reject) => {
      const abort = () => reject(new DOMException('Fixture request aborted', 'AbortError'));
      if (init?.signal?.aborted) abort(); else init?.signal?.addEventListener('abort', abort, { once: true });
    });
  });
  const started = performance.now();
  try {
    const request_id = '01900000-0000-7000-8000-000000000001';
    const elapsed: number[] = [];
    const observe = async (promise: Promise<unknown>, index: number) => { try { return await promise; } finally { elapsed[index] = performance.now() - started; } };
    const results = await Promise.allSettled([
      observe(ipcBridge.officePreview.start.invoke({ request_id, file_path: '/fixture/report.pptx', doc_type: 'ppt' }), 0),
      observe(ipcBridge.officePreview.status.invoke({ request_id }), 1),
      observe(ipcBridge.officePreview.stop.invoke({ request_id }), 2),
    ]);
    expect(paths.some((path) => path.endsWith('/api/office-preview/start'))).toBe(true);
    expect(paths.some((path) => path.endsWith(`/api/office-preview/status/${request_id}`))).toBe(true);
    expect(paths.some((path) => path.endsWith('/api/office-preview/stop'))).toBe(true);
    for (const result of results) {
      expect(result.status).toBe('rejected');
      if (result.status !== 'rejected') continue;
      expect(isBackendRequestError(result.reason)).toBe(true);
      expect(result.reason.kind).toBe('timeout');
    }
    expect(elapsed[0]).toBeGreaterThanOrEqual(7_900);
    expect(elapsed[1]).toBeGreaterThanOrEqual(4_900);
    expect(elapsed[2]).toBeGreaterThanOrEqual(4_900);
    expect(elapsed[1]).toBeLessThan(elapsed[0]);
    expect(elapsed[2]).toBeLessThan(elapsed[0]);
  } finally { fetch.mockRestore(); }
}, 12_000);
