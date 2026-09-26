/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../../test/setup-dom.ts';

import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { withCanvasTestI18n } from '../components/canvasI18nTestUtils';
import CreativeTimelineNode, {
  type CreativeTimelineAssetPresentation,
} from './CreativeTimelineNode';
import { downloadTimelineComposition } from './timelineExport';
import type { CreativeNodeOfKind } from './types';

const timelineNode = (): CreativeNodeOfKind<'timeline'> => ({
  id: 'timeline-1',
  type: 'timeline',
  position: { x: 10, y: 20 },
  size: { width: 680, height: 148 },
  groupId: null,
  zIndex: 1,
  locked: false,
  data: {
    title: '时间线1',
    muted: false,
    clips: [{
      id: 'clip-1',
      assetId: 'asset-image',
      kind: 'image',
      startMs: 0,
      durationMs: 5_000,
      sourceStartMs: 0,
      sourceDurationMs: null,
    }],
  },
});

const assets = new Map<string, CreativeTimelineAssetPresentation>([[
  'asset-image',
  {
    assetId: 'asset-image',
    kind: 'image',
    title: '城市航拍',
    src: '/assets/city.png',
    thumbnailSrc: '/assets/city-thumb.png',
    width: 1_200,
    height: 800,
  },
]]);

afterEach(cleanup);

