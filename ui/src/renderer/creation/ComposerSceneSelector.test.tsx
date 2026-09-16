import '../../../test/setup-dom.ts';
import { act, cleanup, fireEvent, render, within } from '@testing-library/react';
import { afterEach, expect, test } from 'bun:test';
import { useState } from 'react';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import messages from '@/renderer/services/i18n/locales/zh-CN/creation.json';
import { CreationComposerContext } from './CreationComposerContext';
import { ComposerSceneHeader, SceneDiscoveryHint } from './ComposerSceneSelector';
import { useCreationDraft } from './useCreationDraft';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: { creation: messages } } } });
afterEach(() => { cleanup(); localStorage.clear(); sessionStorage.clear(); });

function mount({ preparing = false } = {}) {
  let current!: ReturnType<typeof useCreationDraft>;
  let switches = 0;
  function Harness() {
    const creation = useCreationDraft('scene-picker-test');
    const [prompt, setPrompt] = useState('雨后的森林小屋');
    current = creation;
    return <CreationComposerContext.Provider value={{ ...creation, preparing,
      selectMode: mode => { switches++; creation.setMode(mode); },
      exit: () => { switches++; creation.setMode(null); },
    }}>
      <ComposerSceneHeader agent={<button type='button'>最简问答</button>} />
      <textarea aria-label='描述' value={prompt} onChange={event => setPrompt(event.target.value)} />
      <SceneDiscoveryHint />
    </CreationComposerContext.Provider>;
  }
  return { page: render(<I18nextProvider i18n={i18n}><Harness /></I18nextProvider>), getDraft: () => current, switches: () => switches };
}

test('four direct scene buttons preserve the message, references and generation settings', () => {
  const { page, getDraft, switches } = mount();
  const group = page.getByTestId('composer-scene-selector');
  expect(within(group).getAllByRole('button')).toHaveLength(4);
  expect(page.queryByRole('menu')).toBeNull();
  fireEvent.click(within(group).getByRole('button', { name: '日常对话' }));
  expect(switches()).toBe(1);
  act(() => getDraft().update(draft => ({ ...draft, parameters: { ...draft.parameters, image: { count: 3, size: '1024x1024' } }, references: [{ asset_id: 'one', kind: 'image', role: 'reference', title: '参考图片' }] })));
  for (const [label, mode] of [['图像创作', 'image'], ['视频创作', 'video'], ['音乐创作', 'music'], ['日常对话', null]] as const) {
    const button = within(group).getByRole('button', { name: label });
    fireEvent.click(button);
    expect(page.getByTestId('composer-scene-selector')).toBe(group);
    expect(button.getAttribute('aria-pressed')).toBe('true');
    expect(group.querySelectorAll('[aria-pressed="true"]')).toHaveLength(1);
    expect(getDraft().draft.mode).toBe(mode);
    expect((page.getByRole('textbox', { name: '描述' }) as HTMLTextAreaElement).value).toBe('雨后的森林小屋');
    expect(getDraft().draft.parameters.image).toEqual({ count: 3, size: '1024x1024' });
    expect(getDraft().draft.references).toHaveLength(1);
  }
});

test('arrow keys focus buttons without switching until activated', () => {
  const { page, getDraft, switches } = mount();
  const chat = page.getByRole('button', { name: '日常对话' });
  const image = page.getByRole('button', { name: '图像创作' });
  chat.focus();
  fireEvent.keyDown(chat, { key: 'ArrowRight' });
  expect(document.activeElement).toBe(image);
  expect(switches()).toBe(0);
  fireEvent.click(image);
  expect(getDraft().draft.mode).toBe('image');
  fireEvent.keyDown(image, { key: 'End' });
  expect(document.activeElement).toBe(page.getByRole('button', { name: '音乐创作' }));
  fireEvent.keyDown(document.activeElement!, { key: 'ArrowRight' });
  expect(document.activeElement).toBe(chat);
});

test('hover does not open a popup, take input focus, or switch the Agent', () => {
  const { page, switches } = mount();
  const input = page.getByRole('textbox', { name: '描述' });
  input.focus();
  fireEvent.mouseEnter(page.getByRole('button', { name: '图像创作' }));
  expect(page.queryByRole('menu')).toBeNull();
  expect(document.activeElement).toBe(input);
  expect(switches()).toBe(0);
});

test('the discovery hint can be dismissed and stays dismissed after remount', () => {
  const { page } = mount();
  expect(page.getByText('也可以创作图像、视频和音乐')).toBeTruthy();
  fireEvent.click(page.getByRole('button', { name: '不再显示场景提示' }));
  expect(page.queryByText('也可以创作图像、视频和音乐')).toBeNull();
  page.unmount();
  expect(mount().page.queryByText('也可以创作图像、视频和音乐')).toBeNull();
});

test('preparing an Agent disables scene changes', () => {
  const { page, switches } = mount({ preparing: true });
  const group = page.getByTestId('composer-scene-selector');
  for (const button of within(group).getAllByRole('button')) {
    expect((button as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(button);
  }
  expect(switches()).toBe(0);
});
