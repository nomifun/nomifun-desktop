/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../../test/setup-dom.ts';

import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';

import { withCanvasTestI18n } from '../../canvas/components/canvasI18nTestUtils';
import CreativeVideoPlayer from './CreativeVideoPlayer';

afterEach(() => cleanup());

const elementsFor = (container: HTMLElement) => ({
  player: container.querySelector<HTMLElement>('[data-creative-video-player]')!,
  video: container.querySelector<HTMLVideoElement>('video')!,
  centerPlay: container.querySelector<HTMLButtonElement>('[data-video-node-center-play]'),
  controls: container.querySelector<HTMLElement>('[data-video-node-controls]')!,
  toggle: container.querySelector<HTMLButtonElement>('[data-video-node-playback-toggle]')!,
  seek: container.querySelector<HTMLInputElement>('[data-video-node-seek]')!,
  time: container.querySelector<HTMLElement>('[data-video-node-time]')!,
  mute: container.querySelector<HTMLButtonElement>('[data-video-node-mute]')!,
  volume: container.querySelector<HTMLInputElement>('[data-video-node-volume]')!,
  fullscreen: container.querySelector<HTMLButtonElement>('[data-video-node-fullscreen]')!,
});

const setDuration = (video: HTMLVideoElement, duration: number) => {
  Object.defineProperty(video, 'duration', { configurable: true, value: duration });
  fireEvent.loadedMetadata(video);
};

