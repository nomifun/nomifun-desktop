import '../../../../../test/setup-dom.ts';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, expect, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { useState } from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import zhSettings from '@/renderer/services/i18n/locales/zh-CN/settings.json';
import { aliasProtocolManifest, aliasProviderBaseUrl } from '../../../../../test/fixtures/modelAliasEditor';
import ModelDefinitionEditor from './ModelDefinitionEditor';
import { capabilityInputsFromDefinition, createModelDefinitionDraft, type ModelProtocolManifestMap } from './providerModelAdvanced';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: { settings: zhSettings } } }, interpolation: { escapeValue: false } });
afterEach(cleanup);
const noErrors: never[] = [];
const manifestFor = (model: string): ModelProtocolManifestMap => ({ chat: { ...aliasProtocolManifest, model_defaults: {
  model, protocol_id: 'openai.chat_text', context_limit: model === 'vendor/a' ? 100_000 : 200_000,
  output_limit: 8_192, provider_params: { reasoning_effort: 'high' }, reasoning_efforts: ['low', 'high'],
  sources: ['https://vendor.example/docs/model'], checked_at: '2026-10-15',
} } });
function Harness() {
  const [value, setValue] = useState({ ...createModelDefinitionDraft('chat'), model: 'vendor/a' });
  const [manifests, setManifests] = useState<ModelProtocolManifestMap>({ chat: aliasProtocolManifest });
  return <I18nextProvider i18n={i18n}>
    <button onClick={() => setManifests(manifestFor(value.model))}>Load model defaults</button>
    <ModelDefinitionEditor value={value} onChange={setValue} providerBaseUrl={aliasProviderBaseUrl}
      providerAuthScheme='bearer' manifests={manifests} validationErrors={noErrors} />
    <output data-testid='serialized'>{JSON.stringify(capabilityInputsFromDefinition(value))}</output>
  </I18nextProvider>;
}

test('late defaults fill the form, keep references collapsed, and preserve exact effort choices', async () => {
  const page = render(<Harness />);
  fireEvent.click(page.getByRole('button', { name: 'Load model defaults' }));
  const saved = () => JSON.parse(page.getByTestId('serialized').textContent!)[0];
  await waitFor(() => expect(saved().context_limit).toBe(100_000));
  expect(saved().provider_params).toEqual({ reasoning_effort: 'high' });
  expect(page.queryByText(/资料核对日期/)).toBeNull();
  expect(page.queryByRole('link')).toBeNull();
  fireEvent.click(page.getByRole('button', { name: zhSettings.modelAdvanced.referenceTitle }));
  expect(await page.findByText(/资料核对日期：2026-10-15/)).toBeTruthy();
  const source = page.getByRole('link', { name: /docs\/model.*vendor.example/ });
  expect((source as HTMLAnchorElement).href).toBe('https://vendor.example/docs/model');
  const open = spyOn(window, 'open').mockReturnValue(null);
  try {
    fireEvent.click(source);
    expect(open).toHaveBeenCalledWith('https://vendor.example/docs/model', '_blank', 'noopener,noreferrer');
  } finally {
    open.mockRestore();
  }
  fireEvent.click(page.getByRole('button', { name: zhSettings.modelAdvanced.referenceTitle }));
  fireEvent.click(page.container.querySelector<HTMLButtonElement>('[data-capability-disclosure="chat"]')!);
  fireEvent.click(page.getByRole('tab', { name: zhSettings.modelAdvanced.reasoningEffort }));
  expect(page.container.querySelector('[data-reasoning-effort="xhigh"]')).toBeNull();
  expect(page.container.querySelector('[data-reasoning-effort="none"]')).toBeNull();
  fireEvent.click(page.container.querySelector<HTMLButtonElement>('[data-reasoning-effort="low"]')!);
  expect(saved().provider_params.reasoning_effort).toBe('low');
  fireEvent.click(page.getByRole('button', { name: zhSettings.modelAdvanced.cancelAdjustment }));
  await waitFor(() => expect(saved().provider_params.reasoning_effort).toBe('high'));
  fireEvent.change(page.getByLabelText(zhSettings.modelId), { target: { value: 'vendor/b' } });
  expect(saved()).not.toHaveProperty('context_limit');
  expect(saved()).not.toHaveProperty('provider_params');
  fireEvent.click(page.getByRole('button', { name: 'Load model defaults' }));
  await waitFor(() => expect(saved().context_limit).toBe(200_000));
});