describe('CreativeTimelineNode interactions', () => {
  test('renders real clip media, duration, controls, and a movable playhead', () => {
    const view = render(withCanvasTestI18n(
      <CreativeTimelineNode node={timelineNode()} assets={assets} placement='contained' />
    ));

    expect(view.container.querySelector('[data-timeline-node]')).not.toBeNull();
    expect(view.container.querySelector('[data-timeline-clip-id="clip-1"]')).not.toBeNull();
    expect(view.container.querySelector('img[src="/assets/city-thumb.png"]')).not.toBeNull();
    expect((view.getByRole('button', { name: '播放' }) as HTMLButtonElement).disabled).toBe(false);
    expect(view.getByRole('slider', { name: '播放头' })).not.toBeNull();
    expect(
      view.container.querySelector<HTMLElement>('[data-project-canvas]')?.style
        .getPropertyValue('--timeline-project-aspect')
    ).toBe('1.5');
    expect(view.getAllByText('00:05').length).toBeGreaterThan(0);
    expect(view.getByText('01:00')).not.toBeNull();
  });

  test('persists mute changes and pointer-based clip arrangement', () => {
    const changes: Array<{ startMs: number; muted: boolean; mergeKey?: string }> = [];
    const node = timelineNode();
    const view = render(withCanvasTestI18n(
      <CreativeTimelineNode
        node={node}
        assets={assets}
        placement='contained'
        onChange={(data, mergeKey) => changes.push({
          startMs: data.clips[0]?.startMs ?? -1,
          muted: data.muted,
          mergeKey,
        })}
      />
    ));

    fireEvent.click(view.getByRole('button', { name: '关闭声音' }));
    expect(changes.at(-1)).toMatchObject({ muted: true, startMs: 0 });

    const track = view.container.querySelector<HTMLElement>('[data-timeline-track]');
    const clip = view.container.querySelector<HTMLElement>('[data-timeline-clip-id="clip-1"]');
    if (!track || !clip) throw new Error('timeline track fixture missing');
    track.getBoundingClientRect = () => ({
      x: 0,
      y: 0,
      top: 0,
      right: 600,
      bottom: 60,
      left: 0,
      width: 600,
      height: 60,
      toJSON: () => ({}),
    });
    clip.setPointerCapture = () => undefined;
    clip.hasPointerCapture = () => false;
    fireEvent.pointerDown(clip, { button: 0, pointerId: 4, clientX: 100 });
    fireEvent.pointerMove(clip, { pointerId: 4, clientX: 160 });
    fireEvent.pointerUp(clip, { pointerId: 4, clientX: 160 });

    expect(changes.at(-1)?.startMs).toBe(6_000);
    expect(changes.at(-1)?.mergeKey).toContain('timeline:timeline-1:clip-1:move');
  });

  test('seeks the playhead anywhere across the visible 60-second track', () => {
    const view = render(withCanvasTestI18n(
      <CreativeTimelineNode node={timelineNode()} assets={assets} placement='contained' />
    ));
    const track = view.container.querySelector<HTMLElement>('[data-timeline-track]');
    if (!track) throw new Error('timeline track fixture missing');
    track.getBoundingClientRect = () => ({
      x: 0,
      y: 0,
      top: 0,
      right: 600,
      bottom: 60,
      left: 0,
      width: 600,
      height: 60,
      toJSON: () => ({}),
    });
    fireEvent.pointerDown(track, { button: 0, pointerId: 9, clientX: 300 });
    expect(view.getByRole('slider', { name: '播放头' }).getAttribute('aria-valuenow')).toBe('30');
    expect(view.getAllByText('00:30').length).toBeGreaterThan(1);
  });

  test('opens the real asset picker callback and accepts dropped image/video files', () => {
    let requested = 0;
    let dropped: readonly File[] = [];
    const view = render(withCanvasTestI18n(
      <CreativeTimelineNode
        node={{ ...timelineNode(), data: { ...timelineNode().data, clips: [] } }}
        assets={new Map()}
        placement='contained'
        onRequestAssets={() => { requested += 1; }}
        onUploadFiles={(files) => { dropped = files; }}
      />
    ));

    fireEvent.click(view.getByRole('button', { name: '添加素材到时间线' }));
    fireEvent.click(view.getByRole('menuitem', { name: '从资产库添加' }));
    expect(requested).toBe(1);

    const root = view.container.querySelector<HTMLElement>('[data-timeline-node]');
    if (!root) throw new Error('timeline root fixture missing');
    const image = new File(['image'], 'scene.png', { type: 'image/png' });
    const ignored = new File(['text'], 'notes.txt', { type: 'text/plain' });
    fireEvent.drop(root, {
      dataTransfer: {
        types: ['Files'],
        files: [image, ignored],
      },
    });
    expect(dropped.map((file) => file.name)).toEqual(['scene.png']);
  });

  test('passes the fullscreen root as the asset-dialog portal container', () => {
    const descriptor = Object.getOwnPropertyDescriptor(document, 'fullscreenElement');
    let popupContainer: HTMLElement | null = null;
    const view = render(withCanvasTestI18n(
      <CreativeTimelineNode
        node={{ ...timelineNode(), data: { ...timelineNode().data, clips: [] } }}
        assets={new Map()}
        placement='contained'
        onRequestAssets={(container) => { popupContainer = container; }}
      />
    ));
    const root = view.container.querySelector<HTMLElement>('[data-timeline-node]');
    if (!root) throw new Error('timeline root fixture missing');
    Object.defineProperty(document, 'fullscreenElement', {
      configurable: true,
      value: root,
    });
    try {
      fireEvent.click(view.getByRole('button', { name: '添加素材到时间线' }));
      fireEvent.click(view.getByRole('menuitem', { name: '从资产库添加' }));
      expect(popupContainer).toBe(root);
    } finally {
      if (descriptor) Object.defineProperty(document, 'fullscreenElement', descriptor);
      else delete (document as unknown as Record<string, unknown>).fullscreenElement;
    }
  });

  test('adds an asset-library item by click and accepts it by drag payload', () => {
    const added: string[] = [];
    const view = render(withCanvasTestI18n(
      <CreativeTimelineNode
        node={timelineNode()}
        assets={assets}
        libraryAssets={[...assets.values()]}
        placement='contained'
        onAddAsset={(assetId) => added.push(assetId)}
        onRequestAssets={() => undefined}
      />
    ));

    fireEvent.click(view.getByRole('button', { name: '添加素材 城市航拍 到时间线' }));
    expect(added).toEqual(['asset-image']);

    const root = view.container.querySelector<HTMLElement>('[data-timeline-node]');
    if (!root) throw new Error('timeline root fixture missing');
    fireEvent.drop(root, {
      dataTransfer: {
        types: ['application/x-nomifun-timeline-asset'],
        getData: () => 'asset-image',
      },
    });
    expect(added).toEqual(['asset-image', 'asset-image']);

    const addButton = view.getByRole('button', { name: '添加素材到时间线' });
    fireEvent.pointerDown(addButton, { button: 0, pointerId: 12, clientX: 80 });
    fireEvent.click(addButton);
    expect(view.getByRole('menuitem', { name: '从资产库添加' })).not.toBeNull();
  });

  test('downloads a composed video result with the timeline title', () => {
    const originalCreateObjectUrl = URL.createObjectURL;
    const originalRevokeObjectUrl = URL.revokeObjectURL;
    const originalAnchorClick = HTMLAnchorElement.prototype.click;
    let download: { href: string; fileName: string } | null = null;
    URL.createObjectURL = () => 'blob:timeline-export';
    URL.revokeObjectURL = () => undefined;
    HTMLAnchorElement.prototype.click = function click() {
      download = { href: this.href, fileName: this.download };
    };
    try {
      downloadTimelineComposition({
        blob: new Blob(['video'], { type: 'video/mp4' }),
        mimeType: 'video/mp4',
        extension: 'mp4',
        width: 1280,
        height: 720,
        durationMs: 5_000,
      }, '时间线1');
      expect(download).toEqual({
        href: 'blob:timeline-export',
        fileName: '时间线1.mp4',
      });
    } finally {
      URL.createObjectURL = originalCreateObjectUrl;
      URL.revokeObjectURL = originalRevokeObjectUrl;
      HTMLAnchorElement.prototype.click = originalAnchorClick;
    }
  });
});
