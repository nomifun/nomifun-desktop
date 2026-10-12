import { afterEach, beforeEach, describe, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { ipcBridge } from '@/common';
import { BackendRequestError } from '@/common/adapter/httpBridge';
import type { OfficePreviewRequest, OfficePreviewStatus } from '@/common/types/office/preview';
import OfficeArtifactViewer, { isOfficeArtifactReadyMessage, resolveOfficeArtifactUrl, scheduleOfficeFrameDeadline } from './OfficeArtifactViewer';

const CAPABILITY = '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef';
const OTHER_CAPABILITY = 'abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789';
const artifactPath = (capability = CAPABILITY) => `/api/office-preview-artifacts/${capability}/`;
const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};
const ready = (request_id: string, capability = CAPABILITY): OfficePreviewStatus => ({ request_id, state: 'ready', capability, url: artifactPath(capability) });
const start = mock(async (request: OfficePreviewRequest): Promise<OfficePreviewStatus> => ready(request.request_id));
const status = mock(async ({ request_id }: { request_id: string }): Promise<OfficePreviewStatus> => ready(request_id));
const stop = mock(async (_: { request_id: string }) => {});
let restore: Array<() => void> = [];
const flush = () => act(async () => {});
const settlePoll = () => act(async () => { await new Promise((resolve) => setTimeout(resolve, 550)); });
const frameData = (iframe: HTMLIFrameElement) => {
  const url = new URL(iframe.getAttribute('src')!, window.location.href);
  return { type: 'nomifun-office-preview-ready', request_id: url.searchParams.get('request_id'), nonce: url.searchParams.get('nonce'), capability: url.pathname.split('/')[3] };
};
const paint = (iframe: HTMLIFrameElement, overrides: Record<string, unknown> = {}, source = iframe.contentWindow, origin = 'null') => {
  act(() => { window.dispatchEvent(new MessageEvent('message', { source, origin, data: { ...frameData(iframe), ...overrides } })); });
};
const allFrames = (container: HTMLElement) => Array.from(container.querySelectorAll('iframe'));

const originalIframeSrc = Object.getOwnPropertyDescriptor(HTMLIFrameElement.prototype, 'src')!;
beforeEach(() => {
  // Happy DOM exercises the real iframe lifecycle on about:blank. The requested
  // URL stays in its attribute for assertions, without making network requests.
  Object.defineProperty(HTMLIFrameElement.prototype, 'src', { ...originalIframeSrc, get: () => 'about:blank' });
  start.mockReset().mockImplementation(async (request) => ready(request.request_id));
  status.mockReset().mockImplementation(async ({ request_id }) => ready(request_id));
  stop.mockReset().mockImplementation(async () => {});
  restore = [
    spyOn(ipcBridge.officePreview.start, 'invoke').mockImplementation(start),
    spyOn(ipcBridge.officePreview.status, 'invoke').mockImplementation(status),
    spyOn(ipcBridge.officePreview.stop, 'invoke').mockImplementation(stop),
  ].map((spy) => () => spy.mockRestore());
});
afterEach(() => { cleanup(); restore.forEach((run) => run()); Object.defineProperty(HTMLIFrameElement.prototype, 'src', originalIframeSrc); });

describe('Office artifact trust boundary', () => {
  test('only a canonical capability root is resolved on desktop and WebUI', () => {
    expect(resolveOfficeArtifactUrl(artifactPath(), CAPABILITY, 'http://127.0.0.1:17420/')).toBe(`http://127.0.0.1:17420${artifactPath()}`);
    expect(resolveOfficeArtifactUrl(artifactPath(), CAPABILITY, '')).toBe(artifactPath());
    for (const url of ['http://127.0.0.1:1234/', `//evil.invalid${artifactPath()}`, artifactPath(OTHER_CAPABILITY), artifactPath().slice(0, -1), `${artifactPath()}?nonce=anything`, `${artifactPath()}%2e%2e/x`, `${artifactPath()}assets/index.js`]) {
      expect(() => resolveOfficeArtifactUrl(url, CAPABILITY, '')).toThrow('Invalid Office preview capability URL');
    }
    expect(() => resolveOfficeArtifactUrl(artifactPath(CAPABILITY.toUpperCase()), CAPABILITY.toUpperCase(), '')).toThrow();
  });
  test('source, opaque origin, request, capability and nonce are all required', () => {
    const frame = { requestId: 'request', capability: CAPABILITY, nonce: 'nonce' };
    const data = { type: 'nomifun-office-preview-ready', request_id: frame.requestId, capability: frame.capability, nonce: frame.nonce };
    const event = (override: Record<string, unknown> = {}, source: Window | null = window, origin = 'null') => new MessageEvent('message', { source, origin, data: { ...data, ...override } });
    expect(isOfficeArtifactReadyMessage(event(), window, frame)).toBe(true);
    for (const override of [{ request_id: 'other' }, { capability: OTHER_CAPABILITY }, { nonce: 'other' }, { type: 'other' }]) expect(isOfficeArtifactReadyMessage(event(override), window, frame)).toBe(false);
    expect(isOfficeArtifactReadyMessage(event({}, null), window, frame)).toBe(false);
    expect(isOfficeArtifactReadyMessage(event({}, window, window.location.origin), window, frame)).toBe(false);
  });
});

