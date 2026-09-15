import '../../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { useEffect } from 'react';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import { pluginRuntimeProduct, type PluginRuntimeDraft } from '@/common/adapter/pluginRuntimeProductBridge';
import type { PluginRuntimeWorkshop } from '@/common/types/pluginRuntimePlatform';
import * as modelSelection from '../../guid/hooks/useGuidModelSelection';
import * as preview from './PluginRuntimeDraftPreview';
import PluginRuntimeCreatorPage from './PluginRuntimeCreatorPage';
import en from '@/renderer/services/i18n/locales/en-US/pluginRuntime.json';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { pluginRuntime: en } } }, interpolation: { escapeValue: false } });
const draft: PluginRuntimeDraft = { id: 'draft-one', revision: 4, name: 'Business check', description: '', html: '',
  service_source: 'export const capabilityService = {};', messages: [], status: 'ready', error: null,
  plugin_id: null, base_release_digest: null, updated_at: 1, import: null };
const receipt = { draft_id: draft.id, expected_revision: 6, release_digest: 'a'.repeat(64), receipt_id: 'receipt-one', display_name: draft.name };
const required = (details = receipt) => new BackendHttpError({ method: 'POST', path: '/save', status: 409,
  body: { code: 'PLUGIN_SERVICE_TEST_INPUT_REQUIRED', error: 'Business input required', details } });
const success = { plugin: { plugin_id: 'published-check' } } as PluginRuntimeWorkshop;
afterEach(() => { cleanup(); mock.restore(); });

async function mount(value = draft) {
  spyOn(modelSelection, 'useGuidModelSelection').mockReturnValue({ current_model: undefined, modelList: [],
    setCurrentModel: async () => {}, formatGeminiModelLabel: (_provider, name) => name ?? '' });
  spyOn(preview, 'default').mockImplementation(({ onStatus }) => {
    useEffect(() => { let active = true; void Promise.resolve().then(() => { if (active) onStatus?.(null); });
      return () => { active = false; }; }, [onStatus]); return <div>Preview ready</div>;
  });
  const load = spyOn(pluginRuntimeProduct.draft, 'invoke').mockResolvedValue(value);
  const generate = spyOn(pluginRuntimeProduct.generate, 'invoke');
  const result = render(<I18nextProvider i18n={i18n}><MemoryRouter initialEntries={[`/plugins/create/${value.id}`]}><Routes>
    <Route path='/plugins/create/:draftId' element={<PluginRuntimeCreatorPage />} />
    <Route path='/plugins/run/:id' element={<div>Published destination</div>} />
  </Routes></MemoryRouter></I18nextProvider>);
  await within(result.container).findByRole('heading', { name: value.name, level: 1 });
  return { ...result, view: within(document.body), load, generate };
}
const saveButton = (view: ReturnType<typeof within>) => view.getByRole('button', { name: en.product.saveOpen });

test('publication requires an explicit acknowledgment bound to the returned receipt and revision', async () => {
  let finish!: (value: PluginRuntimeWorkshop) => void;
  const save = spyOn(pluginRuntimeProduct.save, 'invoke').mockRejectedValueOnce(required())
    .mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  const v = await mount();
  fireEvent.click(saveButton(v.view));
  await v.view.findByText(en.product.publishCheck.startupPassed);
  expect(v.view.getByText(en.product.publishCheck.unverified)).toBeTruthy();
  expect(v.view.getByText(en.product.publishCheck.consent)).toBeTruthy();
  expect(save.mock.calls).toEqual([[{ id: draft.id, expected_revision: 4 }]]);
  const confirm = v.view.getByRole('button', { name: en.product.publishCheck.confirm });
  fireEvent.click(confirm); fireEvent.click(confirm);
  expect(save).toHaveBeenCalledTimes(2);
  expect(save.mock.calls[1]).toEqual([{ id: draft.id, expected_revision: 6,
    acknowledge_service_test: { release_digest: receipt.release_digest, receipt_id: receipt.receipt_id } }]);
  await act(async () => { finish(success); });
  expect(v.view.getByText('Published destination')).toBeTruthy();
  expect(v.generate).not.toHaveBeenCalled();
});

test('returning to editing never acknowledges or publishes', async () => {
  const save = spyOn(pluginRuntimeProduct.save, 'invoke').mockRejectedValue(required());
  const v = await mount();
  fireEvent.click(saveButton(v.view));
  await v.view.findByText(en.product.publishCheck.startupPassed);
  fireEvent.click(v.view.getByRole('button', { name: en.product.publishCheck.edit }));
  await waitFor(() => expect(v.view.queryByRole('button', { name: en.product.publishCheck.confirm }) === null).toBe(true));
  expect(save).toHaveBeenCalledTimes(1);
  expect(v.generate).not.toHaveBeenCalled();
  expect(v.view.queryByText('Published destination')).toBeNull();
});

