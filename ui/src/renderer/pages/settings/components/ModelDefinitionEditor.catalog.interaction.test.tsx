import '../../../../../test/setup-dom.ts';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { useState } from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import zhSettings from '@/renderer/services/i18n/locales/zh-CN/settings.json';
import ModelDefinitionEditor, { type ModelCatalogSuggestion } from './ModelDefinitionEditor';
import { emptyCapabilityDraft, type ModelDefinitionDraft } from './providerModelAdvanced';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: { settings: zhSettings } } }, interpolation: { escapeValue: false } });
afterEach(cleanup);
const EMPTY: never[] = [];
const MANIFESTS = {};
const catalog = (value: string): ModelCatalogSuggestion => ({ value, label: value, tasks: [], traits: [] });

function Harness({ refresh, initial = [], source, ready = true }: {
  refresh: () => Promise<ModelCatalogSuggestion[]>;
  initial?: ModelCatalogSuggestion[];
  source?: 'remote' | 'official_documentation';
  ready?: boolean;
}) {
  const [value, setValue] = useState<ModelDefinitionDraft>({ model: 'my-manual-id', capabilities: [emptyCapabilityDraft('chat')] });
  const [models, setModels] = useState(initial);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  return <I18nextProvider i18n={i18n}><ModelDefinitionEditor value={value} onChange={setValue}
    providerBaseUrl='https://example.invalid/v1' providerAuthScheme='bearer' manifests={MANIFESTS}
    validationErrors={EMPTY} catalogSuggestions={models} catalogLoading={loading} catalogError={error}
    catalogSource={source} catalogFetchReady={ready} onRefreshCatalog={async () => {
      setLoading(true); setError('');
      try { setModels(await refresh()); } catch { setError(zhSettings.modelCatalogBadRequest); }
      finally { setLoading(false); }
    }} /><output data-testid='model-id'>{value.model}</output></I18nextProvider>;
}

describe('model ID manual input and provider catalog', () => {
  test('refresh opens all returned models while preserving the manually typed ID', async () => {
    let finish!: (value: ModelCatalogSuggestion[]) => void;
    const pending = new Promise<ModelCatalogSuggestion[]>((resolve) => { finish = resolve; });
    const screen = render(<Harness refresh={() => pending} />);
    fireEvent.click(screen.getByRole('button', { name: '获取模型列表' }));
    await waitFor(() => expect(screen.getAllByText('正在获取供应商模型列表…').length).toBeGreaterThan(0));
    const input = screen.getByLabelText('模型 ID') as HTMLInputElement;
    expect(input.disabled).toBe(false);
    expect(input.value).toBe('my-manual-id');
    await act(async () => { finish([catalog('future-model'), catalog('image-model')]); await pending; });
    await waitFor(() => expect(screen.getByRole('option', { name: 'future-model' })).toBeTruthy());
    expect(screen.getByRole('option', { name: 'image-model' })).toBeTruthy();
    expect(input.value).toBe('my-manual-id');
    fireEvent.click(screen.getByRole('option', { name: 'future-model' }));
    expect(screen.getByTestId('model-id').textContent).toBe('future-model');
    fireEvent.change(input, { target: { value: 'custom-unlisted-id' } });
    expect(screen.getByTestId('model-id').textContent).toBe('custom-unlisted-id');
  });

  test('browse shows cached choices even when the current ID matches none of them', async () => {
    const screen = render(<Harness initial={[catalog('official-one'), catalog('official-two')]} refresh={async () => []} />);
    fireEvent.click(screen.getByRole('button', { name: '查看模型列表' }));
    await waitFor(() => expect(screen.getByRole('option', { name: 'official-two' })).toBeTruthy());
    expect((screen.getByLabelText('模型 ID') as HTMLInputElement).value).toBe('my-manual-id');
  });

  test('a failed refresh explains the 400 and keeps cached options and free text usable', async () => {
    const screen = render(<Harness initial={[catalog('cached-model')]} refresh={async () => { throw new Error('400'); }} />);
    fireEvent.click(screen.getByRole('button', { name: '获取模型列表' }));
    await waitFor(() => expect(screen.getAllByText(zhSettings.modelCatalogBadRequest).length).toBeGreaterThan(0));
    expect(screen.queryByText(zhSettings.modelCatalogUnavailable)).toBeNull();
    expect(screen.getByRole('option', { name: 'cached-model' })).toBeTruthy();
    fireEvent.change(screen.getByLabelText('模型 ID'), { target: { value: 'manual-after-error' } });
    expect(screen.getByTestId('model-id').textContent).toBe('manual-after-error');
  });

  test('empty results and official documentation suggestions have explicit feedback', async () => {
    const screen = render(<Harness refresh={async () => []} />);
    fireEvent.click(screen.getByRole('button', { name: '获取模型列表' }));
    await waitFor(() => expect(screen.getAllByText(zhSettings.modelCatalogEmpty).length).toBeGreaterThan(0));
    screen.unmount();
    const reference = render(<Harness source='official_documentation' initial={[catalog('plan-model')]} refresh={async () => []} />);
    expect(reference.getByText('官方建议 · 1')).toBeTruthy();
    expect(reference.getByLabelText(zhSettings.modelCatalogReference)).toBeTruthy();
    expect(reference.container.querySelector('[data-model-catalog-status]')).toBeNull();
  });

  test('missing connection details disable discovery but allow manual input', () => {
    const screen = render(<Harness ready={false} refresh={async () => []} />);
    expect((screen.getByRole('button', { name: '获取模型列表' }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText(zhSettings.modelCatalogNeedsConfiguration)).toBeTruthy();
    fireEvent.change(screen.getByLabelText('模型 ID'), { target: { value: 'manual-no-discovery' } });
    expect(screen.getByTestId('model-id').textContent).toBe('manual-no-discovery');
  });
});