test('ready HTTP and iframe load alone do not claim first-screen success', async () => {
  const view = render(<OfficeArtifactViewer docType='ppt' file_path='/workspace/report.pptx' workspace='/workspace' />);
  await flush();
  const iframe = allFrames(view.container)[0]!;
  expect(iframe.getAttribute('sandbox')).toBe('allow-scripts');
  expect(iframe.getAttribute('referrerpolicy')).toBe('no-referrer');
  fireEvent.load(iframe);
  expect(view.container.firstElementChild?.getAttribute('aria-busy')).toBe('true');
  paint(iframe, { nonce: 'forged' });
  paint(iframe, {}, window);
  paint(iframe, {}, iframe.contentWindow, window.location.origin);
  expect(iframe.getAttribute('aria-hidden')).toBe('true');
  expect(iframe.hasAttribute('inert')).toBe(true);
  paint(iframe);
  expect(iframe.getAttribute('aria-hidden')).toBe('false');
  expect(iframe.hasAttribute('inert')).toBe(false);
  expect(view.container.firstElementChild?.getAttribute('aria-busy')).toBe('false');
  view.unmount();
  expect(stop).toHaveBeenCalledWith({ request_id: start.mock.calls[0]![0].request_id });
});

test('unmount cancels before start acknowledges, and releases a late response again', async () => {
  const pending = deferred<OfficePreviewStatus>();
  start.mockImplementationOnce(() => pending.promise);
  const view = render(<OfficeArtifactViewer docType='word' file_path='/workspace/report.docx' />);
  const request = start.mock.calls[0]![0];
  view.unmount();
  expect(stop).toHaveBeenCalledWith({ request_id: request.request_id });
  await act(async () => { pending.resolve(ready(request.request_id)); });
  expect(stop.mock.calls.filter(([item]) => item.request_id === request.request_id)).toHaveLength(2);
  expect(status).not.toHaveBeenCalled();
});

test('polls only its pending request and stops polling when an artifact arrives', async () => {
  start.mockImplementationOnce(async (request) => ({ request_id: request.request_id, state: 'queued' }));
  status.mockImplementationOnce(async ({ request_id }) => ({ request_id, state: 'rendering' }));
  const view = render(<OfficeArtifactViewer docType='excel' file_path='/workspace/report.xlsx' />);
  await flush();
  await settlePoll();
  await settlePoll();
  expect(status.mock.calls).toEqual([[{ request_id: start.mock.calls[0]![0].request_id }], [{ request_id: start.mock.calls[0]![0].request_id }]]);
  paint(allFrames(view.container)[0]!);
  await settlePoll();
  expect(status).toHaveBeenCalledTimes(2);
});

test('a prior painted version stays mounted during refresh and is released only after replacement paints', async () => {
  const view = render(<OfficeArtifactViewer docType='ppt' file_path='/workspace/report.pptx' revision={0} />);
  await flush();
  const previous = allFrames(view.container)[0]!;
  paint(previous);
  const pending = deferred<OfficePreviewStatus>();
  start.mockImplementationOnce(() => pending.promise);
  view.rerender(<OfficeArtifactViewer docType='ppt' file_path='/workspace/report.pptx' revision={1} />);
  expect(allFrames(view.container)).toEqual([previous]);
  expect(stop).not.toHaveBeenCalled();
  const request = start.mock.calls[1]![0];
  await act(async () => { pending.resolve(ready(request.request_id, OTHER_CAPABILITY)); });
  const replacement = allFrames(view.container)[1]!;
  expect(previous.getAttribute('aria-hidden')).toBe('false');
  expect(stop).not.toHaveBeenCalled();
  paint(replacement);
  expect(allFrames(view.container)).toEqual([replacement]);
  expect(stop).toHaveBeenCalledWith({ request_id: start.mock.calls[0]![0].request_id });
});

