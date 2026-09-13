/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../test/setup-dom.ts';
import React from 'react';
import { afterEach, describe, expect, test } from 'bun:test';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import zh from '../../../services/i18n/locales/zh-CN/creativeStudio.json';
import ImageWorkbench from './image/ImageWorkbench';
import type { ImageWorkbenchProps } from './image/types';
import VideoWorkbench from './video/VideoWorkbench';
import type { VideoWorkbenchProps } from './video/types';
import ContentSiderTitlebarToggle from '@/renderer/components/layout/ContentSider/ContentSiderTitlebarToggle';
import { workbenchSiderChannels } from '@/renderer/utils/workspace/workbenchSiderEvents';
import { useWorkbenchLayout } from './useWorkbenchLayout';

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'zh-CN',
  resources: { 'zh-CN': { translation: { creativeStudio: zh } } },
});
const noop = () => undefined;
const imageProps: ImageWorkbenchProps = {
  layout: 'side', prompt: '保留的图片提示词', references: [],
  settings: { model: null, interfaceMode: 'images', quality: 'auto', width: 1024, height: 1024, aspectRatio: '1:1', count: 1 },
  modelOptions: [], results: [], selectedResultIds: [], task: { state: 'idle', pendingCount: 0 },
  onLayoutChange: noop, onPromptChange: noop, onRemoveReference: noop, onModelChange: noop,
  onInterfaceModeChange: noop, onQualityChange: noop, onAspectRatioChange: noop, onCountChange: noop,
  onGenerate: noop, onResultSelectionChange: noop, onDeleteResult: noop, onDeleteSelected: noop,
};
const videoProps: VideoWorkbenchProps = {
  layout: 'side', prompt: '保留的视频提示词', references: [], modelSlot: <span>Video model</span>,
  resolution: '1080p', resolutionOptions: [], size: '16:9', sizeOptions: [], duration: '5', durationOptions: [],
  taskCount: 1, tasks: [], selectedTaskIds: [], onSelectedTaskIdsChange: noop,
  onLayoutChange: noop, onPromptChange: noop, onGenerate: noop, onAddReferences: noop, onRemoveReference: noop,
  onResolutionChange: noop, onSizeChange: noop, onDurationChange: noop, onTaskCountChange: noop, onOpenParameters: noop,
};

type Kind = 'image' | 'video';
const widthKey = (kind: Kind) => `nomifun:${kind}-workbench-sider-width`;
const collapseKey = (kind: Kind) => `nomifun:${kind}-workbench-sider-collapsed`;
const heightKey = (kind: Kind) => `nomifun:${kind}-workbench-bottom-height`;
const WorkbenchView: React.FC<{ kind: Kind; layout?: 'side' | 'bottom' }> = ({ kind, layout = 'side' }) => (
  <I18nextProvider i18n={i18n}>
    <header>
      <ContentSiderTitlebarToggle
        channel={workbenchSiderChannels[kind]}
        expandLabel='展开生成设置'
        collapseLabel='收起生成设置'
        strokeWidth={2.5}
      />
    </header>
    {kind === 'image' ? <ImageWorkbench {...imageProps} layout={layout} /> : <VideoWorkbench {...videoProps} layout={layout} />}
  </I18nextProvider>
);
const renderWorkbench = (kind: Kind, layout: 'side' | 'bottom' = 'side') => render(<WorkbenchView kind={kind} layout={layout} />);

afterEach(() => {
  cleanup();
  for (const kind of ['image', 'video'] as const) {
    localStorage.removeItem(widthKey(kind));
    localStorage.removeItem(collapseKey(kind));
    localStorage.removeItem(heightKey(kind));
    localStorage.removeItem(`nomifun:${kind}-workbench-layout`);
  }
});

