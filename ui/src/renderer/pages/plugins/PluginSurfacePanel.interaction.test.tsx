import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import type { PluginSurfaceDescriptor } from '@/common/types/pluginPlatform';
import PluginSurfacePanel from './PluginSurfacePanel';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: {} } } });
const browserSettings = (window as typeof window & {
  happyDOM: { settings: { disableIframePageLoading: boolean } };
}).happyDOM.settings;
const originalIframeLoading = browserSettings.disableIframePageLoading;

class SurfacePort {
  onmessage: ((event: MessageEvent) => void) | null = null;
  peer!: SurfacePort;
  closed = false;
  postMessage(data: unknown) {
    queueMicrotask(() => {
      if (!this.closed && !this.peer.closed) this.peer.onmessage?.({ data } as MessageEvent);
    });
  }
  start() {}
  close() { this.closed = true; }
}

const originalMessageChannel = globalThis.MessageChannel;
afterEach(() => {
  cleanup();
  mock.restore();
  globalThis.MessageChannel = originalMessageChannel;
  browserSettings.disableIframePageLoading = originalIframeLoading;
  delete (window as typeof window & { __backendPort?: number }).__backendPort;
});

const descriptor: PluginSurfaceDescriptor = {
  draft_id: 'draft-ready', artifact_digest: 'a'.repeat(64), surface_session_id: 'surface-ready',
  surface_generation: 1, entrypoint: 'ui/index.html', is_preview: true,
};

async function reopenHarness() {
  browserSettings.disableIframePageLoading = true;
  const reportError = window.console.error;
  spyOn(window.console, 'error').mockImplementation((...args: unknown[]) => {
    // This fixture drives load/handshake events itself rather than fetching an iframe page.
    if (String(args[0]).includes('Iframe page loading is disabled')) return;
    reportError(...args);
  });
  (window as typeof window & { __backendPort?: number }).__backendPort = 11451;
  globalThis.MessageChannel = class {
    port1 = new SurfacePort();
    port2 = new SurfacePort();
    constructor() { this.port1.peer = this.port2; this.port2.peer = this.port1; }
  } as unknown as typeof MessageChannel;
  const probes: { operation: string; probe_token: string }[] = [];
  let readyPort: SurfacePort | undefined;
  const target = {
    postMessage(message: { type: string; version: string; nonce: string }, _origin: string, ports?: SurfacePort[]) {
      if (message.type === 'nomifun-plugin-bridge-challenge-v1') {
        queueMicrotask(() => window.dispatchEvent(new MessageEvent('message', {
          source: target as unknown as Window, origin: 'null',
          data: { ...message, type: 'nomifun-plugin-bridge-handshake-v1' },
        })));
      } else if (ports) {
        const connected = ports[0];
        connected.onmessage = event => {
          const probe = event.data as { operation: string; probe_token: string };
          probes.push(probe);
          if (probe.operation === 'ready') readyPort = connected;
          else connected.postMessage({ type: 'nomifun-plugin-ui-observation-v1', probe_token: probe.probe_token, value: 'Loaded' });
        };
      }
    },
  };
  const onComplete = mock(async (_observations: unknown[], _error?: string) => {});
  const props = { descriptor, title: 'Preview', onReload() {}, onClose() {} };
  const view = render(<I18nextProvider i18n={i18n}><PluginSurfacePanel {...props} /></I18nextProvider>);
  const frame = view.getByTitle('Preview') as HTMLIFrameElement;
  Object.defineProperty(frame, 'contentWindow', { configurable: true, get: () => target });
  fireEvent.load(frame);
  await act(async () => { await Promise.resolve(); });
  view.rerender(<I18nextProvider i18n={i18n}><PluginSurfacePanel {...props} verification={{
    command: { test_token: 'reopen-case', draft_id: 'draft-ready', descriptor, case_name: 'persist',
      steps: [{ operation: 'reopen' }, { operation: 'text', selector: '#status', value: 'Loaded' }] },
    onComplete,
  }} /></I18nextProvider>);
  await act(async () => { await Promise.resolve(); });
  return { frame, probes, onComplete, respondReady(value: boolean) {
    const request = probes.find(probe => probe.operation === 'ready');
    if (!request || !readyPort) throw new Error('Readiness probe is missing');
    readyPort.postMessage({ type: 'nomifun-plugin-ui-observation-v1', probe_token: request.probe_token, value });
  } };
}

test('reopen waits for the new SDK connection and ready response before continuing assertions', async () => {
  const state = await reopenHarness();
  expect(state.frame.getAttribute('sandbox')).toBe('allow-scripts');
  expect(state.frame.hasAttribute('srcdoc')).toBe(false);
  expect(state.probes).toEqual([]);
  fireEvent.load(state.frame);
  await waitFor(() => expect(state.probes.map(probe => probe.operation)).toEqual(['ready']));
  expect(state.onComplete).not.toHaveBeenCalled();
  await act(async () => { state.respondReady(true); });
  await waitFor(() => expect(state.onComplete).toHaveBeenCalledWith([null, 'Loaded']));
  expect(state.probes.map(probe => probe.operation)).toEqual(['ready', 'text']);
});

test('reopen reports failed readiness without running the following assertion', async () => {
  const state = await reopenHarness();
  fireEvent.load(state.frame);
  await waitFor(() => expect(state.probes.map(probe => probe.operation)).toEqual(['ready']));
  await act(async () => { state.respondReady(false); });
  await waitFor(() => expect(state.onComplete).toHaveBeenCalledWith([], 'Reopened preview did not become ready'));
  expect(state.probes.map(probe => probe.operation)).toEqual(['ready']);
});