test('a stale start reply cannot replace a newer file or schedule polling', async () => {
  const pending = deferred<OfficePreviewStatus>();
  start.mockImplementationOnce(() => pending.promise);
  const view = render(<OfficeArtifactViewer docType='word' file_path='/workspace/old.docx' />);
  const old = start.mock.calls[0]![0];
  view.rerender(<OfficeArtifactViewer docType='word' file_path='/workspace/new.docx' />);
  await flush();
  const current = allFrames(view.container)[0]!;
  await act(async () => { pending.resolve({ request_id: old.request_id, state: 'queued' }); });
  expect(allFrames(view.container)).toEqual([current]);
  await settlePoll();
  expect(status).not.toHaveBeenCalled();
  expect(stop).toHaveBeenCalledWith({ request_id: old.request_id });
});

test('a stale pending status cannot replace a refresh or create another poll', async () => {
  const pending = deferred<OfficePreviewStatus>();
  start.mockImplementationOnce(async (request) => ({ request_id: request.request_id, state: 'rendering' }));
  status.mockImplementationOnce(() => pending.promise);
  const view = render(<OfficeArtifactViewer docType='ppt' file_path='/workspace/report.pptx' revision={0} />);
  await settlePoll();
  const old = start.mock.calls[0]![0];
  view.rerender(<OfficeArtifactViewer docType='ppt' file_path='/workspace/report.pptx' revision={1} />);
  await flush();
  const current = allFrames(view.container)[0]!;
  await act(async () => { pending.resolve(ready(old.request_id, OTHER_CAPABILITY)); });
  expect(allFrames(view.container)).toEqual([current]);
  await settlePoll();
  expect(status).toHaveBeenCalledTimes(1);
});

test('permanent failures expose no retry; transient failures retry under a new request identity', async () => {
  start.mockImplementationOnce(async (request) => ({ request_id: request.request_id, state: 'error', error: { code: 'ENGINE_UNAVAILABLE', message: 'Bad component', retryable: false } }));
  const view = render(<OfficeArtifactViewer docType='ppt' file_path='/workspace/report.pptx' revision={0} />);
  await flush();
  expect(view.queryByRole('button')).toBeNull();
  start.mockImplementationOnce(async (request) => ({ request_id: request.request_id, state: 'error', error: { code: 'FILE_CHANGED', message: 'Still writing', retryable: true } }));
  view.rerender(<OfficeArtifactViewer docType='ppt' file_path='/workspace/report.pptx' revision={1} />);
  await flush();
  fireEvent.click(view.getByRole('button'));
  await flush();
  expect(start).toHaveBeenCalledTimes(3);
  expect(start.mock.calls[2]![0].request_id).not.toBe(start.mock.calls[1]![0].request_id);
  expect(allFrames(view.container)).toHaveLength(1);
});

test('invalid responses and untrusted artifact URLs fail closed and release the lease', async () => {
  start.mockImplementationOnce(async () => ({ ...ready('wrong-request'), url: artifactPath() }));
  const view = render(<OfficeArtifactViewer docType='ppt' file_path='/workspace/report.pptx' revision={0} />);
  await flush();
  expect(allFrames(view.container)).toHaveLength(0);
  expect(view.queryByRole('button')).toBeNull();
  expect(stop).toHaveBeenCalledWith({ request_id: start.mock.calls[0]![0].request_id });
  start.mockImplementationOnce(async (request) => ({ ...ready(request.request_id), url: 'https://untrusted.invalid/' }));
  view.rerender(<OfficeArtifactViewer docType='ppt' file_path='/workspace/report.pptx' revision={1} />);
  await flush();
  expect(allFrames(view.container)).toHaveLength(0);
  expect(stop).toHaveBeenCalledWith({ request_id: start.mock.calls[1]![0].request_id });
});

test('a failed update keeps the prior preview and the same iframe DOM', async () => {
  const view = render(<OfficeArtifactViewer docType='excel' file_path='/workspace/report.xlsx' revision={0} />);
  await flush();
  const previous = allFrames(view.container)[0]!;
  paint(previous);
  start.mockImplementationOnce(async (request) => ({ request_id: request.request_id, state: 'error', error: { code: 'RENDER_FAILED', message: 'Render failed', retryable: false } }));
  view.rerender(<OfficeArtifactViewer docType='excel' file_path='/workspace/report.xlsx' revision={1} />);
  await flush();
  expect(allFrames(view.container)).toEqual([previous]);
  expect(previous.getAttribute('aria-hidden')).toBe('false');
  expect(view.getByRole('alert')).toBeTruthy();
  expect(stop.mock.calls.some(([item]) => item.request_id === start.mock.calls[0]![0].request_id)).toBe(false);
});