describe('shared composer reference interactions', () => {
  for (const kind of ['image', 'video'] as const) {
    test(`${kind}: add and remove references without clearing the prompt or showing a zero count`, () => {
      const ReferenceWorkbench = () => {
        const [references, setReferences] = React.useState<Array<{ id: string; kind: 'image'; name: string; previewUrl: string }>>([]);
        const add = () => setReferences([{ id: 'reference-1', kind: 'image', name: '参考图片', previewUrl: '/reference.png' }]);
        const remove = (id: string) => setReferences((items) => items.filter((item) => item.id !== id));
        return <I18nextProvider i18n={i18n}>
          {kind === 'image'
            ? <ImageWorkbench {...imageProps} references={references} onChooseReferences={add} onRemoveReference={remove} />
            : <VideoWorkbench {...videoProps} references={references} onAddReferences={add} addReferenceLabel='添加图片参考' onRemoveReference={remove} />}
        </I18nextProvider>;
      };
      const view = render(<ReferenceWorkbench />);
      expect(view.container.querySelector('[data-workbench-reference-count]')).toBeNull();
      fireEvent.click(view.getByRole('button', { name: '添加图片参考' }));
      expect(view.container.querySelector('[data-workbench-reference-count]')?.textContent).toBe('1 项');
      expect(view.getByRole('button', { name: '继续添加' })).toBeTruthy();
      fireEvent.click(view.getByRole('button', { name: /移除参考/ }));
      expect(view.container.querySelector('[data-workbench-reference-count]')).toBeNull();
      expect(view.container.querySelector('[data-reference-kind]')).toBeNull();
      expect(document.activeElement).toBe(view.getByRole('button', { name: '添加图片参考' }));
      expect(view.getByDisplayValue(kind === 'image' ? imageProps.prompt : videoProps.prompt)).toBeTruthy();
    });
  }
});

describe('workbench sidebar preferences', () => {
  for (const kind of ['image', 'video'] as const) {
    test(`${kind}: drag, reload, collapse and expand preserve width and prompt`, () => {
      let view = renderWorkbench(kind);
      const separator = view.getByRole('separator');
      const handle = separator.firstElementChild!;
      fireEvent.pointerDown(handle, { button: 0, buttons: 1, pointerId: 1, pointerType: 'mouse', clientX: 380 });
      fireEvent.pointerMove(window, { buttons: 1, pointerId: 1, clientX: 450 });
      fireEvent.pointerUp(window, { button: 0, buttons: 0, pointerId: 1, clientX: 450 });
      expect(view.container.querySelector<HTMLElement>('[data-workbench-sider]')?.style.width).toBe('450px');
      expect(localStorage.getItem(widthKey(kind))).toBe('450');

      const titlebarToggle = view.getByRole('button', { name: '收起生成设置' });
      expect(titlebarToggle.closest('header')).toBeTruthy();
      act(() => titlebarToggle.focus());
      fireEvent.click(titlebarToggle);
      expect(document.activeElement).toBe(view.getByRole('button', { name: '展开生成设置' }));
      expect(view.container.querySelector('[data-workbench-sider]')).toBeNull();
      expect(view.queryByRole('separator')).toBeNull();
      expect(view.queryByDisplayValue(kind === 'image' ? imageProps.prompt : videoProps.prompt)).toBeNull();
      expect(view.getByRole('heading', { name: kind === 'image' ? '全部结果' : '全部成果' })).toBeTruthy();
      view.unmount();

      view = renderWorkbench(kind);
      expect(view.container.querySelector('[data-workbench-sider]')).toBeNull();
      const expandToggle = view.getByRole('button', { name: '展开生成设置' });
      act(() => expandToggle.focus());
      fireEvent.click(expandToggle);
      expect(document.activeElement).toBe(view.getByRole('button', { name: '收起生成设置' }));
      expect(view.container.querySelector<HTMLElement>('[data-workbench-sider]')?.style.width).toBe('450px');
      expect(view.getByDisplayValue(kind === 'image' ? imageProps.prompt : videoProps.prompt)).toBeTruthy();

      fireEvent.doubleClick(view.getByRole('separator').firstElementChild!);
      expect(localStorage.getItem(widthKey(kind))).toBe('380');
      view.unmount();
      view = renderWorkbench(kind);
      expect(view.getByRole('separator').getAttribute('aria-valuenow')).toBe('380');
      expect(view.getByRole('button', { name: '收起生成设置' })).toBeTruthy();
    });
  }

  test('keeps image and video preferences independent, and bottom mode does not clear them', () => {
    localStorage.setItem(widthKey('image'), '440');
    localStorage.setItem(collapseKey('image'), 'collapsed');
    let view = renderWorkbench('video');
    expect(view.getByRole('separator').getAttribute('aria-valuenow')).toBe('380');
    fireEvent.keyDown(view.getByRole('separator'), { key: 'End' });
    expect(localStorage.getItem(widthKey('video'))).toBe('480');
    view.unmount();
    view = renderWorkbench('image', 'bottom');
    expect(view.queryByRole('separator')).toBeNull();
    expect(view.getByRole('button', { name: '展开生成设置' }).getAttribute('data-panel-position')).toBe('bottom');
    expect(localStorage.getItem(collapseKey('image'))).toBe('collapsed');
    view.unmount();
    view = renderWorkbench('image');
    fireEvent.click(view.getByRole('button', { name: '展开生成设置' }));
    expect(view.getByRole('separator').getAttribute('aria-valuenow')).toBe('440');
  });

  test('bounds resizing and recovers from invalid stored preferences', () => {
    localStorage.setItem(widthKey('image'), 'invalid');
    localStorage.setItem(collapseKey('image'), 'invalid');
    const view = renderWorkbench('image');
    const separator = view.getByRole('separator');
    expect(separator.getAttribute('aria-valuenow')).toBe('380');
    fireEvent.keyDown(separator, { key: 'Home' });
    fireEvent.keyDown(separator, { key: 'ArrowLeft' });
    expect(separator.getAttribute('aria-valuenow')).toBe('310');
    fireEvent.pointerDown(separator.firstElementChild!, { button: 0, buttons: 1, pointerId: 2, pointerType: 'mouse', clientX: 310 });
    fireEvent.pointerUp(window, { button: 0, buttons: 0, pointerId: 2, clientX: 1200 });
    expect(separator.getAttribute('aria-valuenow')).toBe('480');
    expect(document.body.style.cursor).toBe('');
  });

  test('keeps the same titlebar entry in both positions and restores each workbench state', () => {
    const view = renderWorkbench('image');
    fireEvent.click(view.getByRole('button', { name: '收起生成设置' }));
    view.rerender(<WorkbenchView kind='image' layout='bottom' />);
    expect(view.getByRole('button', { name: '展开生成设置' }).getAttribute('data-panel-position')).toBe('bottom');
    view.rerender(<WorkbenchView kind='image' />);
    expect(view.getByRole('button', { name: '展开生成设置' }).getAttribute('aria-expanded')).toBe('false');
    expect(view.container.querySelector('[data-workbench-sider]')).toBeNull();
    view.rerender(<WorkbenchView kind='video' />);
    expect(view.getByRole('button', { name: '收起生成设置' }).getAttribute('aria-expanded')).toBe('true');
    expect(view.container.querySelector('[data-workbench-sider="video"]')).toBeTruthy();
  });
});