describe('shared creative video player previews', () => {
  test('uses the shared controls for previews without native controls, PiP, or a canvas drag surface', () => {
    const { container } = render(withCanvasTestI18n(
      <CreativeVideoPlayer src='/preview.mp4' poster='/cover.jpg' label='Preview clip' />
    ));
    const elements = elementsFor(container);

    expect(elements.video.controls).toBe(false);
    expect(elements.video.hasAttribute('disablepictureinpicture')).toBe(true);
    expect(elements.video.hasAttribute('playsinline')).toBe(true);
    expect(elements.video.getAttribute('poster')).toBe('/cover.jpg');
    expect(elements.video.getAttribute('aria-label')).toBe('Preview clip');
    expect(elements.video.muted).toBe(false);
    expect(elements.video.autoplay).toBe(false);
    expect(elements.centerPlay).not.toBeNull();
    expect(container.querySelector('[data-video-node-drag-surface]')).toBeNull();
    expect(container.querySelector('[data-video-node-picture-in-picture]')).toBeNull();
    for (const control of [elements.seek, elements.toggle, elements.time, elements.mute, elements.volume, elements.fullscreen]) {
      expect(elements.controls.contains(control)).toBe(true);
    }
    expect(elements.seek.disabled).toBe(true);
  });

  test('plays, pauses, seeks, and adjusts volume through the same controls as canvas nodes', async () => {
    let playRequests = 0;
    const { container } = render(withCanvasTestI18n(
      <CreativeVideoPlayer
        src='/preview.mp4'
        label='Preview clip'
        onPlayRequest={() => { playRequests += 1; }}
      />
    ));
    let elements = elementsFor(container);
    let paused = true;
    let playCalls = 0;
    let pauseCalls = 0;
    Object.defineProperty(elements.video, 'paused', { configurable: true, get: () => paused });
    elements.video.play = () => {
      playCalls += 1;
      paused = false;
      fireEvent.play(elements.video);
      return Promise.resolve();
    };
    elements.video.pause = () => {
      pauseCalls += 1;
      paused = true;
      fireEvent.pause(elements.video);
    };

    await act(async () => { fireEvent.click(elements.centerPlay!); });
    elements = elementsFor(container);
    expect(playCalls).toBe(1);
    expect(playRequests).toBe(1);
    expect(elements.toggle.getAttribute('aria-pressed')).toBe('true');
    expect(elements.centerPlay).toBeNull();
    fireEvent.click(elements.toggle);
    elements = elementsFor(container);
    expect(pauseCalls).toBe(1);
    expect(playRequests).toBe(1);
    expect(elements.toggle.getAttribute('aria-pressed')).toBe('false');
    expect(elements.centerPlay).not.toBeNull();

    setDuration(elements.video, 95);
    fireEvent.input(elements.seek, { target: { value: '45.5' } });
    elements = elementsFor(container);
    expect(elements.video.currentTime).toBeCloseTo(45.5);
    expect(elements.time.textContent).toBe('0:45 / 1:35');
    expect(elements.seek.getAttribute('aria-valuetext')).toBe('0:45 / 1:35');

    fireEvent.input(elements.volume, { target: { value: '0.4' } });
    elements = elementsFor(container);
    expect(elements.video.volume).toBeCloseTo(0.4);
    expect(elements.video.muted).toBe(false);
    fireEvent.click(elements.mute);
    elements = elementsFor(container);
    expect(elements.video.muted).toBe(true);
    expect(Number(elements.volume.value)).toBe(0);
    fireEvent.click(elements.mute);
    elements = elementsFor(container);
    expect(elements.video.muted).toBe(false);
    expect(Number(elements.volume.value)).toBeCloseTo(0.4);
    fireEvent.input(elements.volume, { target: { value: '0' } });
    expect(elementsFor(container).video.muted).toBe(true);
    fireEvent.click(elementsFor(container).mute);
    elements = elementsFor(container);
    expect(elements.video.muted).toBe(false);
    expect(elements.video.volume).toBeCloseTo(0.4);

    await act(async () => { fireEvent.click(elements.toggle); });
    expect(playRequests).toBe(2);
    expect(playCalls).toBe(2);
  });

  test('stops previous playback, resets source state, and removes fullscreen listeners on disposal', () => {
    const originalAdd = document.addEventListener;
    const originalRemove = document.removeEventListener;
    const added: Array<{ type: string; listener: EventListenerOrEventListenerObject }> = [];
    const removed: Array<{ type: string; listener: EventListenerOrEventListenerObject }> = [];
    const isFullscreenEvent = (type: string) => type === 'fullscreenchange' || type === 'webkitfullscreenchange';
    document.addEventListener = (
      type: string,
      listener: EventListenerOrEventListenerObject,
      options?: boolean | AddEventListenerOptions
    ) => {
      if (isFullscreenEvent(type)) added.push({ type, listener });
      originalAdd.call(document, type, listener, options);
    };
    document.removeEventListener = (
      type: string,
      listener: EventListenerOrEventListenerObject,
      options?: boolean | EventListenerOptions
    ) => {
      if (isFullscreenEvent(type)) removed.push({ type, listener });
      originalRemove.call(document, type, listener, options);
    };

    try {
      const view = (src: string) => withCanvasTestI18n(<CreativeVideoPlayer src={src} label='Preview clip' />);
      const { container, rerender, unmount } = render(view('/first.mp4'));
      const first = elementsFor(container);
      let firstPauses = 0;
      first.video.pause = () => { firstPauses += 1; };
      setDuration(first.video, 120);
      fireEvent.input(first.seek, { target: { value: '50' } });
      fireEvent.input(first.volume, { target: { value: '0.25' } });
      fireEvent.play(first.video);

      rerender(view('/second.mp4'));
      const second = elementsFor(container);
      expect(firstPauses).toBeGreaterThan(0);
      expect(first.video.isConnected).toBe(false);
      expect(second.video).not.toBe(first.video);
      expect(second.video.getAttribute('src')).toBe('/second.mp4');
      expect(second.time.textContent).toBe('0:00 / 0:00');
      expect(second.seek.disabled).toBe(true);
      expect(second.toggle.getAttribute('aria-pressed')).toBe('false');
      expect(second.video.muted).toBe(false);
      expect(second.video.volume).toBe(1);

      let secondPauses = 0;
      second.video.pause = () => { secondPauses += 1; };
      fireEvent.play(second.video);
      unmount();
      expect(secondPauses).toBeGreaterThan(0);
      expect(second.video.isConnected).toBe(false);
      expect(added.some(({ type }) => type === 'fullscreenchange')).toBe(true);
      expect(added.some(({ type }) => type === 'webkitfullscreenchange')).toBe(true);
      expect(removed).toHaveLength(added.length);
      for (const registration of added) {
        expect(removed.some(({ type, listener }) => type === registration.type && listener === registration.listener)).toBe(true);
      }
    } finally {
      cleanup();
      document.addEventListener = originalAdd;
      document.removeEventListener = originalRemove;
    }
  });

  test('lets preview Escape reach its dialog while keeping canvas keyboard controls isolated', () => {
    const receivedKeys: string[] = [];
    const view = (variant: 'preview' | 'canvas') => withCanvasTestI18n(
      <div onKeyDown={(event) => { receivedKeys.push(event.key); }}>
        <CreativeVideoPlayer src='/preview.mp4' label='Preview clip' variant={variant} />
      </div>
    );
    const { container, rerender } = render(view('preview'));
    const preview = elementsFor(container);
    for (const control of [preview.centerPlay!, preview.toggle, preview.mute, preview.fullscreen]) {
      fireEvent.keyDown(control, { key: 'Escape' });
    }
    expect(receivedKeys).toEqual(['Escape', 'Escape', 'Escape', 'Escape']);

    rerender(view('canvas'));
    const canvas = elementsFor(container);
    expect(container.querySelector('[data-video-node-drag-surface]')).not.toBeNull();
    for (const control of [canvas.centerPlay!, canvas.toggle, canvas.mute, canvas.fullscreen]) {
      fireEvent.keyDown(control, { key: 'Delete' });
    }
    expect(receivedKeys).toEqual(['Escape', 'Escape', 'Escape', 'Escape']);
  });

  test.each(['standard', 'webkit'] as const)('supports %s fullscreen in preview windows and retains playback settings', async (mode) => {
    const elementKey = mode === 'standard' ? 'fullscreenElement' : 'webkitFullscreenElement';
    const enterKey = mode === 'standard' ? 'requestFullscreen' : 'webkitRequestFullscreen';
    const exitKey = mode === 'standard' ? 'exitFullscreen' : 'webkitExitFullscreen';
    const eventName = mode === 'standard' ? 'fullscreenchange' : 'webkitfullscreenchange';
    const properties = ['fullscreenElement', 'webkitFullscreenElement', 'exitFullscreen', 'webkitExitFullscreen'] as const;
    const descriptors = properties.map((key) => Object.getOwnPropertyDescriptor(document, key));
    const { container } = render(withCanvasTestI18n(<CreativeVideoPlayer src='/preview.mp4' label='Preview clip' />));
    const elements = elementsFor(container);
    let fullscreenElement: Element | null = null;
    let entries = 0;
    let exits = 0;

    try {
      for (const key of properties) Object.defineProperty(document, key, { configurable: true, value: undefined });
      Object.defineProperty(document, elementKey, { configurable: true, get: () => fullscreenElement });
      Object.defineProperty(elements.player, 'requestFullscreen', { configurable: true, value: undefined });
      Object.defineProperty(elements.player, enterKey, {
        configurable: true,
        value: async () => {
          entries += 1;
          fullscreenElement = elements.player;
          document.dispatchEvent(new Event(eventName));
        },
      });
      Object.defineProperty(document, exitKey, {
        configurable: true,
        value: async () => {
          exits += 1;
          fullscreenElement = null;
          document.dispatchEvent(new Event(eventName));
        },
      });
      fireEvent.input(elements.volume, { target: { value: '0.3' } });
      await act(async () => { fireEvent.click(elements.fullscreen); });
      expect(entries).toBe(1);
      expect(elements.fullscreen.getAttribute('aria-pressed')).toBe('true');
      expect(elements.video.volume).toBeCloseTo(0.3);
      await act(async () => { fireEvent.click(elements.fullscreen); });
      expect(exits).toBe(1);
      expect(elements.fullscreen.getAttribute('aria-pressed')).toBe('false');
      expect(elements.video.volume).toBeCloseTo(0.3);
    } finally {
      cleanup();
      for (const [index, key] of properties.entries()) {
        const descriptor = descriptors[index];
        if (descriptor) Object.defineProperty(document, key, descriptor);
        else delete (document as unknown as Record<string, unknown>)[key];
      }
    }
  });

  test('contains rejected playback requests and recovers from media errors after replacing the source', async () => {
    const view = (src: string) => withCanvasTestI18n(<CreativeVideoPlayer src={src} label='Preview clip' />);
    const { container, rerender } = render(view('/blocked.mp4'));
    let elements = elementsFor(container);
    elements.video.play = () => Promise.reject(new Error('Playback denied'));
    await act(async () => { fireEvent.click(elements.centerPlay!); });
    elements = elementsFor(container);
    expect(elements.toggle.getAttribute('aria-pressed')).toBe('false');
    expect(elements.centerPlay).not.toBeNull();

    fireEvent.error(elements.video);
    expect(container.querySelector('[role="status"]')).not.toBeNull();
    expect(elementsFor(container).centerPlay).toBeNull();
    rerender(view('/replacement.mp4'));
    expect(container.querySelector('[role="status"]')).toBeNull();
    expect(elementsFor(container).centerPlay).not.toBeNull();
  });
});