test('file updates restore authenticated scroll position before showing the new frame', async () => {
  const view = render(<OfficeArtifactViewer docType='ppt' file_path='/workspace/report.pptx' revision={0} />);
  await flush();
  const previous = allFrames(view.container)[0]!;
  paint(previous);
  paint(previous, { type: 'nomifun-office-preview-scroll', x: 10, y: 250 });
  // Malformed and forged messages cannot change the retained position.
  paint(previous, { type: 'nomifun-office-preview-scroll', nonce: 'forged', x: 10, y: 999 });
  paint(previous, { type: 'nomifun-office-preview-scroll', x: 10, y: Number.NaN });
  view.rerender(<OfficeArtifactViewer docType='ppt' file_path='/workspace/report.pptx' revision={1} />);
  await flush();
  const replacement = allFrames(view.container)[1]!;
  const post = spyOn(replacement.contentWindow!, 'postMessage').mockImplementation(() => {});
  try {
    paint(replacement);
    expect(post).toHaveBeenCalledWith({ ...frameData(replacement), type: 'nomifun-office-preview-restore-scroll', x: 10, y: 250 }, '*');
  } finally { post.mockRestore(); }
});


test('Excel refresh restores a validated active sheet as well as pane position', async () => {
  const view = render(<OfficeArtifactViewer docType='excel' file_path='/workspace/report.xlsx' revision={0} />);
  await flush();
  const previous = allFrames(view.container)[0]!;
  paint(previous);
  paint(previous, { type: 'nomifun-office-preview-scroll', x: 15, y: 300, sheet: 2 });
  for (const sheet of [-1, 4097, 1.5, '2', Number.POSITIVE_INFINITY]) {
    paint(previous, { type: 'nomifun-office-preview-scroll', x: 99, y: 999, sheet });
  }
  view.rerender(<OfficeArtifactViewer docType='excel' file_path='/workspace/report.xlsx' revision={1} />);
  await flush();
  const replacement = allFrames(view.container)[1]!;
  const post = spyOn(replacement.contentWindow!, 'postMessage').mockImplementation(() => {});
  try {
    paint(replacement);
    expect(post).toHaveBeenCalledWith({ ...frameData(replacement), type: 'nomifun-office-preview-restore-scroll', x: 15, y: 300, sheet: 2 }, '*');
  } finally { post.mockRestore(); }
});


test('first-paint deadline pauses while the document is hidden and resumes with its remaining visible budget', async () => {
  const original = Object.getOwnPropertyDescriptor(document, 'visibilityState');
  let visibility: DocumentVisibilityState = 'hidden';
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => visibility });
  const expired = mock(() => {});
  const cancel = scheduleOfficeFrameDeadline(expired, 200);
  const tick = (milliseconds: number) => new Promise((resolve) => setTimeout(resolve, milliseconds));
  try {
    await tick(100);
    expect(expired).not.toHaveBeenCalled();
    visibility = 'visible'; document.dispatchEvent(new Event('visibilitychange'));
    await tick(10);
    visibility = 'hidden'; document.dispatchEvent(new Event('visibilitychange'));
    await tick(100);
    expect(expired).not.toHaveBeenCalled();
    visibility = 'visible'; document.dispatchEvent(new Event('visibilitychange'));
    await tick(20);
    expect(expired).not.toHaveBeenCalled();
    await tick(220);
    expect(expired).toHaveBeenCalledTimes(1);
  } finally {
    cancel();
    if (original) Object.defineProperty(document, 'visibilityState', original);
    else Reflect.deleteProperty(document, 'visibilityState');
  }
});

test('cancelling a hidden first-paint deadline prevents expiry on return', async () => {
  const original = Object.getOwnPropertyDescriptor(document, 'visibilityState');
  let visibility: DocumentVisibilityState = 'hidden';
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => visibility });
  const expired = mock(() => {});
  const cancel = scheduleOfficeFrameDeadline(expired, 10);
  try {
    cancel();
    visibility = 'visible'; document.dispatchEvent(new Event('visibilitychange'));
    await new Promise((resolve) => setTimeout(resolve, 25));
    expect(expired).not.toHaveBeenCalled();
  } finally {
    if (original) Object.defineProperty(document, 'visibilityState', original);
    else Reflect.deleteProperty(document, 'visibilityState');
  }
});


test('a timed-out start releases its request and exposes a transient retry using a new identity', async () => {
  start.mockRejectedValueOnce(new BackendRequestError('timeout', 'Backend request timed out'));
  const view = render(<OfficeArtifactViewer docType='ppt' file_path='/workspace/report.pptx' />);
  await flush();
  const timedOut = start.mock.calls[0]![0].request_id;
  expect(stop).toHaveBeenCalledWith({ request_id: timedOut });
  expect(allFrames(view.container)).toHaveLength(0);
  fireEvent.click(view.getByRole('button'));
  await flush();
  expect(start.mock.calls[1]![0].request_id).not.toBe(timedOut);
  expect(allFrames(view.container)).toHaveLength(1);
});
