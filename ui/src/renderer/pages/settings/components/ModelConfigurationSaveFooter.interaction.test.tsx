import '../../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, describe, expect, mock, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import zhCommon from '@/renderer/services/i18n/locales/zh-CN/common.json';
import zhSettings from '@/renderer/services/i18n/locales/zh-CN/settings.json';
import ModelConfigurationSaveFooter, { type ModelConfigurationSaveFooterProps } from './ModelConfigurationSaveFooter';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: { common: zhCommon, settings: zhSettings } } }, interpolation: { escapeValue: false } });
afterEach(cleanup);

const footer = (props: ModelConfigurationSaveFooterProps) => <I18nextProvider i18n={i18n}><ModelConfigurationSaveFooter {...props} /></I18nextProvider>;
const actions = () => ({ onCancel: mock(() => {}), onSave: mock(() => {}), onForceSave: mock(() => {}) });

describe('model configuration save footer', () => {
  test('configuration pending disables normal confirmation and transitions to an enabled unchanged confirm label', () => {
    const callbacks = actions();
    const page = render(footer({ ...callbacks, configuring: true, saving: false, disabled: false }));
    const confirm = page.getByRole('button', { name: zhCommon.confirm }) as HTMLButtonElement;
    expect(confirm.disabled).toBe(true);
    expect(confirm.classList.contains('arco-btn-loading')).toBe(true);
    expect(confirm.getAttribute('aria-busy')).toBe('true');
    expect(confirm.textContent).toBe(zhCommon.confirm);
    expect(page.getByRole('button', { name: zhSettings.modelProtocolDiscovery.forceSave })).toBeTruthy();
    fireEvent.click(confirm);
    expect(callbacks.onSave).not.toHaveBeenCalled();

    page.rerender(footer({ ...callbacks, configuring: false, saving: false, disabled: false }));
    expect(confirm.disabled).toBe(false);
    expect(confirm.classList.contains('arco-btn-loading')).toBe(false);
    expect(confirm.getAttribute('aria-busy')).toBe('false');
    expect(confirm.textContent).toBe(zhCommon.confirm);
    expect(page.queryByRole('button', { name: zhSettings.modelProtocolDiscovery.forceSave })).toBeNull();
    fireEvent.click(confirm);
    expect(callbacks.onSave).toHaveBeenCalledTimes(1);
    expect(callbacks.onForceSave).not.toHaveBeenCalled();
  });

  test('force save has its own callback and tooltip while cancellation remains available during configuration', async () => {
    const callbacks = actions();
    const page = render(footer({ ...callbacks, configuring: true, saving: false, disabled: false }));
    const forceSave = page.getByRole('button', { name: zhSettings.modelProtocolDiscovery.forceSave }) as HTMLButtonElement;
    const cancel = page.getByRole('button', { name: zhCommon.cancel }) as HTMLButtonElement;
    expect(forceSave.disabled).toBe(false);
    expect(forceSave.classList.contains('arco-btn-text')).toBe(true);
    expect(cancel.disabled).toBe(false);
    fireEvent.click(forceSave);
    expect(callbacks.onForceSave).toHaveBeenCalledTimes(1);
    expect(callbacks.onSave).not.toHaveBeenCalled();
    fireEvent.click(cancel);
    expect(callbacks.onCancel).toHaveBeenCalledTimes(1);
    fireEvent.mouseEnter(forceSave);
    expect(await page.findByText(zhSettings.modelProtocolDiscovery.forceSaveHint)).toBeTruthy();
  });

  for (const state of [
    { name: 'invalid configuration', saving: false, disabled: true },
    { name: 'an active save', saving: true, disabled: false },
    { name: 'an invalid configuration during an active save', saving: true, disabled: true },
  ]) test(`${state.name} cannot use either save action`, () => {
    const callbacks = actions();
    const page = render(footer({ ...callbacks, configuring: true, saving: state.saving, disabled: state.disabled }));
    const confirm = page.getByRole('button', { name: zhCommon.confirm }) as HTMLButtonElement;
    const forceSave = page.getByRole('button', { name: zhSettings.modelProtocolDiscovery.forceSave }) as HTMLButtonElement;
    const cancel = page.getByRole('button', { name: zhCommon.cancel }) as HTMLButtonElement;
    expect(confirm.disabled).toBe(true);
    expect(forceSave.disabled).toBe(true);
    fireEvent.click(confirm);
    fireEvent.click(forceSave);
    expect(callbacks.onSave).not.toHaveBeenCalled();
    expect(callbacks.onForceSave).not.toHaveBeenCalled();
    expect(cancel.disabled).toBe(state.saving);
    fireEvent.click(cancel);
    expect(callbacks.onCancel).toHaveBeenCalledTimes(state.saving ? 0 : 1);
  });

  test('an active save keeps the custom label and disables cancellation until it finishes', () => {
    const callbacks = actions();
    const page = render(footer({ ...callbacks, configuring: false, saving: true, disabled: false, confirmText: zhCommon.save }));
    const save = page.getByRole('button', { name: zhCommon.save }) as HTMLButtonElement;
    const cancel = page.getByRole('button', { name: zhCommon.cancel }) as HTMLButtonElement;
    expect(save.disabled).toBe(true);
    expect(save.classList.contains('arco-btn-loading')).toBe(true);
    expect(save.textContent).toBe(zhCommon.save);
    expect(cancel.disabled).toBe(true);
    expect(page.queryByRole('button', { name: zhSettings.modelProtocolDiscovery.forceSave })).toBeNull();
    fireEvent.click(save);
    fireEvent.click(cancel);
    expect(callbacks.onSave).not.toHaveBeenCalled();
    expect(callbacks.onCancel).not.toHaveBeenCalled();

    page.rerender(footer({ ...callbacks, configuring: false, saving: false, disabled: false, confirmText: zhCommon.save }));
    expect(save.disabled).toBe(false);
    expect(save.classList.contains('arco-btn-loading')).toBe(false);
    expect(cancel.disabled).toBe(false);
    fireEvent.click(save);
    expect(callbacks.onSave).toHaveBeenCalledTimes(1);
  });
});
