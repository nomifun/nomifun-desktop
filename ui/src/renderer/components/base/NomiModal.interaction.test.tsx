/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import * as theme from '@/renderer/hooks/context/ThemeContext';
import NomiModal, { type NomiModalProps } from './NomiModal';

const i18n = createInstance();
await i18n.init({ lng: 'en', resources: {} });

let restoreTheme: () => void;
beforeEach(() => {
  const context = spyOn(theme, 'useThemeContext').mockReturnValue({
    theme: 'light', colorScheme: 'default', fontScale: 1,
    setTheme: async () => {}, setColorScheme: async () => {}, setFontScale: async () => {},
  });
  restoreTheme = () => context.mockRestore();
});
afterEach(() => {
  cleanup();
  restoreTheme();
});

const modal = (props: NomiModalProps = {}) => (
  <I18nextProvider i18n={i18n}>
    <NomiModal visible header='Model settings' okText='Save' cancelText='Cancel' {...props}>
      Dialog content
    </NomiModal>
  </I18nextProvider>
);

test('default confirm button follows disabled changes and never confirms while disabled', () => {
  const onOk = mock(() => {});
  const page = render(modal({ onOk, okButtonProps: { disabled: true } }));
  const save = page.getByRole('button', { name: 'Save' }) as HTMLButtonElement;
  expect(page.getByRole('dialog').classList.contains('nomifun-modal')).toBe(true);
  expect(save.disabled).toBe(true);
  fireEvent.click(save);
  expect(onOk).not.toHaveBeenCalled();

  page.rerender(modal({ onOk, okButtonProps: { disabled: false } }));
  expect(save.disabled).toBe(false);
  fireEvent.click(save);
  expect(onOk).toHaveBeenCalledTimes(1);
});

test('both modal and caller loading states suppress confirmation until rerendered idle', () => {
  const onOk = mock(() => {});
  const page = render(modal({ onOk, confirmLoading: true, okButtonProps: { loading: false } }));
  const save = page.getByRole('button', { name: 'Save' });
  expect(save.classList.contains('arco-btn-loading')).toBe(true);
  fireEvent.click(save);
  expect(onOk).not.toHaveBeenCalled();

  page.rerender(modal({ onOk, confirmLoading: false, okButtonProps: { loading: true } }));
  expect(save.classList.contains('arco-btn-loading')).toBe(true);
  fireEvent.click(save);
  expect(onOk).not.toHaveBeenCalled();

  page.rerender(modal({ onOk, confirmLoading: false, okButtonProps: { loading: false } }));
  expect(save.classList.contains('arco-btn-loading')).toBe(false);
  fireEvent.click(save);
  expect(onOk).toHaveBeenCalledTimes(1);
});

test('default buttons merge host classes and styles while forwarding native button properties', () => {
  const page = render(modal({
    okButtonProps: {
      className: ['host-save', 'host-save-extra'], style: { borderRadius: 12, marginLeft: 3 },
      type: 'outline', status: 'danger', size: 'small', title: 'Confirm changes',
    },
    cancelButtonProps: {
      className: 'host-cancel', style: { marginRight: 5 }, type: 'text', title: 'Discard changes',
    },
  }));
  const save = page.getByRole('button', { name: 'Save' });
  for (const className of ['px-16px', 'min-w-72px', 'host-save', 'host-save-extra', 'arco-btn-outline', 'arco-btn-status-danger', 'arco-btn-size-small']) {
    expect(save.classList.contains(className)).toBe(true);
  }
  expect(save.style.borderRadius).toBe('12px');
  expect(save.style.marginLeft).toBe('3px');
  expect(save.getAttribute('title')).toBe('Confirm changes');

  const cancel = page.getByRole('button', { name: 'Cancel' });
  for (const className of ['px-16px', 'min-w-72px', 'host-cancel', 'arco-btn-text']) {
    expect(cancel.classList.contains(className)).toBe(true);
  }
  expect(cancel.style.borderRadius).toBe('8px');
  expect(cancel.style.marginRight).toBe('5px');
  expect(cancel.getAttribute('title')).toBe('Discard changes');
});

test('cancel button respects caller disabled and loading states without suppressing later cancellation', () => {
  const onCancel = mock(() => {});
  const page = render(modal({ onCancel, cancelButtonProps: { disabled: true } }));
  const cancel = page.getByRole('button', { name: 'Cancel' }) as HTMLButtonElement;
  expect(cancel.disabled).toBe(true);
  fireEvent.click(cancel);
  expect(onCancel).not.toHaveBeenCalled();

  page.rerender(modal({ onCancel, cancelButtonProps: { loading: true } }));
  expect(cancel.disabled).toBe(false);
  expect(cancel.classList.contains('arco-btn-loading')).toBe(true);
  fireEvent.click(cancel);
  expect(onCancel).not.toHaveBeenCalled();

  page.rerender(modal({ onCancel, cancelButtonProps: { loading: false, disabled: false } }));
  fireEvent.click(cancel);
  expect(onCancel).toHaveBeenCalledTimes(1);
});

test('explicit button click handlers retain native override semantics and disabled guards', () => {
  const onOk = mock(() => {});
  const onCancel = mock(() => {});
  const onButtonOk = mock(() => {});
  const onButtonCancel = mock(() => {});
  const page = render(modal({
    onOk, onCancel,
    okButtonProps: { onClick: onButtonOk, disabled: true },
    cancelButtonProps: { onClick: onButtonCancel, disabled: true },
  }));
  fireEvent.click(page.getByRole('button', { name: 'Save' }));
  fireEvent.click(page.getByRole('button', { name: 'Cancel' }));
  expect(onButtonOk).not.toHaveBeenCalled();
  expect(onButtonCancel).not.toHaveBeenCalled();

  page.rerender(modal({ onOk, onCancel, okButtonProps: { onClick: onButtonOk }, cancelButtonProps: { onClick: onButtonCancel } }));
  fireEvent.click(page.getByRole('button', { name: 'Save' }));
  fireEvent.click(page.getByRole('button', { name: 'Cancel' }));
  expect(onButtonOk).toHaveBeenCalledTimes(1);
  expect(onButtonCancel).toHaveBeenCalledTimes(1);
  expect(onOk).not.toHaveBeenCalled();
  expect(onCancel).not.toHaveBeenCalled();
});

test('custom footer and header content remain independent from default button props', () => {
  const customClick = mock(() => {});
  const page = render(modal({
    header: { render: () => <strong>Custom heading</strong> },
    footer: <button onClick={customClick}>Own action</button>,
    confirmLoading: true, okButtonProps: { disabled: true }, cancelButtonProps: { disabled: true },
  }));
  expect(page.getByText('Custom heading')).toBeTruthy();
  expect(page.queryByRole('button', { name: 'Save' })).toBeNull();
  expect(page.queryByRole('button', { name: 'Cancel' })).toBeNull();
  fireEvent.click(page.getByRole('button', { name: 'Own action' }));
  expect(customClick).toHaveBeenCalledTimes(1);

  page.rerender(modal({ footer: { render: () => <button>Configured action</button> }, okButtonProps: { disabled: true } }));
  expect(page.getByRole('button', { name: 'Configured action' })).toBeTruthy();
  expect(page.queryByRole('button', { name: 'Save' })).toBeNull();
  page.rerender(modal({ footer: null }));
  expect(page.queryByRole('button', { name: 'Cancel' })).toBeNull();
  expect(page.queryByRole('button', { name: 'Save' })).toBeNull();
});
