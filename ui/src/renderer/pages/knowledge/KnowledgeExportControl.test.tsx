import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, spyOn, test } from 'bun:test';
import { Message } from '@arco-design/web-react';
import i18next from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { ipcBridge } from '@/common';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import * as runtime from '@/common/adapter/tauriRuntime';
import type { IKnowledgeBase } from '@/common/adapter/ipcBridge';
import { parseKnowledgeBaseId } from '@/common/types/ids';
import * as theme from '@/renderer/hooks/context/ThemeContext';
import knowledge from '@/renderer/services/i18n/locales/zh-CN/knowledge.json';
import common from '@/renderer/services/i18n/locales/zh-CN/common.json';
import * as selection from './knowledgeExport';
import KnowledgeExportControl from './KnowledgeExportControl';

const restore: Array<() => void> = [];
const locale = i18next.createInstance();
const base: IKnowledgeBase = {
  knowledge_base_id: parseKnowledgeBaseId('0190f5fe-7c00-7a00-8000-000000000991'),
  name: '产品资料', description: '', root_path: '/knowledge/product', managed: true,
  tree_access: 'editable', created_at: 1, updated_at: 2, file_count: 1,
  total_size: 12, root_exists: true, tags: [], kind: 'blank',
};

beforeEach(async () => {
  await locale.init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: { knowledge, common } } } });
  const context = spyOn(theme, 'useThemeContext').mockReturnValue({
    theme: 'light', colorScheme: 'default', fontScale: 1,
    setTheme: async () => {}, setColorScheme: async () => {}, setFontScale: async () => {},
  } as ReturnType<typeof theme.useThemeContext>);
  restore.push(() => context.mockRestore());
});

afterEach(() => {
  cleanup();
  restore.splice(0).reverse().forEach((dispose) => dispose());
});

function mount() {
  return render(<I18nextProvider i18n={locale}><KnowledgeExportControl base={base} /></I18nextProvider>);
}

function mockRuntime(native: boolean) {
  const mock = spyOn(runtime, 'isTauriRuntime').mockReturnValue(native);
  restore.push(() => mock.mockRestore());
}

function mockExport() {
  const invoke = spyOn(ipcBridge.knowledge.exportBase, 'invoke').mockResolvedValue({ dest_path: 'C:\\Exports\\资料.zip' });
  const success = spyOn(Message, 'success').mockImplementation(() => () => {});
  const failure = spyOn(Message, 'error').mockImplementation(() => () => {});
  restore.push(() => invoke.mockRestore(), () => success.mockRestore(), () => failure.mockRestore());
  return { invoke, success, failure };
}

describe('knowledge export feedback', () => {
  test('blocks repeated clicks while selecting; cancellation sends no export or error', async () => {
    mockRuntime(true);
    let resolve!: (path: string | null) => void;
    const picker = spyOn(selection, 'pickKnowledgeExportFile').mockImplementation(() => new Promise((done) => { resolve = done; }));
    restore.push(() => picker.mockRestore());
    const { invoke, success, failure } = mockExport();
    const view = mount();
    const button = view.getByRole('button', { name: '导出' });
    fireEvent.click(button);
    fireEvent.click(button);
    expect(picker).toHaveBeenCalledTimes(1);
    await act(async () => resolve(null));
    expect(invoke).not.toHaveBeenCalled();
    expect(success).not.toHaveBeenCalled();
    expect(failure).not.toHaveBeenCalled();
    expect(view.queryByRole('dialog')).toBeNull();
  });

  test('native save exports the complete ZIP path and only shows a success bubble', async () => {
    mockRuntime(true);
    const picker = spyOn(selection, 'pickKnowledgeExportFile').mockResolvedValue('C:\\Exports\\资料.zip');
    restore.push(() => picker.mockRestore());
    const { invoke, success, failure } = mockExport();
    const view = mount();
    fireEvent.click(view.getByRole('button', { name: '导出' }));
    await waitFor(() => expect(success).toHaveBeenCalledWith('资料导出成功'));
    expect(invoke).toHaveBeenCalledWith({ knowledge_base_id: base.knowledge_base_id, dest_path: 'C:\\Exports\\资料.zip', overwrite: true });
    expect(failure).not.toHaveBeenCalled();
    expect(view.queryByRole('dialog')).toBeNull();
  });

  test('export failure opens a modal with collapsed technical information and no error bubble', async () => {
    mockRuntime(true);
    const picker = spyOn(selection, 'pickKnowledgeExportFile').mockResolvedValue('C:\\Exports\\资料.zip');
    restore.push(() => picker.mockRestore());
    const { invoke, success, failure } = mockExport();
    invoke.mockRejectedValue(new BackendHttpError({ method: 'POST', path: '/api/knowledge/bases/test/export', status: 500,
      body: { code: 'KNOWLEDGE_EXPORT_ACCESS_DENIED', error: 'Access denied (os error 5)', details: { stage: 'finalize_file' } },
    }));
    const view = mount();
    fireEvent.click(view.getByRole('button', { name: '导出' }));
    await waitFor(() => expect(view.getByText('资料导出失败')).toBeTruthy());
    expect(view.getByRole('button', { name: '重新选择保存位置' })).toBeTruthy();
    expect(document.querySelector('details')?.open).toBe(false);
    expect(failure).not.toHaveBeenCalled();
    expect(success).not.toHaveBeenCalled();
  });

  test('picker exceptions also use the failure modal', async () => {
    mockRuntime(true);
    const picker = spyOn(selection, 'pickKnowledgeExportFile').mockRejectedValue(new Error('native picker failed'));
    restore.push(() => picker.mockRestore());
    const { invoke, failure } = mockExport();
    const view = mount();
    fireEvent.click(view.getByRole('button', { name: '导出' }));
    await waitFor(() => expect(view.getByText('资料导出失败')).toBeTruthy());
    expect(invoke).not.toHaveBeenCalled();
    expect(failure).not.toHaveBeenCalled();
  });

  test('WebUI appends a ZIP filename and replaces an existing archive only after confirmation', async () => {
    mockRuntime(false);
    const picker = spyOn(ipcBridge.dialog.showOpen, 'invoke').mockResolvedValue(['C:\\Exports']);
    restore.push(() => picker.mockRestore());
    const { invoke, success } = mockExport();
    invoke.mockRejectedValueOnce(new BackendHttpError({ method: 'POST', path: '/api/knowledge/bases/test/export', status: 409,
      body: { code: 'KNOWLEDGE_EXPORT_DESTINATION_EXISTS', error: 'File exists' },
    }));
    const view = mount();
    fireEvent.click(view.getByRole('button', { name: '导出' }));
    const name = await view.findByRole('textbox', { name: '文件名' });
    fireEvent.change(name, { target: { value: '资料.zip' } });
    fireEvent.click(view.getAllByRole('button', { name: '导出' }).find((button) => button.closest('[role="dialog"]'))!);
    await waitFor(() => expect(view.getByText('替换已有资料包？')).toBeTruthy());
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke.mock.calls[0][0]).toEqual({ knowledge_base_id: base.knowledge_base_id, dest_path: 'C:\\Exports\\资料.zip', overwrite: false });
    fireEvent.click(view.getByRole('button', { name: '替换文件' }));
    await waitFor(() => expect(success).toHaveBeenCalledWith('资料导出成功'));
    expect(invoke.mock.calls[1][0]).toEqual({ knowledge_base_id: base.knowledge_base_id, dest_path: 'C:\\Exports\\资料.zip', overwrite: true });
  });
});
