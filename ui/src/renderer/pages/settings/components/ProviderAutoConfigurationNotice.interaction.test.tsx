import '../../../../../test/setup-dom.ts';
import { cleanup, fireEvent, render, within } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import type { ComponentProps } from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import zhSettings from '@/renderer/services/i18n/locales/zh-CN/settings.json';
import ProviderAutoConfigurationNotice from './ProviderAutoConfigurationNotice';
import type { ProviderAutoConfigurationDetection } from './providerAutoConfiguration';
import { emptyCapabilityDraft, type ModelDefinitionDraft } from './providerModelAdvanced';
import type { ProviderAutoConfigurationBatch } from './useProviderAutoConfiguration';

afterEach(cleanup);
const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'zh-CN', resources: { 'zh-CN': { translation: { settings: zhSettings } } }, interpolation: { escapeValue: false },
});
const available: ProviderAutoConfigurationDetection = {
  task: 'chat', protocol: 'openai.chat_text', authScheme: 'bearer', confidence: 'verified',
};
const rejected: ProviderAutoConfigurationDetection = {
  task: 'chat', protocol: 'anthropic.messages', authScheme: 'header_key:x-api-key', confidence: 'endpoint_confirmed',
};
const unconfirmed: ProviderAutoConfigurationDetection = {
  task: 'chat', protocol: 'openai.responses', authScheme: 'bearer', confidence: 'fallback',
  suggestedBaseUrl: 'https://example.invalid/v1',
};
const definition: ModelDefinitionDraft = {
  model: 'test-model', capabilities: [{ ...emptyCapabilityDraft('chat'), protocol: available.protocol }],
};
const batch: ProviderAutoConfigurationBatch = {
  requestBaseUrl: 'https://example.invalid', detections: [available], candidates: [available, rejected, unconfirmed],
};
function mount(props: Partial<ComponentProps<typeof ProviderAutoConfigurationNotice>> = {}) {
  return render(<I18nextProvider i18n={i18n}>
    <ProviderAutoConfigurationNotice enabled loading={false} batch={batch} definition={definition} {...props} />
  </I18nextProvider>);
}

