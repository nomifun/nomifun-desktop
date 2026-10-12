import { afterEach, beforeAll, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { mcpService } from '@/common/adapter/ipcBridge';
import * as agentHooks from '@/renderer/hooks/agent/useAgents';
import * as themeContext from '@/renderer/hooks/context/ThemeContext';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import OneClickImportModal from './OneClickImportModal';

const locale = createInstance();
type Detection = Awaited<ReturnType<typeof mcpService.getAgentMcpConfigs.invoke>>;
const detect = mock(async (): Promise<Detection> => [{ source: 'claude', servers: [] }]);
const restore: Array<() => void> = [];
const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};
const track = <T extends { mockRestore: () => void }>(spy: T): T => { restore.push(() => spy.mockRestore()); return spy; };

beforeAll(async () => {
  await locale.init({ lng: 'en-US', resources: { 'en-US': { translation: { settings, common } } } });
});
beforeEach(() => {
  detect.mockReset().mockImplementation(async () => [{ source: 'claude', servers: [] }]);
  track(spyOn(mcpService.getAgentMcpConfigs, 'invoke').mockImplementation(detect));
  track(spyOn(agentHooks, 'getAgents').mockResolvedValue([]));
  track(spyOn(themeContext, 'useThemeContext').mockReturnValue({
    theme: 'light', colorScheme: 'default', fontScale: 1,
    setTheme: async () => {}, setColorScheme: async () => {}, setFontScale: async () => {},
  }));
  track(spyOn(console, 'error').mockImplementation(() => {}));
});
afterEach(() => { cleanup(); restore.splice(0).reverse().forEach((dispose) => dispose()); });

const fixture = () => {
  const batch = mock(async () => []);
  const cancel = mock(() => {});
  const view = render(<I18nextProvider i18n={locale}><OneClickImportModal visible onCancel={cancel} onBatchImport={batch} /></I18nextProvider>);
  return { ...view, batch, cancel };
};
const next = async (view: ReturnType<typeof fixture>) => {
  await act(async () => { fireEvent.click(view.getByRole('button', { name: settings.mcpNextStep })); });
};
const assertFailure = (view: ReturnType<typeof fixture>, message: string) => {
  expect(view.getByRole('alert').textContent).toBe(message);
  expect(view.getByRole('button', { name: common.retry })).toBeTruthy();
  expect(view.queryByText(settings.mcpNoServersFound)).toBeNull();
  expect(view.queryByText(settings.mcpImportedSuccess)).toBeNull();
  expect((view.getByRole('button', { name: settings.mcpImportButton }) as HTMLButtonElement).disabled).toBe(true);
  expect(view.batch).not.toHaveBeenCalled();
};

test('structured CLI detection failure is an error, and retry can establish a genuinely empty scan', async () => {
  const first = deferred<Detection>();
  const retry = deferred<Detection>();
  detect.mockImplementationOnce(() => first.promise).mockImplementationOnce(() => retry.promise);
  const view = fixture();
  await next(view);
  expect(view.getByText(settings.mcpLoadingTools)).toBeTruthy();
  expect(view.queryByText(settings.mcpNoServersFound)).toBeNull();
  await act(async () => { first.resolve([{ source: 'claude', servers: [], detection_error: 'Configured CLI could not be verified' }]); });
  assertFailure(view, 'Configured CLI could not be verified');
  await act(async () => { fireEvent.click(view.getByRole('button', { name: common.retry })); });
  expect(view.getByText(settings.mcpLoadingTools)).toBeTruthy();
  expect(view.queryByRole('alert')).toBeNull();
  expect(view.queryByText(settings.mcpNoServersFound)).toBeNull();
  await act(async () => { retry.resolve([{ source: 'claude', servers: [] }]); });
  expect(view.getByText(settings.mcpNoServersFound)).toBeTruthy();
  expect(view.queryByRole('alert')).toBeNull();
  expect(view.queryByRole('button', { name: common.retry })).toBeNull();
  expect((view.getByRole('button', { name: settings.mcpImportButton }) as HTMLButtonElement).disabled).toBe(true);
  expect(detect).toHaveBeenCalledTimes(2);
  expect(view.batch).not.toHaveBeenCalled();
});

