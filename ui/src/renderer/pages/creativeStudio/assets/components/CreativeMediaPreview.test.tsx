/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../../test/setup-dom.ts';

import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import React from 'react';

import CreativeMediaPreview from './CreativeMediaPreview';
import CreativeAssetMedia from './CreativeAssetMedia';
import CreativeVideoMedia from './CreativeVideoMedia';
import type { CreativeAsset } from '../types';

const OriginalObserver = globalThis.IntersectionObserver;
let initiallyVisible = true;
let revealVideo: () => void;
beforeEach(() => {
  initiallyVisible = true;
  globalThis.IntersectionObserver = class extends OriginalObserver {
    constructor(callback: IntersectionObserverCallback) {
      super(callback);
      revealVideo = () => callback([{ isIntersecting: true } as IntersectionObserverEntry], this);
    }
    observe() { if (initiallyVisible) revealVideo(); }
  };
});
afterEach(() => {
  cleanup();
  globalThis.IntersectionObserver = OriginalObserver;
});

describe('shared creative media previews', () => {
  test('falls back from a broken image thumbnail to the original, then an honest unavailable state', () => {
    const { container, rerender } = render(<CreativeMediaPreview kind='image' src='/original.png' posterSrc='/thumb.jpg' />);
    expect(container.querySelector('img')?.getAttribute('src')).toBe('/thumb.jpg');
    fireEvent.error(container.querySelector('img')!);
    expect(container.querySelector('img')?.getAttribute('src')).toBe('/original.png');
    fireEvent.error(container.querySelector('img')!);
    expect(container.querySelector('img')).toBeNull();
    expect(container.querySelector('[data-asset-media-state="missing"]')).not.toBeNull();
    rerender(<CreativeMediaPreview kind='image' src='/replacement.png' posterSrc='/replacement.jpg' />);
    expect(container.querySelector('img')?.getAttribute('src')).toBe('/replacement.jpg');
  });

  test('uses an actual poster first, then decodes the original video without treating MP4 as an image', () => {
    const { container } = render(<CreativeMediaPreview kind='video' src='/clip.mp4' posterSrc='/cover.jpg' />);
    expect(container.querySelector('video')).toBeNull();
    fireEvent.error(container.querySelector('img')!);
    expect(container.querySelector('img')).toBeNull();
    const video = container.querySelector('video')!;
    expect(video.getAttribute('src')).toBe('/clip.mp4');
    expect(video.preload).toBe('auto');
    expect(video.muted).toBe(true);
    expect(video.autoplay).toBe(false);
    expect(video.controls).toBe(false);
    fireEvent.error(video);
    expect(container.querySelector('video')).toBeNull();
    expect(container.querySelector('[role="status"]')).not.toBeNull();
  });

  test('does not fetch deleted content even when stale URLs remain on the record', () => {
    const asset: CreativeAsset = {
      id: 'deleted', kind: 'video', title: 'Clip', collection: null, tags: [], mimeType: 'video/mp4',
      width: null, height: null, bytes: 100, inLibrary: false, textContent: null, origin: null,
      originalUrl: '/deleted.mp4', thumbnailUrl: '/deleted.jpg', createdAt: 1, updatedAt: 1, deletedAt: 2,
    };
    const { container } = render(<CreativeAssetMedia asset={asset} unavailableLabel='Missing' />);
    expect(container.querySelector('img, video')).toBeNull();
    expect(container.querySelector('[data-asset-media-state="deleted"]')).not.toBeNull();
  });
});

describe('shared video first frame', () => {
  test('waits until a video approaches the viewport before requesting its first frame', () => {
    initiallyVisible = false;
    const { container } = render(<CreativeVideoMedia src='/offscreen.mp4' />);
    const video = container.querySelector('video')!;
    expect(video.preload).toBe('none');
    act(() => revealVideo());
    expect(video.preload).toBe('auto');
  });

  test('falls back when the poster fails after video metadata has already loaded', () => {
    const OriginalImage = globalThis.Image;
    let posterImage: HTMLImageElement;
    globalThis.Image = class extends OriginalImage {
      constructor() { super(); posterImage = this; }
    };
    try {
      const { container } = render(<CreativeVideoMedia src='/clip.mp4' poster='/bad-poster.jpg' />);
      const video = container.querySelector('video')!;
      Object.defineProperty(video, 'duration', { configurable: true, value: 33 });
      Object.defineProperty(video, 'readyState', { configurable: true, value: 1 });
      fireEvent.loadedMetadata(video);
      expect(video.currentTime).toBe(0);
      fireEvent.error(posterImage!);
      expect(video.getAttribute('poster')).toBeNull();
      expect(video.preload).toBe('auto');
      expect(video.currentTime).toBe(0.01);
    } finally {
      globalThis.Image = OriginalImage;
    }
  });

  test('primes a paused video after metadata and forwards the original media events and ref', () => {
    const ref = React.createRef<HTMLVideoElement>();
    let metadataEvents = 0;
    let dataEvents = 0;
    const { container } = render(<CreativeVideoMedia ref={ref} src='/clip.mp4' onLoadedMetadata={() => { metadataEvents += 1; }} onLoadedData={() => { dataEvents += 1; }} />);
    const video = container.querySelector('video')!;
    expect(ref.current).toBe(video);
    Object.defineProperty(video, 'duration', { configurable: true, value: 33 });
    expect(video.preload).toBe('auto');
    fireEvent.loadedMetadata(video);
    expect(video.currentTime).toBe(0.01);
    expect(metadataEvents).toBe(1);
    fireEvent.loadedData(video);
    expect(video.preload).toBe('metadata');
    expect(dataEvents).toBe(1);
    expect(video.paused).toBe(true);
  });

  test('never overwrites playback, autoplay, supplied poster or a caller-selected position', () => {
    for (const mode of ['playing', 'autoplay', 'poster', 'position'] as const) {
      const { container, unmount } = render(<CreativeVideoMedia src='/clip.mp4' autoPlay={mode === 'autoplay'} poster={mode === 'poster' ? '/cover.jpg' : undefined} />);
      const video = container.querySelector('video')!;
      Object.defineProperty(video, 'duration', { configurable: true, value: 10 });
      if (mode === 'playing') Object.defineProperty(video, 'paused', { configurable: true, value: false });
      if (mode === 'position') video.currentTime = 5;
      fireEvent.loadedMetadata(video);
      fireEvent.loadedData(video);
      expect([mode, video.currentTime]).toEqual([mode, mode === 'position' ? 5 : 0]);
      unmount();
    }
  });

  test('keeps short and unknown-duration media valid and resets first-frame state when the URL changes', () => {
    const { container, rerender } = render(<CreativeVideoMedia src='/short.mp4' />);
    const video = container.querySelector('video')!;
    Object.defineProperty(video, 'duration', { configurable: true, value: 0.004 });
    fireEvent.loadedMetadata(video);
    expect(video.currentTime).toBe(0.002);
    fireEvent.loadedData(video);
    rerender(<CreativeVideoMedia src='/next.mp4' />);
    const replacement = container.querySelector('video')!;
    expect(replacement).not.toBe(video);
    expect(replacement.preload).toBe('auto');
    Object.defineProperty(replacement, 'duration', { configurable: true, value: Infinity });
    fireEvent.loadedMetadata(replacement);
    expect(replacement.currentTime).toBe(0);
  });
});
