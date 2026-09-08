import '../../../../../test/setup-dom.ts';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import { useState } from 'react';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { SWRConfig, unstable_serialize } from 'swr';
import zhSettings from '@/renderer/services/i18n/locales/zh-CN/settings.json';
import zhCommon from '@/renderer/services/i18n/locales/zh-CN/common.json';
import type { ModelAdvancedPatch } from './ModelAdvancedEditor';
import { capabilityInputFromResponse } from './providerModelAdvanced';
import { aliasCapability, aliasProtocolManifest, aliasProviderBaseUrl, aliasProviderId } from '../../../../../test/fixtures/modelAliasEditor';

const originalFetch = globalThis.fetch;
const originalWindowFetch = window.fetch;
const testFetch = (async () => new Response(JSON.stringify({ success: true, data: {} }), {
  headers: { 'Content-Type': 'application/json' },
})) as typeof fetch;
const installTransport = () => { globalThis.fetch = testFetch; window.fetch = testFetch; };
const restoreTransport = () => { globalThis.fetch = originalFetch; window.fetch = originalWindowFetch; };
installTransport();
const { default: ModelAdvancedEditor } = await import('./ModelAdvancedEditor');
const { ThemeProvider } = await import('@/renderer/hooks/context/ThemeContext');
restoreTransport();
beforeEach(installTransport);

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'zh-CN', resources: { 'zh-CN': { translation: { settings: zhSettings, common: zhCommon } } },
  interpolation: { escapeValue: false },
});
const capabilities = [aliasCapability];
const requestKey = JSON.stringify(['preview', aliasProviderBaseUrl, ['chat']]);
const aliasLabel = zhSettings.modelDisplayNameTitle;
afterEach(() => { cleanup(); restoreTransport(); });

function mount(failSave = false) {
  const saved: ModelAdvancedPatch[] = [];
  const config = {
    provider: () => new Map(), revalidateOnMount: false,
    fallback: {
      [unstable_serialize(['model-protocol-manifests', requestKey])]: { requestKey, manifests: { chat: aliasProtocolManifest }, errorTasks: [] },
      [`provider-connections:${aliasProviderId}`]: [],
    },
  };
  function Harness() {
    const [displayName, setDisplayName] = useState<string | undefined>('原有别名');
    return <ModelAdvancedEditor providerId={aliasProviderId} providerName='测试服务商' preset='preview'
      providerBaseUrl={aliasProviderBaseUrl} providerAuthScheme='bearer' model='immutable-model-id'
      displayName={displayName} capabilities={capabilities} onSave={async (patch) => {
        saved.push(patch);
        if (failSave) throw new Error('save failed');
        setDisplayName(patch.display_name ?? undefined);
      }} />;
  }
  const page = render(<I18nextProvider i18n={i18n}><SWRConfig value={config}><ThemeProvider><Harness /></ThemeProvider></SWRConfig></I18nextProvider>);
  const open = async () => {
    fireEvent.click(page.getByRole('button', { name: zhSettings.editModelCapabilities }));
    return await page.findByLabelText(aliasLabel) as HTMLInputElement;
  };
  return { page, open, saved };
}

describe('existing model alias editing', () => {
  test('shows the saved alias immediately, saves a trimmed alias, and restores it on reopen', async () => {
    const { page, open, saved } = mount();
    const input = await open();
    expect(input.value).toBe('原有别名');
    expect((page.getByLabelText(zhSettings.modelId) as HTMLInputElement).readOnly).toBe(true);
    fireEvent.change(input, { target: { value: '  新别名  ' } });
    fireEvent.click(page.getByRole('button', { name: zhCommon.save }));
    await waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0]).toEqual({ display_name: '新别名', capabilities: [capabilityInputFromResponse(aliasCapability)] });
    expect((await open()).value).toBe('新别名');
  });

  test('clearing the alias saves explicit removal and does not restore the previous value', async () => {
    const { page, open, saved } = mount();
    fireEvent.change(await open(), { target: { value: '' } });
    fireEvent.click(page.getByRole('button', { name: zhCommon.save }));
    await waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0].display_name).toBeNull();
    expect(saved[0].capabilities).toEqual([capabilityInputFromResponse(aliasCapability)]);
    expect((await open()).value).toBe('');
  });

  test('cancel discards the alias draft and reopens the persisted value', async () => {
    const { page, open, saved } = mount();
    fireEvent.change(await open(), { target: { value: '不保存' } });
    fireEvent.click(page.getByRole('button', { name: zhCommon.cancel }));
    expect(saved).toHaveLength(0);
    expect((await open()).value).toBe('原有别名');
  });

  test('a failed save keeps the alias draft available for retry', async () => {
    const { page, open, saved } = mount(true);
    fireEvent.change(await open(), { target: { value: '等待重试' } });
    fireEvent.click(page.getByRole('button', { name: zhCommon.save }));
    await waitFor(() => expect(saved).toHaveLength(1));
    expect((page.getByLabelText(aliasLabel) as HTMLInputElement).value).toBe('等待重试');
  });
});