test('rejected discovery transport is distinct from successful empty discovery', async () => {
  detect.mockRejectedValueOnce(new Error('Discovery connection failed'));
  const view = fixture();
  await next(view);
  assertFailure(view, 'Discovery connection failed');
  await act(async () => { fireEvent.click(view.getByRole('button', { name: common.retry })); });
  expect(view.getByText(settings.mcpNoServersFound)).toBeTruthy();
  expect(view.queryByRole('alert')).toBeNull();
  expect(detect).toHaveBeenCalledTimes(2);
});

test('a missing selected agent is unavailable rather than an empty successful inventory', async () => {
  detect.mockResolvedValueOnce([{ source: 'codex', servers: [] }]);
  const view = fixture();
  await next(view);
  assertFailure(view, settings.mcpAgentUnavailable);
});

test('detection_error wins even if a failed CLI response happens to include servers', async () => {
  detect.mockResolvedValueOnce([{ source: 'claude', detection_error: 'CLI exited with an error', servers: [{
    name: 'unverified-server', importable: true, transport: { type: 'stdio', command: 'fixture-command' },
  }] }]);
  const view = fixture();
  await next(view);
  assertFailure(view, 'CLI exited with an error');
  expect(view.queryByText('unverified-server')).toBeNull();
});

test('reversed scan responses cannot replace the current successful result with an old error', async () => {
  const old = deferred<Detection>();
  const current = deferred<Detection>();
  detect.mockImplementationOnce(() => old.promise).mockImplementationOnce(() => current.promise);
  const view = fixture();
  await next(view);
  await act(async () => { fireEvent.click(view.getByRole('button', { name: settings.mcpPrevStep })); });
  await next(view);
  await act(async () => { current.resolve([{ source: 'claude', servers: [] }]); });
  expect(view.getByText(settings.mcpNoServersFound)).toBeTruthy();
  await act(async () => { old.resolve([{ source: 'claude', servers: [], detection_error: 'Stale discovery error' }]); });
  expect(view.getByText(settings.mcpNoServersFound)).toBeTruthy();
  expect(view.queryByRole('alert')).toBeNull();
  expect(view.queryByRole('button', { name: common.retry })).toBeNull();
  expect(detect).toHaveBeenCalledTimes(2);
});

test('an old scan cannot clear the newer loading state or import stale server data', async () => {
  const old = deferred<Detection>();
  const current = deferred<Detection>();
  detect.mockImplementationOnce(() => old.promise).mockImplementationOnce(() => current.promise);
  const view = fixture();
  await next(view);
  await act(async () => { fireEvent.click(view.getByRole('button', { name: settings.mcpPrevStep })); });
  await next(view);
  await act(async () => { old.resolve([{ source: 'claude', servers: [{ name: 'stale-server', importable: true, transport: { type: 'stdio', command: 'fixture-command' } }] }]); });
  expect(view.getByText(settings.mcpLoadingTools)).toBeTruthy();
  expect(view.queryByText('stale-server')).toBeNull();
  expect(view.queryByText(settings.mcpNoServersFound)).toBeNull();
  expect((view.getByRole('button', { name: settings.mcpImportButton }) as HTMLButtonElement).disabled).toBe(true);
  await act(async () => { current.resolve([{ source: 'claude', servers: [], detection_error: 'Current discovery error' }]); });
  assertFailure(view, 'Current discovery error');
});

test('closing and reopening the modal fences a previous in-flight scan', async () => {
  const old = deferred<Detection>();
  detect.mockImplementationOnce(() => old.promise);
  const view = fixture();
  await next(view);
  const rerenderModal = (visible: boolean) => view.rerender(
    <I18nextProvider i18n={locale}><OneClickImportModal visible={visible} onCancel={view.cancel} onBatchImport={view.batch} /></I18nextProvider>
  );
  await act(async () => { rerenderModal(false); });
  await act(async () => { rerenderModal(true); });
  await next(view);
  expect(view.getByText(settings.mcpNoServersFound)).toBeTruthy();
  await act(async () => { old.reject(new Error('Previous modal discovery failure')); });
  expect(view.getByText(settings.mcpNoServersFound)).toBeTruthy();
  expect(view.queryByRole('alert')).toBeNull();
  expect(view.batch).not.toHaveBeenCalled();
  expect(detect).toHaveBeenCalledTimes(2);
});
