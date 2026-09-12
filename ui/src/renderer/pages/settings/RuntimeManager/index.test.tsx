import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { Modal } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import { javascriptRuntime } from '@/common/adapter/javascriptRuntimeBridge';
import type { JavaScriptRuntimeStatus } from '@/common/types/javascriptRuntime';
import * as platform from '@/renderer/utils/platform';
import RuntimeManager from './index';

const i18n = createInstance();
await i18n.init({ lng: 'en', keySeparator: false, resources: { en: { translation: {} } } });
const key = (name: string) => 'settings.runtimeManager.' + (name === 'use' ? 'candidate.use' : 'actions.' + name);
const restore: Array<() => void> = [];
afterEach(() => { cleanup(); restore.splice(0).reverse().forEach(fn => fn()); });
function deferred<T>() {
  const pending: Array<{ resolve: (value: T) => void; reject: (error: unknown) => void }> = [];
  return { pending, invoke: () => new Promise<T>((resolve, reject) => pending.push({ resolve, reject })) };
}
const runtime = { runtime_installation_id: 'managed-node', node_version: '24.8.0', runtime_target: 'test-target', executable_digest: 'a'.repeat(64) };
const candidate = { source: 'managed' as const, executable_path: '/managed/node', compatibility: 'recommended' as const, runtime };
const snapshot = (overrides: Partial<JavaScriptRuntimeStatus> = {}): JavaScriptRuntimeStatus => ({
  selection_revision: 3, probes: [], switch_participants: [], requires_switch_decision: false,
  non_recommended_warning_acknowledged: [], download: { download_revision: 0, state: 'not_installed' },
  download_offer: { offer_digest: 'b'.repeat(64), node_version: runtime.node_version, runtime_target: runtime.runtime_target, archive_file_name: 'node.zip' },
  ...overrides,
});
const downloading = () => snapshot({ download: { download_revision: 1, state: 'downloading' } });
const ready = (overrides: Partial<JavaScriptRuntimeStatus> = {}) => snapshot({ probes: [candidate], download: { download_revision: 2, state: 'ready', runtime }, ...overrides });

async function fixture(initial = snapshot()) {
  const reads = deferred<JavaScriptRuntimeStatus>(); const probes = deferred<JavaScriptRuntimeStatus>();
  const downloads = deferred<JavaScriptRuntimeStatus>(); const switches = deferred<JavaScriptRuntimeStatus>();
  const decisions = deferred<JavaScriptRuntimeStatus>();
  const paths = deferred<string[]>();
  const status = spyOn(javascriptRuntime.status, 'invoke').mockImplementation(reads.invoke);
  const probe = spyOn(javascriptRuntime.probe, 'invoke').mockImplementation(probes.invoke);
  const download = spyOn(javascriptRuntime.download, 'invoke').mockImplementation(downloads.invoke);
  const beginSwitch = spyOn(javascriptRuntime.beginSwitch, 'invoke').mockImplementation(switches.invoke);
  const decideSwitch = spyOn(javascriptRuntime.decideSwitch, 'invoke').mockImplementation(decisions.invoke);
  const picker = spyOn(ipcBridge.dialog.showOpen, 'invoke').mockImplementation(paths.invoke);
  const desktop = spyOn(platform, 'isDesktopShell').mockReturnValue(true);
  const error = spyOn(console, 'error').mockImplementation(() => {});
  const confirmations: Array<Parameters<typeof Modal.confirm>[0]> = [];
  const close = mock();
  const confirm = spyOn(Modal, 'confirm').mockImplementation(props => { confirmations.push(props); return { close, update: mock() }; });
  // Only this component's 1.5s/3s polling timers are controlled; other DOM timers remain real.
  const timers = new Map<number, () => void>(); let timerId = -1;
  const timerWindow: Window = window;
  const nativeSet = timerWindow.setTimeout.bind(timerWindow); const nativeClear = timerWindow.clearTimeout.bind(timerWindow);
  const timer = spyOn(timerWindow, 'setTimeout').mockImplementation((handler, delay, ...args) => {
    if (delay !== 1500 && delay !== 3000) return nativeSet(handler, delay, ...args);
    const id = timerId--; timers.set(id, () => { if (typeof handler === 'function') handler(...args); }); return id;
  });
  const clear = spyOn(timerWindow, 'clearTimeout').mockImplementation(id => { if (id !== undefined && timers.delete(id)) return; nativeClear(id); });
  for (const spy of [status, probe, download, beginSwitch, decideSwitch, picker, desktop, error, confirm, timer, clear]) restore.push(() => spy.mockRestore());
  const view = render(<I18nextProvider i18n={i18n}><RuntimeManager /></I18nextProvider>);
  await act(async () => { reads.pending[0]!.resolve(initial); });
  return {
    view, status, probe, download, beginSwitch, decideSwitch, picker, error, confirmations, close, timers,
    decisions: decisions.pending,
    reads: reads.pending, probes: probes.pending, downloads: downloads.pending, switches: switches.pending, paths: paths.pending,
    click: (name: string) => fireEvent.click(view.getByRole('button', { name: key(name) })),
    tick: () => { const entry = timers.entries().next().value; expect(entry).toBeDefined(); timers.delete(entry![0]); act(() => entry![1]()); },
  };
}

