import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { cleanup, fireEvent, render } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { Message } from '@arco-design/web-react';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import * as theme from '@/renderer/hooks/context/ThemeContext';
import * as clipboard from '@/renderer/utils/ui/clipboard';
import { KnowledgeErrorModal, KnowledgeErrorNotice, showKnowledgeErrorMessage } from './KnowledgeErrorFeedback';
import { normalizeKnowledgeError } from './knowledgeError';

const i18n = createInstance();
await i18n.init({ lng: 'zh-CN', resources: {} });
const restores: Array<() => void> = [];
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach((restore) => restore()); });

function exportError() {
  return normalizeKnowledgeError(new BackendHttpError({
    method: 'POST', path: '/api/knowledge/bases/base-id/export', status: 500,
    body: { code: 'INTERNAL_ERROR', error: 'failed to finalize export file: os error 5' },
  }), 'export', i18n.t.bind(i18n));
}

test('export modal keeps technical details collapsed and closes before choosing a new location', () => {
  const themeSpy = spyOn(theme, 'useThemeContext').mockReturnValue({
    theme: 'light', colorScheme: 'default', fontScale: 1,
    setTheme: async () => {}, setColorScheme: async () => {}, setFontScale: async () => {},
  });
  restores.push(() => themeSpy.mockRestore());
  const calls: string[] = [];
  const view = render(<I18nextProvider i18n={i18n}>
    <KnowledgeErrorModal error={exportError()} onClose={() => calls.push('close')} onRetry={() => calls.push('retry')} />
  </I18nextProvider>);
  const dialog = view.getByRole('dialog', { name: '资料导出失败' });
  expect(dialog.getAttribute('aria-describedby')).toBeTruthy();
  expect(view.container.ownerDocument.querySelector('details')?.open).toBe(false);
  expect(view.getByText('导出文件未能保存。请重新选择 ZIP 文件的保存位置后再试。')).toBeTruthy();
  fireEvent.click(view.getByRole('button', { name: '重新选择保存位置' }));
  expect(calls).toEqual(['close', 'retry']);
});

test('inline notice offers retry and copies the diagnostic through the existing copy primitive', () => {
  const copy = spyOn(clipboard, 'copyText').mockResolvedValue(undefined);
  const toast = spyOn(Message, 'success').mockImplementation(() => () => {});
  restores.push(() => copy.mockRestore(), () => toast.mockRestore());
  const retry = mock(() => {});
  const error = normalizeKnowledgeError(new Error('failed to read tree'), 'load', i18n.t.bind(i18n));
  const view = render(<I18nextProvider i18n={i18n}>
    <KnowledgeErrorNotice error={error} onRetry={retry} />
  </I18nextProvider>);
  expect(view.getByRole('alert').textContent).toContain('知识库加载失败');
  fireEvent.click(view.getByRole('button', { name: '重试' }));
  expect(retry).toHaveBeenCalledTimes(1);
  const details = view.container.querySelector('details')!;
  details.open = true;
  fireEvent.click(view.getByRole('button', { name: '复制技术信息' }));
  expect(copy).toHaveBeenCalledWith('Error: failed to read tree');
});

test('handled authentication produces no competing knowledge toast or notice', () => {
  const error = { ...exportError(), handled: true };
  const toast = spyOn(Message, 'error').mockImplementation(() => () => {});
  restores.push(() => toast.mockRestore());
  showKnowledgeErrorMessage(error);
  const view = render(<I18nextProvider i18n={i18n}><KnowledgeErrorNotice error={error} /></I18nextProvider>);
  expect(toast).not.toHaveBeenCalled();
  expect(view.queryByRole('alert')).toBeNull();
});
