/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../../test/setup-dom.ts';

import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';

import { withCanvasTestI18n } from '../components/canvasI18nTestUtils';
import { testNode, testUuid } from '../core/testFixtures';
import { CreativeNodeView } from './CreativeNodeViews';
import { formatVideoNodeTime } from './CreativeVideoNodeMedia';

afterEach(() => cleanup());

const loadedVideoNode = (index: number) => {
  const node = testNode('video', index);
  node.data.assetId = testUuid(index + 1);
  return node;
};

const playerElements = (container: HTMLElement) => ({
  player: container.querySelector<HTMLElement>('[data-video-node-player]')!,
  video: container.querySelector<HTMLVideoElement>('video')!,
  dragSurface: container.querySelector<HTMLElement>('[data-video-node-drag-surface]')!,
  centerPlay: container.querySelector<HTMLButtonElement>('[data-video-node-center-play]'),
  controls: container.querySelector<HTMLElement>('[data-video-node-controls]')!,
  seek: container.querySelector<HTMLInputElement>('[data-video-node-seek]')!,
  toggle: container.querySelector<HTMLButtonElement>('[data-video-node-playback-toggle]')!,
  time: container.querySelector<HTMLElement>('[data-video-node-time]')!,
  mute: container.querySelector<HTMLButtonElement>('[data-video-node-mute]')!,
  volume: container.querySelector<HTMLInputElement>('[data-video-node-volume]')!,
  fullscreen: container.querySelector<HTMLButtonElement>('[data-video-node-fullscreen]')!,
});

const setMediaNumber = (
  video: HTMLVideoElement,
  property: 'currentTime' | 'duration',
  value: number
) => {
  Object.defineProperty(video, property, {
    configurable: true,
    writable: true,
    value,
  });
};

