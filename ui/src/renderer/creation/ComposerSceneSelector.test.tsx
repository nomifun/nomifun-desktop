import '../../../test/setup-dom.ts';
import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
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

test('all four scenes remain accessible through one persistent trigger and preserve draft state', async () => {
  const { page, getDraft, switches } = mount();
  const trigger = page.getByTestId('composer-scene-selector');
  fireEvent.click(trigger);
  expect(within(document.body).getAllByRole('menuitemradio')).toHaveLength(4);
  fireEvent.click(within(document.body).getByRole('menuitemradio', { name: /日常对话/ }));
  expect(switches()).toBe(0);
  act(() => getDraft().update(draft => ({ ...draft, parameters: { ...draft.parameters, image: { count: 3, size: '1024x1024' } }, references: [{ asset_id: 'one', kind: 'image', role: 'reference', title: '参考图片' }] })));
  for (const [label, mode] of [['图像创作', 'image'], ['视频创作', 'video'], ['音乐创作', 'music'], ['日常对话', null]] as const) {
    fireEvent.click(trigger);
    const menu = within(document.body).getByRole('menu');
    expect(within(menu).getAllByRole('menuitemradio')).toHaveLength(4);
    fireEvent.click(within(menu).getByRole('menuitemradio', { name: new RegExp(label) }));
    expect(page.getByTestId('composer-scene-selector')).toBe(trigger);
    expect(trigger.getAttribute('aria-label')).toContain(label);
    expect(trigger.getAttribute('aria-expanded')).toBe('false');
    expect(getDraft().draft.mode).toBe(mode);
    expect((page.getByRole('textbox', { name: '描述' }) as HTMLTextAreaElement).value).toBe('雨后的森林小屋');
    expect(getDraft().draft.parameters.image).toEqual({ count: 3, size: '1024x1024' });
    expect(getDraft().draft.references).toHaveLength(1);
  }
  await waitFor(() => expect(document.activeElement).toBe(trigger));
});

test('keyboard navigation selects a scene and Escape restores trigger focus', async () => {
  const { page, getDraft } = mount();
  const trigger = page.getByTestId('composer-scene-selector');
  trigger.focus();
  fireEvent.keyDown(trigger, { key: 'ArrowDown' });
  const chat = await within(document.body).findByRole('menuitemradio', { name: /日常对话/ });
  await waitFor(() => expect(document.activeElement).toBe(chat));
  fireEvent.keyDown(chat, { key: 'ArrowRight' });
  const image = within(document.body).getByRole('menuitemradio', { name: /图像创作/ });
  await waitFor(() => expect(document.activeElement).toBe(image));
  fireEvent.click(image);
  expect(getDraft().draft.mode).toBe('image');
  fireEvent.click(trigger);
  fireEvent.keyDown(within(document.body).getByRole('menu'), { key: 'Escape' });
  await waitFor(() => expect(document.activeElement).toBe(trigger));
  expect(within(document.body).queryByRole('menu')).toBeNull();
});

test('hover previews the menu without taking focus from an unfinished message or changing scenes', async () => {
  const { page, switches } = mount();
  const input = page.getByRole('textbox', { name: '描述' });
  input.focus();
  fireEvent.mouseEnter(page.getByTestId('composer-scene-selector'));
  await within(document.body).findByRole('menu');
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 250)); });
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
  const trigger = page.getByTestId('composer-scene-selector') as HTMLButtonElement;
  expect(trigger.disabled).toBe(true);
  fireEvent.click(trigger);
  expect(within(document.body).queryByRole('menu')).toBeNull();
  expect(switches()).toBe(0);
});