test('scan is single-flight even before the disabled state commits', async () => {
  const f = await fixture();
  act(() => { f.click('scan'); f.click('scan'); });
  expect(f.probe).toHaveBeenCalledTimes(1);
  await act(async () => { f.probes[0]!.resolve(snapshot({ selection_revision: 4 })); });
  expect(f.view.getByText('selection_revision: 4')).not.toBeNull();
});

test.each([false, true])('a file picker cannot publish after unmount (failure=%s)', async failure => {
  const f = await fixture(); f.click('choosePath'); f.view.unmount();
  await act(async () => { if (failure) f.paths[0]!.reject('picker closed'); else f.paths[0]!.resolve(['/chosen/node']); });
  expect(f.probe).not.toHaveBeenCalled(); expect(f.error).not.toHaveBeenCalled();
});

test.each(['download', 'use'] as const)('a retained %s confirmation is inert after unmount', async action => {
  const f = await fixture(action === 'use' ? snapshot({ probes: [{ ...candidate, compatibility: 'compatible' }] }) : snapshot());
  f.click(action); expect(f.confirmations.length).toBe(1); f.view.unmount();
  act(() => { void f.confirmations[0]!.onOk?.(); });
  expect(action === 'download' ? f.download : f.beginSwitch).not.toHaveBeenCalled();
  expect(f.close).toHaveBeenCalledTimes(1);
});

test('a poll overtaken by a probe cannot replace its snapshot or start a switch', async () => {
  const f = await fixture(downloading()); f.tick(); f.click('scan');
  await act(async () => { f.probes[0]!.resolve({ ...downloading(), selection_revision: 4 }); });
  await act(async () => { f.reads[1]!.resolve(ready()); });
  expect(f.beginSwitch).not.toHaveBeenCalled();
  expect(f.view.getByText('selection_revision: 4')).not.toBeNull();
});

test.each(['selected', 'pending_candidate'] as const)('a completed download cannot re-switch a %s runtime', async field => {
  const f = await fixture(downloading()); f.tick();
  await act(async () => { f.reads[1]!.resolve(ready({ [field]: runtime })); });
  expect(f.beginSwitch).not.toHaveBeenCalled();
});

test('confirmed download polls, switches with exact identities, and cleans up', async () => {
  const f = await fixture(); f.click('download');
  act(() => { void f.confirmations[0]!.onOk?.(); });
  expect(f.download).toHaveBeenCalledWith({ expected_selection_revision: 3, expected_offer_digest: 'b'.repeat(64) });
  await act(async () => { f.downloads[0]!.resolve(downloading()); }); f.tick();
  await act(async () => { f.reads[1]!.resolve(ready()); });
  expect(f.beginSwitch).toHaveBeenCalledWith({ expected_selection_revision: 3, candidate_runtime_id: runtime.runtime_installation_id, expected_candidate_executable_digest: runtime.executable_digest, acknowledge_non_recommended_runtime: false });
  await act(async () => { f.switches[0]!.resolve(ready({ selected: runtime, selection_revision: 4 })); });
  expect(f.view.getByText('selection_revision: 4')).not.toBeNull();
  f.view.unmount(); expect(f.timers.size).toBe(0);
});

test('a failed decision releases the action and preserves the pending identity for retry', async () => {
  const f = await fixture(ready({ pending_candidate: runtime, requires_switch_decision: true }));
  act(() => { f.click('restore'); f.click('restore'); });
  expect(f.decideSwitch).toHaveBeenCalledTimes(1);
  await act(async () => { f.decisions[0]!.reject(new Error('decision offline')); });
  expect(f.view.getByText('decision offline')).not.toBeNull();
  f.click('continue');
  expect(f.decideSwitch).toHaveBeenLastCalledWith({ expected_selection_revision: 3, candidate_runtime_id: runtime.runtime_installation_id, expected_candidate_executable_digest: runtime.executable_digest, decision: 'commit_candidate' });
  await act(async () => { f.decisions[1]!.resolve(ready({ selected: runtime, selection_revision: 4 })); });
  expect(f.view.queryByText('decision offline')).toBeNull();
  expect(f.view.getByText('selection_revision: 4')).not.toBeNull();
});