describe('Creative video node custom player', () => {
  test('formats finite media times without allowing invalid values into the UI', () => {
    expect(formatVideoNodeTime(0)).toBe('0:00');
    expect(formatVideoNodeTime(62.99)).toBe('1:02');
    expect(formatVideoNodeTime(3_661.8)).toBe('1:01:01');
    expect(formatVideoNodeTime(-1)).toBe('0:00');
    expect(formatVideoNodeTime(Number.POSITIVE_INFINITY)).toBe('0:00');
    expect(formatVideoNodeTime(Number.NaN)).toBe('0:00');
  });

  test('renders one compact custom control surface and removes native PiP and skip affordances', () => {
    const node = loadedVideoNode(2000);
    node.data.muted = true;
    node.data.loop = true;
    const { container } = render(withCanvasTestI18n(
      <CreativeNodeView
        node={node}
        selected
        asset={{ src: '/video.mp4', posterSrc: '/poster.png', alt: 'Preview' }}
      />
    ));
    const elements = playerElements(container);

    expect(elements.video.controls).toBe(false);
    expect(elements.video.hasAttribute('disablepictureinpicture')).toBe(true);
    expect(elements.video.hasAttribute('playsinline')).toBe(true);
    expect(elements.video.tabIndex).toBe(-1);
    expect(elements.video.getAttribute('draggable')).toBe('false');
    expect(elements.video.muted).toBe(true);
    expect(elements.video.loop).toBe(true);
    expect(elements.video.getAttribute('poster')).toBe('/poster.png');
    expect(elements.centerPlay).not.toBeNull();
    expect(elements.controls.contains(elements.seek)).toBe(true);
    expect(elements.controls.contains(elements.toggle)).toBe(true);
    expect(elements.controls.contains(elements.mute)).toBe(true);
    expect(elements.controls.contains(elements.volume)).toBe(true);
    expect(elements.controls.contains(elements.fullscreen)).toBe(true);
    expect(elements.volume.type).toBe('range');
    expect(elements.volume.min).toBe('0');
    expect(elements.volume.max).toBe('1');
    expect(elements.volume.step).toBe('0.01');
    expect(Number(elements.volume.value)).toBe(0);
    expect(elements.controls.querySelectorAll('button')).toHaveLength(3);
    expect(elements.player.querySelectorAll('button')).toHaveLength(4);
    expect(elements.player.querySelector('[data-video-node-picture-in-picture]')).toBeNull();
    expect(elements.player.querySelector('[data-video-node-skip-back]')).toBeNull();
    expect(elements.player.querySelector('[data-video-node-skip-forward]')).toBeNull();
  });

  test('derives playing, duration, current time, and ended state from media events', () => {
    const { container } = render(withCanvasTestI18n(
      <CreativeNodeView
        node={loadedVideoNode(2020)}
        selected
        asset={{ src: '/video.mp4' }}
      />
    ));
    let elements = playerElements(container);
    setMediaNumber(elements.video, 'duration', 125.9);
    setMediaNumber(elements.video, 'currentTime', 62.8);

    fireEvent.loadedMetadata(elements.video);
    elements = playerElements(container);
    expect(elements.seek.disabled).toBe(false);
    expect(elements.seek.max).toBe('125.9');
    expect(Number(elements.seek.value)).toBeCloseTo(62.8);
    expect(elements.time.textContent).toBe('1:02 / 2:05');
    expect(elements.seek.getAttribute('aria-valuetext')).toBe('1:02 / 2:05');

    setMediaNumber(elements.video, 'duration', 130);
    fireEvent.durationChange(elements.video);
    setMediaNumber(elements.video, 'currentTime', 65.4);
    fireEvent.timeUpdate(elements.video);
    elements = playerElements(container);
    expect(elements.time.textContent).toBe('1:05 / 2:10');
    expect(elements.seek.max).toBe('130');
    expect(Number(elements.seek.value)).toBeCloseTo(65.4);

    fireEvent.play(elements.video);
    elements = playerElements(container);
    expect(elements.player.getAttribute('data-video-node-playing')).toBe('true');
    expect(elements.toggle.getAttribute('aria-pressed')).toBe('true');
    expect(elements.centerPlay).toBeNull();

    fireEvent.pause(elements.video);
    elements = playerElements(container);
    expect(elements.player.getAttribute('data-video-node-playing')).toBe('false');
    expect(elements.toggle.getAttribute('aria-pressed')).toBe('false');
    expect(elements.centerPlay).not.toBeNull();

    fireEvent.play(elements.video);
    fireEvent.ended(elements.video);
    elements = playerElements(container);
    expect(elements.toggle.getAttribute('aria-pressed')).toBe('false');
    expect(elements.centerPlay).not.toBeNull();
  });

  test('operates play, pause, seek, volume, and mute without persisting playback state', async () => {
    let activations = 0;
    const node = loadedVideoNode(2040);
    const view = (selected: boolean) => withCanvasTestI18n(
      <CreativeNodeView
        node={node}
        selected={selected}
        asset={{ src: '/video.mp4' }}
        onActivate={() => { activations += 1; }}
      />
    );
    const { container, rerender } = render(view(false));
    let elements = playerElements(container);
    let paused = true;
    let playCalls = 0;
    let pauseCalls = 0;
    Object.defineProperty(elements.video, 'paused', {
      configurable: true,
      get: () => paused,
    });
    elements.video.play = () => {
      playCalls += 1;
      return Promise.resolve();
    };
    elements.video.pause = () => { pauseCalls += 1; };

    await act(async () => { fireEvent.click(elements.centerPlay!); });
    expect(activations).toBe(1);
    expect(playCalls).toBe(1);
    paused = false;
    fireEvent.play(elements.video);
    elements = playerElements(container);
    expect(elements.player.getAttribute('data-video-node-playing')).toBe('true');
    rerender(view(true));
    fireEvent.click(elements.toggle);
    expect(pauseCalls).toBe(1);
    fireEvent.pause(elements.video);

    setMediaNumber(elements.video, 'duration', 90);
    setMediaNumber(elements.video, 'currentTime', 0);
    fireEvent.loadedMetadata(elements.video);
    elements = playerElements(container);
    fireEvent.input(elements.seek, { target: { value: '45.5' } });
    elements = playerElements(container);
    expect(elements.video.currentTime).toBeCloseTo(45.5);
    expect(elements.time.textContent).toBe('0:45 / 1:30');
    expect(elements.seek.getAttribute('aria-valuetext')).toBe('0:45 / 1:30');

    expect(elements.video.muted).toBe(false);
    expect(elements.video.volume).toBe(1);
    expect(Number(elements.volume.value)).toBe(1);
    fireEvent.input(elements.volume, { target: { value: '0.35' } });
    elements = playerElements(container);
    expect(elements.video.volume).toBeCloseTo(0.35);
    expect(elements.video.muted).toBe(false);
    expect(Number(elements.volume.value)).toBeCloseTo(0.35);

    fireEvent.click(elements.mute);
    elements = playerElements(container);
    expect(elements.video.muted).toBe(true);
    expect(elements.video.volume).toBeCloseTo(0.35);
    expect(Number(elements.volume.value)).toBe(0);
    expect(elements.mute.getAttribute('aria-pressed')).toBe('true');
    fireEvent.click(elements.mute);
    elements = playerElements(container);
    expect(elements.video.muted).toBe(false);
    expect(elements.video.volume).toBeCloseTo(0.35);
    expect(Number(elements.volume.value)).toBeCloseTo(0.35);
    expect(elements.mute.getAttribute('aria-pressed')).toBe('false');

    fireEvent.input(elements.volume, { target: { value: '0' } });
    elements = playerElements(container);
    expect(elements.video.volume).toBe(0);
    expect(elements.video.muted).toBe(true);
    expect(elements.mute.getAttribute('aria-pressed')).toBe('true');
    fireEvent.click(elements.mute);
    elements = playerElements(container);
    expect(elements.video.volume).toBeCloseTo(0.35);
    expect(elements.video.muted).toBe(false);

    fireEvent.click(elements.mute);
    fireEvent.input(elements.volume, { target: { value: '1' } });
    elements = playerElements(container);
    expect(elements.video.volume).toBe(1);
    expect(elements.video.muted).toBe(false);
    expect(Number(elements.volume.value)).toBe(1);
    expect(node.data.muted).toBe(false);
  });

  test('synchronizes external media volume changes and restores the last audible level', () => {
    const { container } = render(withCanvasTestI18n(
      <CreativeNodeView
        node={loadedVideoNode(2050)}
        selected
        asset={{ src: '/video.mp4' }}
      />
    ));
    let elements = playerElements(container);

    elements.video.volume = 0.62;
    elements.video.muted = false;
    fireEvent.volumeChange(elements.video);
    elements = playerElements(container);
    expect(Number(elements.volume.value)).toBeCloseTo(0.62);
    expect(elements.mute.getAttribute('aria-pressed')).toBe('false');

    elements.video.volume = 0;
    elements.video.muted = true;
    fireEvent.volumeChange(elements.video);
    elements = playerElements(container);
    expect(Number(elements.volume.value)).toBe(0);
    expect(elements.mute.getAttribute('aria-pressed')).toBe('true');

    fireEvent.click(elements.mute);
    elements = playerElements(container);
    expect(elements.video.volume).toBeCloseTo(0.62);
    expect(elements.video.muted).toBe(false);
    expect(Number(elements.volume.value)).toBeCloseTo(0.62);
  });

  test('isolates every player control while leaving the remaining media surface draggable', () => {
    const received = { pointer: 0, click: 0, doubleClick: 0, key: 0 };
    const { container } = render(withCanvasTestI18n(
      <div
        onPointerDown={() => { received.pointer += 1; }}
        onClick={() => { received.click += 1; }}
        onDoubleClick={() => { received.doubleClick += 1; }}
        onKeyDown={() => { received.key += 1; }}
      >
        <CreativeNodeView
          node={loadedVideoNode(2060)}
          selected
          asset={{ src: '/video.mp4' }}
        />
      </div>
    ));
    const elements = playerElements(container);
    elements.video.play = () => Promise.resolve();
    const interactiveTargets = [
      elements.centerPlay!,
      elements.toggle,
      elements.mute,
      elements.fullscreen,
    ];

    for (const target of interactiveTargets) {
      const pointerDown = new PointerEvent('pointerdown', { bubbles: true, cancelable: true });
      fireEvent(target, pointerDown);
      expect(pointerDown.defaultPrevented).toBe(false);
      fireEvent.keyDown(target, { key: 'Delete' });
    }
    fireEvent.pointerDown(elements.seek, { pointerId: 6, button: 0 });
    fireEvent.pointerDown(elements.volume, { pointerId: 7, button: 0 });
    fireEvent.input(elements.volume, { target: { value: '0.4' } });
    expect(elements.video.volume).toBeCloseTo(0.4);
    expect(received.pointer).toBe(0);
    expect(received).toEqual({ pointer: 0, click: 0, doubleClick: 0, key: 0 });

    fireEvent.pointerDown(elements.dragSurface, { pointerId: 8, button: 0 });
    expect(received.pointer).toBe(1);
  });

  test('preserves playback across selection changes and resets state for a new source or node', () => {
    const node = loadedVideoNode(2080);
    node.data.autoplay = true;
    const view = (nodeId: string, src: string, selected: boolean) => withCanvasTestI18n(
      <CreativeNodeView
        node={{ ...node, id: nodeId }}
        selected={selected}
        asset={{ src, posterSrc: '/poster.png' }}
      />
    );
    const { container, rerender } = render(view(node.id, '/first.mp4', true));
    let elements = playerElements(container);
    let pauseCalls = 0;
    elements.video.pause = () => { pauseCalls += 1; };

    fireEvent.play(elements.video);
    fireEvent.input(elements.volume, { target: { value: '0.35' } });
    rerender(view(node.id, '/first.mp4', false));
    expect(pauseCalls).toBe(0);
    expect(elements.video.autoplay).toBe(true);
    expect(playerElements(container).toggle.getAttribute('aria-pressed')).toBe('true');
    expect(playerElements(container).video.volume).toBeCloseTo(0.35);
    expect(Number(playerElements(container).volume.value)).toBeCloseTo(0.35);
    rerender(view(node.id, '/first.mp4', true));
    expect(playerElements(container).video).toBe(elements.video);
    expect(pauseCalls).toBe(0);

    fireEvent.pause(elements.video);
    expect(playerElements(container).toggle.getAttribute('aria-pressed')).toBe('false');

    for (const [nodeId, src] of [
      [node.id, '/replacement.mp4'],
      ['another-node', '/replacement.mp4'],
    ]) {
      const previousVideo = playerElements(container).video;
      fireEvent.play(previousVideo);
      fireEvent.input(playerElements(container).volume, { target: { value: '0.25' } });
      setMediaNumber(previousVideo, 'duration', 60);
      setMediaNumber(previousVideo, 'currentTime', 30);
      fireEvent.loadedMetadata(previousVideo);

      rerender(view(nodeId, src, true));
      elements = playerElements(container);
      expect(elements.video).not.toBe(previousVideo);
      expect(elements.video.getAttribute('src')).toBe(src);
      expect(elements.video.getAttribute('poster')).toBe('/poster.png');
      expect(elements.video.controls).toBe(false);
      expect(elements.video.tabIndex).toBe(-1);
      expect(elements.video.volume).toBe(1);
      expect(Number(elements.volume.value)).toBe(1);
      expect(elements.mute.getAttribute('aria-pressed')).toBe('false');
      expect(elements.toggle.getAttribute('aria-pressed')).toBe('false');
      expect(elements.time.textContent).toBe('0:00 / 0:00');
      expect(elements.centerPlay).not.toBeNull();
    }
  });

  test('uses standard and WebKit fullscreen APIs and contains request failures', async () => {
    type TestFullscreenDocument = Document & {
      webkitExitFullscreen?: () => Promise<void> | void;
      webkitFullscreenElement?: Element | null;
    };
    type TestFullscreenElement = HTMLElement & {
      webkitRequestFullscreen?: () => Promise<void> | void;
    };
    const fullscreenDocument = document as TestFullscreenDocument;
    const propertyNames = [
      'fullscreenElement',
      'exitFullscreen',
      'webkitFullscreenElement',
      'webkitExitFullscreen',
    ] as const;
    const descriptors = propertyNames.map((name) =>
      Object.getOwnPropertyDescriptor(fullscreenDocument, name)
    );
    let canvasPointerDowns = 0;
    let canvasDoubleClicks = 0;
    const { container } = render(withCanvasTestI18n(
      <div
        onPointerDown={() => { canvasPointerDowns += 1; }}
        onDoubleClick={() => { canvasDoubleClicks += 1; }}
      >
        <CreativeNodeView
          node={loadedVideoNode(2100)}
          selected
          asset={{ src: '/video.mp4' }}
        />
      </div>
    ));
    const elements = playerElements(container);
    const player = elements.player as TestFullscreenElement;
    let standardElement: Element | null = null;
    let webkitElement: Element | null = null;
    let standardRequests = 0;
    let standardExits = 0;
    let webkitRequests = 0;
    let webkitExits = 0;

    try {
      fireEvent.input(elements.volume, { target: { value: '0.4' } });
      expect(elements.video.volume).toBeCloseTo(0.4);
      Object.defineProperty(fullscreenDocument, 'fullscreenElement', {
        configurable: true,
        get: () => standardElement,
      });
      Object.defineProperty(fullscreenDocument, 'webkitFullscreenElement', {
        configurable: true,
        get: () => webkitElement,
      });
      Object.defineProperty(fullscreenDocument, 'exitFullscreen', {
        configurable: true,
        value: async () => {
          standardExits += 1;
          standardElement = null;
          fullscreenDocument.dispatchEvent(new Event('fullscreenchange'));
        },
      });
      Object.defineProperty(player, 'requestFullscreen', {
        configurable: true,
        value: async () => {
          standardRequests += 1;
          standardElement = player;
          fullscreenDocument.dispatchEvent(new Event('fullscreenchange'));
        },
      });

      await act(async () => {
        fireEvent.click(elements.fullscreen);
        await Promise.resolve();
      });
      expect(standardRequests).toBe(1);
      expect(elements.fullscreen.getAttribute('aria-pressed')).toBe('true');
      expect(elements.video.volume).toBeCloseTo(0.4);
      expect(Number(elements.volume.value)).toBeCloseTo(0.4);
      fireEvent.pointerDown(elements.dragSurface, { pointerId: 9, button: 0 });
      fireEvent.doubleClick(elements.dragSurface);
      expect(canvasPointerDowns).toBe(0);
      expect(canvasDoubleClicks).toBe(0);
      await act(async () => {
        fireEvent.click(elements.fullscreen);
        await Promise.resolve();
      });
      expect(standardExits).toBe(1);
      expect(elements.fullscreen.getAttribute('aria-pressed')).toBe('false');
      expect(elements.video.volume).toBeCloseTo(0.4);

      Object.defineProperty(fullscreenDocument, 'exitFullscreen', {
        configurable: true,
        value: undefined,
      });
      Object.defineProperty(player, 'requestFullscreen', {
        configurable: true,
        value: undefined,
      });
      Object.defineProperty(fullscreenDocument, 'webkitExitFullscreen', {
        configurable: true,
        value: async () => {
          webkitExits += 1;
          webkitElement = null;
          fullscreenDocument.dispatchEvent(new Event('webkitfullscreenchange'));
        },
      });
      Object.defineProperty(player, 'webkitRequestFullscreen', {
        configurable: true,
        value: async () => {
          webkitRequests += 1;
          webkitElement = player;
          fullscreenDocument.dispatchEvent(new Event('webkitfullscreenchange'));
        },
      });

      await act(async () => {
        fireEvent.click(elements.fullscreen);
        await Promise.resolve();
      });
      expect(webkitRequests).toBe(1);
      expect(elements.fullscreen.getAttribute('aria-pressed')).toBe('true');
      await act(async () => {
        fireEvent.click(elements.fullscreen);
        await Promise.resolve();
      });
      expect(webkitExits).toBe(1);
      expect(elements.fullscreen.getAttribute('aria-pressed')).toBe('false');

      Object.defineProperty(player, 'webkitRequestFullscreen', {
        configurable: true,
        value: () => Promise.reject(new Error('Fullscreen denied')),
      });
      await act(async () => {
        fireEvent.click(elements.fullscreen);
        await Promise.resolve();
      });
      expect(elements.fullscreen.getAttribute('aria-pressed')).toBe('false');
      expect(elements.video.controls).toBe(false);
    } finally {
      for (const [index, name] of propertyNames.entries()) {
        const descriptor = descriptors[index];
        if (descriptor) Object.defineProperty(fullscreenDocument, name, descriptor);
        else delete (fullscreenDocument as unknown as Record<string, unknown>)[name];
      }
    }
  });
});