describe('provider interface detection choices', () => {
  test('shows every result and allows selecting an explicitly unconfirmed protocol', () => {
    const selections: ProviderAutoConfigurationDetection[] = [];
    const page = mount({ onSelect: value => selections.push(value) });
    const results = within(page.getByRole('group', { name: '接口检测结果' }));
    expect(results.getAllByRole('button')).toHaveLength(3);
    expect(results.getByText('接口可用')).toBeTruthy();
    expect(results.getByText('鉴权失败')).toBeTruthy();
    expect(results.getByText('未确认')).toBeTruthy();
    expect(results.getByText(unconfirmed.suggestedBaseUrl!)).toBeTruthy();
    expect(results.getByRole('button', { name: available.protocol }).getAttribute('aria-pressed')).toBe('true');
    const option = results.getByRole('button', { name: unconfirmed.protocol });
    expect(option.getAttribute('aria-pressed')).toBe('false');
    expect((option as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(option);
    expect(selections).toEqual([unconfirmed]);
    expect(page.queryByText(zhSettings.modelProtocolDiscovery.connected)).toBeNull();
    expect(page.queryByText(zhSettings.modelProtocolDiscovery.verified)).toBeNull();
  });

  test('distinguishes task choices and marks each configured protocol', () => {
    const image: ProviderAutoConfigurationDetection = { ...available, task: 'image_generation', protocol: 'openai.images' };
    const selections: ProviderAutoConfigurationDetection[] = [];
    const page = mount({
      batch: { ...batch, candidates: [available, image] },
      definition: { ...definition, capabilities: [
        { ...emptyCapabilityDraft('chat'), protocol: available.protocol },
        { ...emptyCapabilityDraft('image_generation'), protocol: image.protocol },
      ] },
      onSelect: value => selections.push(value),
    });
    const chatButton = page.getByRole('button', { name: `${zhSettings.modelTask.chat} · ${available.protocol}` });
    const imageButton = page.getByRole('button', { name: `${zhSettings.modelTask.image_generation} · ${image.protocol}` });
    expect(chatButton.getAttribute('aria-pressed')).toBe('true');
    expect(imageButton.getAttribute('aria-pressed')).toBe('true');
    fireEvent.click(imageButton);
    expect(selections).toEqual([image]);
  });

  test('keeps displayed results disabled while probing', () => {
    const selections: ProviderAutoConfigurationDetection[] = [];
    let rechecks = 0;
    const page = mount({ loading: true, onSelect: value => selections.push(value), onRecheck: () => { rechecks++; } });
    for (const option of page.getAllByRole('button')) {
      expect((option as HTMLButtonElement).disabled).toBe(true);
      fireEvent.click(option);
    }
    expect(selections).toEqual([]);
    expect(rechecks).toBe(0);
  });

  test('keeps changed results visible without presenting them as current or allowing selection', () => {
    const selections: ProviderAutoConfigurationDetection[] = [];
    let rechecks = 0;
    const page = mount({ stale: true, onSelect: value => selections.push(value), onRecheck: () => { rechecks++; } });
    expect(page.getByRole('status').textContent).toContain('配置已修改，尚未重新检测');
    expect(page.container.querySelector('[data-provider-auto-configuration]')?.getAttribute('data-provider-auto-configuration')).toBe('stale');
    expect(page.queryByText(zhSettings.providerAutoConfiguration.applied)).toBeNull();
    expect(page.container.querySelector('.arco-tag-green')).toBeNull();
    const results = within(page.getByRole('group', { name: '接口检测结果' }));
    expect(results.getAllByRole('button')).toHaveLength(3);
    expect(results.getByText(unconfirmed.suggestedBaseUrl!)).toBeTruthy();
    for (const option of results.getAllByRole('button')) {
      expect((option as HTMLButtonElement).disabled).toBe(true);
      fireEvent.click(option);
    }
    expect(selections).toEqual([]);
    const recheck = page.getByRole('button', { name: zhSettings.modelProtocolDiscovery.recheck });
    expect((recheck as HTMLButtonElement).disabled).toBe(false);
    expect(recheck.hasAttribute('data-provider-protocol-recheck')).toBe(true);
    fireEvent.click(recheck);
    expect(rechecks).toBe(1);
  });

  test('provides explicit detection before any results and disables it until connection fields are ready', () => {
    let rechecks = 0;
    const page = mount({ batch: undefined, onRecheck: () => { rechecks++; }, canRecheck: false });
    expect(page.getByRole('status').textContent).toContain(zhSettings.modelProtocolDiscovery.needsConfiguration);
    expect(page.queryByRole('group', { name: '接口检测结果' })).toBeNull();
    const recheck = page.getByRole('button', { name: zhSettings.modelProtocolDiscovery.recheck }) as HTMLButtonElement;
    expect(recheck.disabled).toBe(true);
    fireEvent.click(recheck);
    expect(rechecks).toBe(0);
    page.rerender(<I18nextProvider i18n={i18n}>
      <ProviderAutoConfigurationNotice enabled loading={false} onRecheck={() => { rechecks++; }} canRecheck />
    </I18nextProvider>);
    expect(page.getByRole('status').textContent).toContain(zhSettings.healthNotChecked);
    expect(recheck.disabled).toBe(false);
    fireEvent.click(recheck);
    expect(rechecks).toBe(1);
    expect(page.queryByText(zhSettings.providerAutoConfiguration.applied)).toBeNull();
  });

  test('keeps fresh task choices available when another task has changed', () => {
    const image: ProviderAutoConfigurationDetection = { ...available, task: 'image_generation', protocol: 'openai.images' };
    const selections: ProviderAutoConfigurationDetection[] = [];
    const page = mount({
      stale: true, batch: { ...batch, detections: [image], candidates: [available, image], staleTasks: ['chat'] },
      definition: { ...definition, capabilities: [
        { ...emptyCapabilityDraft('chat'), protocol: available.protocol },
        { ...emptyCapabilityDraft('image_generation'), protocol: image.protocol },
      ] },
      onSelect: value => selections.push(value),
    });
    const chatButton = page.getByRole('button', { name: `${zhSettings.modelTask.chat} · ${available.protocol}` }) as HTMLButtonElement;
    const imageButton = page.getByRole('button', { name: `${zhSettings.modelTask.image_generation} · ${image.protocol}` }) as HTMLButtonElement;
    expect(page.getByRole('status').textContent).toContain('配置已修改，尚未重新检测');
    expect(chatButton.disabled).toBe(true);
    expect(chatButton.parentElement?.querySelector('.arco-tag-green')).toBeNull();
    expect(imageButton.disabled).toBe(false);
    expect(imageButton.parentElement?.querySelector('.arco-tag-green')).not.toBeNull();
    fireEvent.click(chatButton);
    fireEvent.click(imageButton);
    expect(selections).toEqual([image]);
  });

  test('keeps read-only results disabled', () => {
    const page = mount({ batch: { requestBaseUrl: batch.requestBaseUrl, detections: [available], candidates: [available] } });
    const option = page.getByRole('button', { name: available.protocol });
    expect((option as HTMLButtonElement).disabled).toBe(true);
    expect(page.getAllByRole('button')).toHaveLength(1);
  });

  test('the summary follows the chosen protocol and its confidence', () => {
    const page = mount({ definition: { ...definition, capabilities: [{ ...emptyCapabilityDraft('chat'), protocol: rejected.protocol }] } });
    expect(page.getByRole('status').textContent).toContain(rejected.protocol);
    expect(page.getByRole('status').textContent).not.toContain(available.protocol);
    expect(page.container.querySelector('[data-provider-auto-configuration]')?.getAttribute('data-provider-auto-configuration')).toBe('endpoint_confirmed');
  });

  test('hides results for providers without automatic interface detection', () => {
    const page = mount({ enabled: false });
    expect(page.queryByRole('group', { name: '接口检测结果' })).toBeNull();
    expect(page.queryByRole('status')).toBeNull();
  });
});