test('stale receipt forces a fresh check; retry uses the refreshed revision without the old acknowledgment', async () => {
  const save = spyOn(pluginRuntimeProduct.save, 'invoke').mockRejectedValueOnce(required())
    .mockRejectedValueOnce(new BackendHttpError({ method: 'POST', path: '/save', status: 409, body: { code: 'STALE', error: 'Changed' } }))
    .mockRejectedValueOnce(required({ ...receipt, expected_revision: 10, receipt_id: 'receipt-two' }));
  const v = await mount();
  v.load.mockResolvedValue({ ...draft, revision: 9, service_source: 'changed source' });
  fireEvent.click(saveButton(v.view));
  await v.view.findByText(en.product.publishCheck.startupPassed);
  fireEvent.click(v.view.getByRole('button', { name: en.product.publishCheck.confirm }));
  await v.view.findByText(en.product.publishCheck.stale);
  await waitFor(() => expect((v.view.getByRole('button', { name: en.product.publishCheck.retry }) as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(v.view.getByRole('button', { name: en.product.publishCheck.retry }));
  await v.view.findByText(en.product.publishCheck.startupPassed);
  expect(save.mock.calls[2]).toEqual([{ id: draft.id, expected_revision: 9 }]);
  expect(save).toHaveBeenCalledTimes(3);
  expect(v.generate).not.toHaveBeenCalled();
});

test('ordinary save failure retries saving and cannot invoke the model-generation retry', async () => {
  const save = spyOn(pluginRuntimeProduct.save, 'invoke').mockRejectedValueOnce(new Error('offline')).mockResolvedValueOnce(success);
  const v = await mount();
  v.load.mockResolvedValue({ ...draft, revision: 7, status: 'ready', error: 'save_failed' });
  fireEvent.click(saveButton(v.view));
  await v.view.findByText(en.product.saveFailed);
  await waitFor(() => expect((v.view.getByRole('button', { name: en.product.publishCheck.retry }) as HTMLButtonElement).disabled).toBe(false));
  expect(v.view.queryByRole('button', { name: en.product.retry })).toBeNull();
  fireEvent.click(v.view.getByRole('button', { name: en.product.publishCheck.retry }));
  await v.view.findByText('Published destination');
  expect(save).toHaveBeenCalledTimes(2);
  expect(v.generate).not.toHaveBeenCalled();
});

test('a persisted save_failed draft offers a save retry, never a generation retry', async () => {
  const save = spyOn(pluginRuntimeProduct.save, 'invoke').mockRejectedValueOnce(required());
  const v = await mount({ ...draft, status: 'ready', error: 'save_failed' });
  expect(v.view.getByText(en.product.saveFailed)).toBeTruthy();
  expect(v.view.queryByRole('button', { name: en.product.retry }) === null).toBe(true);
  fireEvent.click(v.view.getByRole('button', { name: en.product.publishCheck.retry }));
  await v.view.findByText(en.product.publishCheck.startupPassed);
  expect(v.view.queryByText(en.product.saveFailed) === null).toBe(true);
  expect(v.view.queryByText(en.product.generateFailed) === null).toBe(true);
  expect(save).toHaveBeenCalledTimes(1);
  expect(v.generate).not.toHaveBeenCalled();
});

test('a confirmation for another draft is rejected and cannot be acknowledged', async () => {
  const save = spyOn(pluginRuntimeProduct.save, 'invoke').mockRejectedValue(required({ ...receipt, draft_id: 'other' }));
  const v = await mount();
  fireEvent.click(saveButton(v.view));
  await v.view.findByText(en.product.publishCheck.stale);
  expect(v.view.queryByRole('button', { name: en.product.publishCheck.confirm })).toBeNull();
  expect(save).toHaveBeenCalledTimes(1);
});

test('UI-only drafts still save directly after a valid preview', async () => {
  const save = spyOn(pluginRuntimeProduct.save, 'invoke').mockResolvedValue(success);
  const v = await mount({ ...draft, html: '<main>Hello</main>', service_source: null });
  await waitFor(() => expect((saveButton(v.view) as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(saveButton(v.view));
  await v.view.findByText('Published destination');
  expect(save.mock.calls).toEqual([[{ id: draft.id, expected_revision: 4 }]]);
  expect(v.generate).not.toHaveBeenCalled();
});

test('duplicate saves and late responses after unmount never publish a second time or navigate', async () => {
  let finish!: (value: PluginRuntimeWorkshop) => void;
  const save = spyOn(pluginRuntimeProduct.save, 'invoke').mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  const v = await mount();
  fireEvent.click(saveButton(v.view)); fireEvent.click(saveButton(v.view));
  expect(save).toHaveBeenCalledTimes(1);
  v.unmount();
  await act(async () => { finish(success); });
  expect(within(document.body).queryByText('Published destination')).toBeNull();
});

test('save HTTP bridge sends only the explicitly supplied publication acknowledgment', async () => {
  const bodies: unknown[] = [];
  const transport = spyOn(globalThis, 'fetch').mockImplementation(async (_url, init) => {
    bodies.push(JSON.parse(String(init?.body)));
    return new Response(JSON.stringify({ success: true, data: { plugin: { plugin_id: '0190f5fe-7c00-7a00-8000-000000000001' } } }),
      { status: 200, headers: { 'Content-Type': 'application/json' } });
  });
  await pluginRuntimeProduct.save.invoke({ id: draft.id, expected_revision: 4 });
  await pluginRuntimeProduct.save.invoke({ id: draft.id, expected_revision: 6,
    acknowledge_service_test: { release_digest: receipt.release_digest, receipt_id: receipt.receipt_id } });
  expect(bodies).toEqual([{ expected_revision: 4 }, { expected_revision: 6,
    acknowledge_service_test: { release_digest: receipt.release_digest, receipt_id: receipt.receipt_id } }]);
  expect(transport).toHaveBeenCalledTimes(2);
});