describe('bottom panel preferences', () => {
  for (const kind of ['image', 'video'] as const) {
    test(`${kind}: fits content by default and preserves a resized height across collapse and reload`, () => {
      let view = renderWorkbench(kind, 'bottom');
      let panel = view.container.querySelector<HTMLElement>('[data-workbench-panel]')!;
      expect(panel.style.height).toBe('');
      expect(localStorage.getItem(heightKey(kind))).toBeNull();
      panel.getBoundingClientRect = () => ({ height: 200 } as DOMRect);
      const separator = view.getByRole('separator');
      expect(separator.getAttribute('aria-orientation')).toBe('horizontal');
      fireEvent.pointerDown(separator, { button: 0, buttons: 1, pointerId: 3, clientY: 600 });
      fireEvent.pointerMove(window, { buttons: 1, pointerId: 3, clientY: 450 });
      expect(localStorage.getItem(heightKey(kind))).toBeNull();
      fireEvent.pointerUp(window, { button: 0, buttons: 0, pointerId: 3, clientY: 450 });
      expect(panel.style.height).toBe('300px');
      expect(localStorage.getItem(heightKey(kind))).toBe('300');
      expect(document.body.style.cursor).toBe('');
      fireEvent.click(view.getByRole('button', { name: '收起生成设置' }));
      expect(view.container.querySelector('[data-workbench-panel]') === null).toBe(true);
      view.unmount();
      view = renderWorkbench(kind, 'bottom');
      fireEvent.click(view.getByRole('button', { name: '展开生成设置' }));
      panel = view.container.querySelector<HTMLElement>('[data-workbench-panel]')!;
      expect(panel.style.height).toBe('300px');
      expect(view.getByDisplayValue(kind === 'image' ? imageProps.prompt : videoProps.prompt) !== null).toBe(true);
      fireEvent.doubleClick(view.getByRole('separator'));
      expect(panel.style.height).toBe('');
      expect(localStorage.getItem(heightKey(kind))).toBeNull();
      fireEvent.keyDown(view.getByRole('separator'), { key: 'End' });
      expect(panel.style.height).toBe('300px');
      fireEvent.keyDown(view.getByRole('separator'), { key: 'Home' });
      expect(panel.style.height).toBe('');
    });
  }

  test('clamps legacy saved heights to the smaller maximum and available space without overwriting them', () => {
    const NativeObserver = globalThis.ResizeObserver;
    let measure = () => undefined;
    globalThis.ResizeObserver = class {
      constructor(callback: () => undefined) { measure = callback; }
      observe() {}
      unobserve() {}
      disconnect() {}
    } as unknown as typeof ResizeObserver;
    try {
      localStorage.setItem(heightKey('video'), '400');
      const view = renderWorkbench('video', 'bottom');
      const panel = view.container.querySelector<HTMLElement>('[data-workbench-panel]')!;
      const content = panel.firstElementChild!.firstElementChild!;
      let available = 400;
      let natural = 199;
      Object.defineProperty(panel.parentElement, 'clientHeight', { configurable: true, get: () => available });
      content.getBoundingClientRect = () => ({ height: natural } as DOMRect);
      act(() => measure());
      expect(panel.style.height).toBe('260px');
      expect(localStorage.getItem(heightKey('video'))).toBe('400');
      available = 800;
      act(() => measure());
      expect(panel.style.height).toBe('300px');
      natural = 420;
      act(() => measure());
      expect(panel.style.height).toBe('300px');
      expect(view.getByRole('separator').getAttribute('aria-valuemin')).toBe('300');
      expect(localStorage.getItem(heightKey('video'))).toBe('400');
      fireEvent.doubleClick(view.getByRole('separator'));
      expect(panel.style.height).toBe('');
      view.unmount();
    } finally {
      globalThis.ResizeObserver = NativeObserver;
    }
  });

  test('cleans up a drag when collapsing or unmounting, and ignores invalid stored heights', () => {
    localStorage.setItem(heightKey('image'), 'Infinity');
    const view = renderWorkbench('image', 'bottom');
    expect(view.container.querySelector<HTMLElement>('[data-workbench-panel]')!.style.height).toBe('');
    fireEvent.pointerDown(view.getByRole('separator'), { button: 0, buttons: 1, pointerId: 9, clientY: 300 });
    expect(document.body.style.cursor).toBe('row-resize');
    fireEvent.pointerCancel(window, { pointerId: 9 });
    expect(document.body.style.cursor).toBe('');
    fireEvent.pointerDown(view.getByRole('separator'), { button: 0, buttons: 1, pointerId: 9, clientY: 300 });
    view.unmount();
    expect(document.body.style.cursor).toBe('');
    expect(document.body.style.userSelect).toBe('');
    fireEvent.pointerUp(window, { pointerId: 9, clientY: 100 });
    expect(localStorage.getItem(heightKey('image'))).toBe('Infinity');
  });

  test('persists image and video placement separately from session drafts', () => {
    const Placement: React.FC<{ kind: Kind }> = ({ kind }) => {
      const [layout, setLayout] = useWorkbenchLayout(kind, 'side');
      return <button onClick={() => setLayout(layout === 'side' ? 'bottom' : 'side')}>{layout}</button>;
    };
    let view = render(<Placement kind='image' />);
    fireEvent.click(view.getByRole('button', { name: 'side' }));
    expect(localStorage.getItem('nomifun:image-workbench-layout')).toBe('bottom');
    view.unmount();
    view = render(<Placement kind='video' />);
    expect(view.getByRole('button').textContent).toBe('side');
    view.unmount();
    view = render(<Placement kind='image' />);
    expect(view.getByRole('button').textContent).toBe('bottom');
  });
});
