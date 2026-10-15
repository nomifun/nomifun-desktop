import '../../../../test/setup-dom.ts';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { Message } from '@arco-design/web-react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import type { ReactElement } from 'react';

import * as theme from '@/renderer/hooks/context/ThemeContext';
import * as clipboard from '@/renderer/utils/ui/clipboard';
import { ErrorDetails, ErrorModal, ErrorNotice, type ErrorFeedbackPresentation } from './ErrorFeedback';

const i18n = createInstance();
await i18n.init({ lng: 'zh-CN', resources: {} });
const restores: Array<() => void> = [];
const error: ErrorFeedbackPresentation = {
  title: '任务失败',
  message: '生成服务暂时不可用，请稍后重试。',
  technicalDetails: 'Provider request failed: 503',
  handled: false,
};
const mount = (element: ReactElement) => render(<I18nextProvider i18n={i18n}>{element}</I18nextProvider>);

beforeEach(() => {
  const context = spyOn(theme, 'useThemeContext').mockReturnValue({
    theme: 'light', colorScheme: 'default', fontScale: 1,
    setTheme: async () => {}, setColorScheme: async () => {}, setFontScale: async () => {},
  });
  restores.push(() => context.mockRestore());
});
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach((restore) => restore()); });

test('keeps diagnostics collapsed and copies only redacted technical information', () => {
  const copy = spyOn(clipboard, 'copyText').mockResolvedValue(undefined);
  const toast = spyOn(Message, 'success').mockImplementation(() => () => {});
  restores.push(() => copy.mockRestore(), () => toast.mockRestore());
  const view = mount(<ErrorDetails technicalDetails={'Authorization: Bearer fixture-secret\nHTTP 503'} />);
  const details = view.container.querySelector('details')!;

  expect(details.open).toBe(false);
  expect(view.getByText('技术信息')).toBeTruthy();
  expect(view.container.textContent).not.toContain('fixture-secret');
  expect(view.container.querySelector('pre')?.textContent).toBe('Authorization: Bearer [REDACTED]\nHTTP 503');
  details.open = true;
  fireEvent.click(view.getByRole('button', { name: '复制技术信息' }));
  expect(copy).toHaveBeenCalledWith('Authorization: Bearer [REDACTED]\nHTTP 503');
});

test('shows the shared modal body, keeps mask clicks inert, and closes before retrying', async () => {
  const calls: string[] = [];
  const view = mount(<ErrorModal error={error} onClose={() => calls.push('close')} onRetry={() => calls.push('retry')} retryLabel='重新运行' />);
  const dialog = view.getByRole('dialog', { name: error.title });
  expect(dialog.style.width).toBe('520px');
  expect(document.getElementById(dialog.getAttribute('aria-describedby')!)?.textContent).toBe(error.message);
  expect(document.querySelector('details')?.open).toBe(false);

  const mask = dialog.closest('.arco-modal-wrapper')!;
  await act(async () => {
    fireEvent.mouseDown(mask);
    fireEvent.click(mask);
    await new Promise((resolve) => setTimeout(resolve, 125));
  });
  expect(calls).toEqual([]);
  fireEvent.click(view.getByRole('button', { name: '重新运行' }));
  expect(calls).toEqual(['close', 'retry']);
  fireEvent.click(view.getByRole('button', { name: '关闭' }));
  fireEvent.click(view.getByRole('button', { name: 'Close' }));
  fireEvent.keyDown(view.getByRole('button', { name: '关闭' }), { key: 'Escape' });
  expect(calls).toEqual(['close', 'retry', 'close', 'close', 'close']);
});

test('keeps notice retry separate and suppresses handled or absent errors', () => {
  const retry = mock(() => {});
  const view = mount(<ErrorNotice error={error} onRetry={retry} />);
  expect(view.getByRole('alert').textContent).toContain(error.message);
  fireEvent.click(view.getByRole('button', { name: '重试' }));
  expect(retry).toHaveBeenCalledTimes(1);

  view.rerender(<I18nextProvider i18n={i18n}>
    <ErrorNotice error={{ ...error, handled: true }} />
    <ErrorModal error={{ ...error, handled: true }} onClose={() => {}} />
    <ErrorModal error={null} onClose={() => {}} />
    <ErrorDetails technicalDetails='  ' />
  </I18nextProvider>);
  expect(view.queryByRole('alert')).toBeNull();
  expect(view.queryByRole('dialog')).toBeNull();
  expect(view.container.querySelector('details')).toBeNull();
});
