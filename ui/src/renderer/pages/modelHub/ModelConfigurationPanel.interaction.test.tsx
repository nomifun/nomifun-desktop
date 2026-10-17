/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, expect, mock, test } from 'bun:test';
import { useState } from 'react';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import ModelConfigurationPanel, { ModelConfigurationWorkspace } from './ModelConfigurationPanel';

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'zh-CN',
  resources: { 'zh-CN': { translation: { settings: { modelConfiguration: { back: '返回列表' } } } } },
});

afterEach(cleanup);

function Overview({ onOpen }: { onOpen: () => void }) {
  const [filter, setFilter] = useState('');
  const [selected, setSelected] = useState('first');
  return <>
    <label>筛选模型<input value={filter} onChange={(event) => setFilter(event.target.value)} /></label>
    <label>已选模型<select value={selected} onChange={(event) => setSelected(event.target.value)}>
      <option value='first'>第一个模型</option>
      <option value='second'>第二个模型</option>
    </select></label>
    <button onClick={onOpen}>配置模型</button>
  </>;
}

function Fixture({ busy = false, onBack, onEditingChange }: {
  busy?: boolean;
  onBack: () => void;
  onEditingChange: (editing: boolean) => void;
}) {
  const [visible, setVisible] = useState(false);
  const [draft, setDraft] = useState('');
  return <I18nextProvider i18n={i18n}>
    <button>页面外部操作</button>
    <ModelConfigurationWorkspace onEditingChange={onEditingChange}>
      <Overview onOpen={() => setVisible(true)} />
      <ModelConfigurationPanel visible={visible} title='编辑模型' subtitle='填写模型连接信息'
        busy={busy} onBack={() => { onBack(); setVisible(false); }}
        footer={<button>保存配置</button>}>
        <label>模型标识<input value={draft} onChange={(event) => setDraft(event.target.value)} /></label>
      </ModelConfigurationPanel>
    </ModelConfigurationWorkspace>
  </I18nextProvider>;
}

function mount(busy = false) {
  const onBack = mock(() => {});
  const onEditingChange = mock((_editing: boolean) => {});
  const page = render(<Fixture busy={busy} onBack={onBack} onEditingChange={onEditingChange} />);
  const open = () => {
    const trigger = page.getByRole('button', { name: '配置模型' });
    trigger.focus();
    fireEvent.click(trigger);
    return trigger;
  };
  return { page, open, onBack, onEditingChange };
}

test('opening configuration preserves the mounted overview and gives the form its own page body and footer', async () => {
  const { page, open, onEditingChange } = mount();
  const filter = page.getByRole('textbox', { name: '筛选模型' }) as HTMLInputElement;
  const selection = page.getByRole('combobox', { name: '已选模型' }) as HTMLSelectElement;
  fireEvent.change(filter, { target: { value: 'my-provider' } });
  fireEvent.change(selection, { target: { value: 'second' } });
  const overview = filter.parentElement!.parentElement as HTMLDivElement;
  overview.scrollTop = 240;

  open();

  expect(overview.hidden).toBe(true);
  expect(filter.isConnected).toBe(true);
  expect(filter.value).toBe('my-provider');
  expect(selection.value).toBe('second');
  expect(overview.scrollTop).toBe(240);
  expect(onEditingChange.mock.calls.map(([editing]) => editing)).toEqual([false, true]);
  expect(page.queryByRole('button', { name: '配置模型' })).toBeNull();
  const editor = page.getByRole('region', { name: '编辑模型' });
  expect(editor.parentElement).not.toBe(overview);
  const heading = page.getByRole('heading', { name: '编辑模型' });
  await waitFor(() => expect(document.activeElement === heading).toBe(true));
  const body = editor.querySelector('[data-model-configuration-body]');
  expect(body?.contains(page.getByRole('textbox', { name: '模型标识' }))).toBe(true);
  const footer = editor.querySelector('footer');
  expect(footer?.contains(page.getByRole('button', { name: '保存配置' }))).toBe(true);
  expect(body?.contains(footer)).toBe(false);
  expect(page.queryByRole('dialog')).toBeNull();
  expect(page.baseElement.querySelector('[aria-modal="true"], .arco-modal-mask, .arco-drawer-mask')).toBeNull();
});

test('outside clicks and Escape leave the open form and draft intact', () => {
  const { page, open, onBack } = mount();
  open();
  const input = page.getByRole('textbox', { name: '模型标识' }) as HTMLInputElement;
  fireEvent.change(input, { target: { value: 'lowercase-model-draft' } });

  fireEvent.mouseDown(document.body);
  fireEvent.click(page.getByRole('button', { name: '页面外部操作' }));
  fireEvent.keyDown(input, { key: 'Escape', code: 'Escape' });
  fireEvent.keyDown(document, { key: 'Escape', code: 'Escape' });

  expect(page.getByRole('region', { name: '编辑模型' })).toBeTruthy();
  expect(page.getByRole('textbox', { name: '模型标识' })).toBe(input);
  expect(input.value).toBe('lowercase-model-draft');
  expect(onBack).toHaveBeenCalledTimes(0);
});

test('explicit back restores the overview state, scroll, and opening button focus', async () => {
  const { page, open, onBack, onEditingChange } = mount();
  const filter = page.getByRole('textbox', { name: '筛选模型' }) as HTMLInputElement;
  const selection = page.getByRole('combobox', { name: '已选模型' }) as HTMLSelectElement;
  fireEvent.change(filter, { target: { value: 'unchanged-filter' } });
  fireEvent.change(selection, { target: { value: 'second' } });
  const overview = filter.parentElement!.parentElement as HTMLDivElement;
  overview.scrollTop = 420;
  const trigger = open();

  fireEvent.click(page.getByRole('button', { name: '返回列表' }));

  expect(page.queryByRole('region', { name: '编辑模型' })).toBeNull();
  expect(overview.hidden).toBe(false);
  expect(page.getByRole('textbox', { name: '筛选模型' })).toBe(filter);
  expect(filter.value).toBe('unchanged-filter');
  expect(selection.value).toBe('second');
  expect(overview.scrollTop).toBe(420);
  expect(onBack).toHaveBeenCalledTimes(1);
  expect(onEditingChange.mock.calls.map(([editing]) => editing)).toEqual([false, true, false]);
  await waitFor(() => expect(document.activeElement === trigger).toBe(true));
});

test('busy configuration disables back navigation while preserving the draft', () => {
  const { page, open, onBack } = mount(true);
  open();
  const input = page.getByRole('textbox', { name: '模型标识' }) as HTMLInputElement;
  fireEvent.change(input, { target: { value: 'saving-model' } });
  const back = page.getByRole('button', { name: '返回列表' }) as HTMLButtonElement;

  expect(back.disabled).toBe(true);
  fireEvent.click(back);

  expect(page.getByRole('region', { name: '编辑模型' })).toBeTruthy();
  expect(input.value).toBe('saving-model');
  expect(onBack).toHaveBeenCalledTimes(0);
});
