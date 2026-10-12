import { afterEach, beforeAll, beforeEach, expect, mock, test } from 'bun:test';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import type { DesktopStartupIO, DesktopStartupSnapshot } from '@/common/adapter/tauriStartup';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import DesktopStartup from './DesktopStartup';

const locale = createInstance();
const fixture = (revision: number, stage: DesktopStartupSnapshot['stage'] = 'starting_backend'): DesktopStartupSnapshot => ({ revision, stage });
const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};
let receive: ((snapshot: DesktopStartupSnapshot) => void) | undefined;
const load = mock(async () => fixture(0));
const quit = mock(async () => {});
const unlisten = mock(() => {});
const subscribe = mock(async (callback: (snapshot: DesktopStartupSnapshot) => void): Promise<() => void> => {
  receive = callback;
  return unlisten;
});
const io: DesktopStartupIO = { snapshot: load, quit, subscribe };
beforeAll(async () => {
  await locale.init({ lng: 'en-US', resources: { 'en-US': { translation: { common } } } });
});
beforeEach(() => {
  receive = undefined;
  load.mockReset().mockImplementation(async () => fixture(0));
  quit.mockReset().mockImplementation(async () => {});
  unlisten.mockReset();
  subscribe.mockReset().mockImplementation(async (callback) => { receive = callback; return unlisten; });
});
afterEach(cleanup);
const mounted = () => render(<I18nextProvider i18n={locale}><DesktopStartup io={io} /></I18nextProvider>);
const event = (next: DesktopStartupSnapshot) => act(() => receive?.(next));
const flush = () => act(async () => {});

test('desktop startup waits for backend readiness without component preparation controls', async () => {
  const view = mounted();
  await flush();
  expect(view.getByText(common.startup.starting)).toBeTruthy();
  expect(view.queryByRole('alert')).toBeNull();
  expect(view.queryByRole('progressbar')).toBeNull();
  expect(view.queryByRole('button', { name: common.retry })).toBeNull();
  expect(view.queryByRole('combobox')).toBeNull();
  expect(view.getByRole('button', { name: common.startup.quit })).toBeTruthy();
});

test('a newer backend readiness event wins over a delayed initial snapshot', async () => {
  const pending = deferred<DesktopStartupSnapshot>();
  load.mockImplementationOnce(() => pending.promise);
  const view = mounted();
  await flush();
  expect(subscribe).toHaveBeenCalledTimes(1);
  expect(load).toHaveBeenCalledTimes(1);
  event(fixture(3, 'ready'));
  expect(view.queryByRole('button', { name: common.startup.quit })).toBeNull();
  await act(async () => pending.resolve(fixture(1)));
  event(fixture(2, 'quitting'));
  expect(view.getByText(common.startup.starting)).toBeTruthy();
  expect(view.queryByRole('button', { name: common.startup.quit })).toBeNull();
});

test('quit during backend startup stays visible while native cleanup runs; old events and late errors cannot revive it', async () => {
  const pending = deferred<void>();
  quit.mockImplementationOnce(() => pending.promise);
  const view = mounted();
  await flush();
  fireEvent.click(view.getByRole('button', { name: common.startup.quit }));
  expect(quit).toHaveBeenCalledTimes(1);
  event(fixture(1, 'quitting'));
  expect(view.getByText(common.startup.quitting)).toBeTruthy();
  expect((view.getByRole('button', { name: common.startup.quit }) as HTMLButtonElement).disabled).toBe(true);
  event(fixture(0, 'ready'));
  expect(view.getByText(common.startup.quitting)).toBeTruthy();
  view.unmount();
  expect(unlisten).toHaveBeenCalledTimes(1);
  await act(async () => pending.reject(new Error('late native close')));
});

test('unmount before subscription finishes disposes the listener and never requests a startup snapshot', async () => {
  const pending = deferred<() => void>();
  subscribe.mockImplementationOnce(() => pending.promise);
  const view = mounted();
  view.unmount();
  await act(async () => pending.resolve(unlisten));
  expect(unlisten).toHaveBeenCalledTimes(1);
  expect(load).not.toHaveBeenCalled();
});

test('a rejected older snapshot does not claim connection failure after live native readiness', async () => {
  const pending = deferred<DesktopStartupSnapshot>();
  load.mockImplementationOnce(() => pending.promise);
  const view = mounted();
  await flush();
  event(fixture(3, 'ready'));
  await act(async () => pending.reject(new Error('late snapshot delivery failed')));
  expect(view.queryByRole('alert')).toBeNull();
  expect(view.getByText(common.startup.starting)).toBeTruthy();
});

test('a failed initial native snapshot shows a connection error and keeps quit available', async () => {
  load.mockImplementationOnce(async () => { throw new Error('native status unavailable'); });
  const view = mounted();
  await flush();
  expect(view.getByRole('alert').textContent).toBe(common.startup.connectionFailed);
  fireEvent.click(view.getByRole('button', { name: common.startup.quit }));
  await flush();
  expect(quit).toHaveBeenCalledTimes(1);
  event(fixture(1));
  expect(view.queryByRole('alert')).toBeNull();
});
