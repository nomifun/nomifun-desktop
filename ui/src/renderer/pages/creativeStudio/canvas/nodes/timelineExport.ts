/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  CreativeTimelineClip,
  CreativeTimelineNodeData,
} from '../../domain';
import { timelineDurationMs } from './timelineModel';

const DEFAULT_EXPORT_FPS = 30;
const MAX_EXPORT_EDGE = 1_920;
const MEDIA_LOAD_TIMEOUT_MS = 30_000;

const EXPORT_FORMATS = [
  {
    mimeType: 'video/mp4;codecs=avc1.42E01E,mp4a.40.2',
    extension: 'mp4',
    audio: true,
  },
  {
    mimeType: 'video/mp4;codecs=avc1.42E01E',
    extension: 'mp4',
    audio: false,
  },
  { mimeType: 'video/mp4', extension: 'mp4', audio: true },
  { mimeType: 'video/webm;codecs=vp9,opus', extension: 'webm', audio: true },
  { mimeType: 'video/webm;codecs=vp8,opus', extension: 'webm', audio: true },
  { mimeType: 'video/webm;codecs=vp9', extension: 'webm', audio: false },
  { mimeType: 'video/webm;codecs=vp8', extension: 'webm', audio: false },
  { mimeType: 'video/webm', extension: 'webm', audio: true },
] as const satisfies readonly TimelineExportFormat[];

export interface CreativeTimelineAssetPresentation {
  assetId: string;
  kind: 'image' | 'video';
  title: string;
  src: string;
  thumbnailSrc?: string | null;
  deleted?: boolean;
  width?: number | null;
  height?: number | null;
  mimeType?: string | null;
}

export type TimelineExportErrorCode =
  | 'empty'
  | 'asset-unavailable'
  | 'render-unsupported'
  | 'recording-unsupported'
  | 'audio-unsupported'
  | 'media-load-failed'
  | 'recording-failed';

export class TimelineExportError extends Error {
  readonly code: TimelineExportErrorCode;

  constructor(code: TimelineExportErrorCode, message: string) {
    super(message);
    this.name = 'TimelineExportError';
    this.code = code;
  }
}

interface TimelineExportPlanClip {
  clip: CreativeTimelineClip;
  asset: CreativeTimelineAssetPresentation;
}

export interface TimelineExportPlan {
  clips: TimelineExportPlanClip[];
  durationMs: number;
}

export interface TimelineCompositionExportOptions {
  data: CreativeTimelineNodeData;
  assets: ReadonlyMap<string, CreativeTimelineAssetPresentation>;
  fps?: number;
  signal?: AbortSignal;
  onProgress?: (progress: number) => void;
}

export interface TimelineCompositionExportResult {
  blob: Blob;
  mimeType: string;
  extension: 'mp4' | 'webm';
  width: number;
  height: number;
  durationMs: number;
}

export interface TimelineExportFormat {
  mimeType: string;
  extension: 'mp4' | 'webm';
  audio: boolean;
}

type LoadedTimelineClip =
  | (TimelineExportPlanClip & {
      kind: 'image';
      element: HTMLImageElement;
      width: number;
      height: number;
    })
  | (TimelineExportPlanClip & {
      kind: 'video';
      element: HTMLVideoElement;
      width: number;
      height: number;
    });

const safeFileName = (value: string): string =>
  value.trim().replace(/[\\/:*?"<>|]+/g, '-').slice(0, 120) || 'timeline';

const aborted = (): DOMException =>
  new DOMException('Timeline export was canceled', 'AbortError');

const throwIfAborted = (signal?: AbortSignal): void => {
  if (signal?.aborted) throw aborted();
};

const finitePositive = (value: number | null | undefined): number | null =>
  typeof value === 'number' && Number.isFinite(value) && value > 0
    ? value
    : null;

const evenDimension = (value: number): number => {
  const rounded = Math.max(2, Math.round(value));
  return rounded % 2 === 0 ? rounded : rounded - 1;
};

export function timelineExportSize(
  width: number,
  height: number,
  maxEdge = MAX_EXPORT_EDGE
): { width: number; height: number } {
  const safeWidth = finitePositive(width) ?? 1_280;
  const safeHeight = finitePositive(height) ?? 720;
  const scale = Math.min(1, maxEdge / Math.max(safeWidth, safeHeight));
  return {
    width: evenDimension(safeWidth * scale),
    height: evenDimension(safeHeight * scale),
  };
}

export function buildTimelineExportPlan(
  data: CreativeTimelineNodeData,
  assets: ReadonlyMap<string, CreativeTimelineAssetPresentation>
): TimelineExportPlan {
  const durationMs = timelineDurationMs(data.clips);
  if (data.clips.length === 0 || durationMs <= 0) {
    throw new TimelineExportError('empty', 'Timeline has no clips to export');
  }

  const clips = data.clips.map((clip): TimelineExportPlanClip => {
    const asset = assets.get(clip.assetId);
    if (
      !asset ||
      asset.deleted ||
      !asset.src.trim() ||
      asset.kind !== clip.kind
    ) {
      throw new TimelineExportError(
        'asset-unavailable',
        `Timeline clip ${clip.id} references an unavailable ${clip.kind} asset`
      );
    }
    return { clip: structuredClone(clip), asset: { ...asset } };
  });

  return {
    clips: clips.sort(
      (left, right) =>
        left.clip.startMs - right.clip.startMs ||
        data.clips.findIndex((clip) => clip.id === left.clip.id) -
          data.clips.findIndex((clip) => clip.id === right.clip.id)
    ),
    durationMs,
  };
}

export function selectTimelineExportMimeType(
  isTypeSupported: (mimeType: string) => boolean,
  includeAudio = false
): string | null {
  return selectTimelineExportFormat(isTypeSupported, includeAudio)?.mimeType ?? null;
}

export function selectTimelineExportFormat(
  isTypeSupported: (mimeType: string) => boolean,
  includeAudio = false
): TimelineExportFormat | null {
  const match = EXPORT_FORMATS.find(
    (format) => (!includeAudio || format.audio) && isTypeSupported(format.mimeType)
  );
  return match ? { ...match } : null;
}

const waitForMedia = (
  target: EventTarget,
  successEvent: string,
  errorEvent: string,
  signal?: AbortSignal
): Promise<void> =>
  new Promise((resolve, reject) => {
    throwIfAborted(signal);
    const timeout = window.setTimeout(() => {
      cleanup();
      reject(
        new TimelineExportError(
          'media-load-failed',
          'Timed out while loading timeline media'
        )
      );
    }, MEDIA_LOAD_TIMEOUT_MS);
    const succeed = () => {
      cleanup();
      resolve();
    };
    const fail = () => {
      cleanup();
      reject(
        new TimelineExportError(
          'media-load-failed',
          'A timeline media asset could not be decoded'
        )
      );
    };
    const cancel = () => {
      cleanup();
      reject(aborted());
    };
    const cleanup = () => {
      window.clearTimeout(timeout);
      target.removeEventListener(successEvent, succeed);
      target.removeEventListener(errorEvent, fail);
      signal?.removeEventListener('abort', cancel);
    };
    target.addEventListener(successEvent, succeed, { once: true });
    target.addEventListener(errorEvent, fail, { once: true });
    signal?.addEventListener('abort', cancel, { once: true });
  });

const loadTimelineClip = async (
  item: TimelineExportPlanClip,
  signal?: AbortSignal
): Promise<LoadedTimelineClip> => {
  throwIfAborted(signal);
  if (item.clip.kind === 'image') {
    const image = new Image();
    image.crossOrigin = 'anonymous';
    image.decoding = 'async';
    const loaded = waitForMedia(image, 'load', 'error', signal);
    image.src = item.asset.src;
    await loaded;
    throwIfAborted(signal);
    return {
      ...item,
      kind: 'image',
      element: image,
      width: image.naturalWidth,
      height: image.naturalHeight,
    };
  }

  const video = document.createElement('video');
  video.crossOrigin = 'anonymous';
  video.preload = 'auto';
  video.playsInline = true;
  video.volume = 1;
  const loaded = waitForMedia(video, 'loadeddata', 'error', signal);
  video.src = item.asset.src;
  video.load();
  await loaded;
  throwIfAborted(signal);
  const sourceStart = Math.max(0, item.clip.sourceStartMs / 1_000);
  if (sourceStart > 0.001) {
    const seeked = waitForMedia(video, 'seeked', 'error', signal);
    video.currentTime = sourceStart;
    await seeked;
  }
  return {
    ...item,
    kind: 'video',
    element: video,
    width: video.videoWidth,
    height: video.videoHeight,
  };
};

const drawContained = (
  context: CanvasRenderingContext2D,
  source: CanvasImageSource,
  sourceWidth: number,
  sourceHeight: number,
  targetWidth: number,
  targetHeight: number
): void => {
  context.fillStyle = '#000';
  context.fillRect(0, 0, targetWidth, targetHeight);
  const scale = Math.min(targetWidth / sourceWidth, targetHeight / sourceHeight);
  const width = sourceWidth * scale;
  const height = sourceHeight * scale;
  context.drawImage(
    source,
    (targetWidth - width) / 2,
    (targetHeight - height) / 2,
    width,
    height
  );
};

const activeLoadedClip = (
  clips: readonly LoadedTimelineClip[],
  timeMs: number
): LoadedTimelineClip | null => {
  for (let index = clips.length - 1; index >= 0; index -= 1) {
    const item = clips[index];
    if (
      item &&
      timeMs >= item.clip.startMs &&
      timeMs < item.clip.startMs + item.clip.durationMs
    ) {
      return item;
    }
  }
  return null;
};

const createAudioContext = (): AudioContext | null => {
  const AudioContextCtor =
    typeof AudioContext !== 'undefined'
      ? AudioContext
      : (window as Window & { webkitAudioContext?: typeof AudioContext })
          .webkitAudioContext;
  return AudioContextCtor ? new AudioContextCtor() : null;
};

const recordedBlob = (
  recorder: MediaRecorder,
  chunks: Blob[]
): Promise<Blob> =>
  new Promise((resolve, reject) => {
    recorder.addEventListener('error', () => {
      reject(
        new TimelineExportError(
          'recording-failed',
          'The browser failed while recording the composed timeline'
        )
      );
    }, { once: true });
    recorder.addEventListener('stop', () => {
      const blob = new Blob(chunks, {
        type: recorder.mimeType || 'video/webm',
      });
      if (blob.size <= 0) {
        reject(
          new TimelineExportError(
            'recording-failed',
            'The composed timeline recording was empty'
          )
        );
        return;
      }
      resolve(blob);
    }, { once: true });
  });

export async function exportTimelineComposition(
  options: TimelineCompositionExportOptions
): Promise<TimelineCompositionExportResult> {
  const plan = buildTimelineExportPlan(options.data, options.assets);
  const fps = Math.min(60, Math.max(12, Math.round(options.fps ?? DEFAULT_EXPORT_FPS)));
  throwIfAborted(options.signal);

  if (
    typeof MediaRecorder === 'undefined' ||
    typeof HTMLCanvasElement.prototype.captureStream !== 'function'
  ) {
    throw new TimelineExportError(
      'recording-unsupported',
      'This browser cannot record a composed timeline'
    );
  }

  const needsAudio =
    !options.data.muted && plan.clips.some((item) => item.clip.kind === 'video');
  const format = selectTimelineExportFormat(
    (candidate) =>
      typeof MediaRecorder.isTypeSupported !== 'function' ||
      MediaRecorder.isTypeSupported(candidate),
    needsAudio
  );
  if (!format) {
    throw new TimelineExportError(
      'recording-unsupported',
      'This browser has no supported timeline video encoder'
    );
  }
  const { mimeType } = format;
  const audioContext = needsAudio ? createAudioContext() : null;
  if (needsAudio && !audioContext) {
    throw new TimelineExportError(
      'audio-unsupported',
      'This browser cannot mix video audio into a timeline export'
    );
  }

  const audioDestination = audioContext?.createMediaStreamDestination() ?? null;
  const audioSources: MediaElementAudioSourceNode[] = [];
  let loaded: LoadedTimelineClip[] = [];
  let canvasStream: MediaStream | null = null;
  let outputStream: MediaStream | null = null;
  let recorder: MediaRecorder | null = null;
  let recordingResult: Promise<Blob> | null = null;
  let recorderStarted = false;
  let animationFrame: number | null = null;

  try {
    if (audioContext?.state === 'suspended') await audioContext.resume();
    loaded = await Promise.all(
      plan.clips.map((item) => loadTimelineClip(item, options.signal))
    );
    throwIfAborted(options.signal);

    if (audioContext && audioDestination) {
      for (const item of loaded) {
        if (item.kind !== 'video') continue;
        const source = audioContext.createMediaElementSource(item.element);
        source.connect(audioDestination);
        audioSources.push(source);
      }
    }

    const first = loaded[0];
    if (!first || first.width <= 0 || first.height <= 0) {
      throw new TimelineExportError(
        'media-load-failed',
        'Timeline media has no usable dimensions'
      );
    }
    const size = timelineExportSize(
      finitePositive(first.asset.width) ?? first.width,
      finitePositive(first.asset.height) ?? first.height
    );
    const canvas = document.createElement('canvas');
    canvas.width = size.width;
    canvas.height = size.height;
    const context = canvas.getContext('2d', { alpha: false });
    if (!context) {
      throw new TimelineExportError(
        'render-unsupported',
        'This browser cannot create the timeline render surface'
      );
    }

    canvasStream = canvas.captureStream(fps);
    const tracks = [
      ...canvasStream.getVideoTracks(),
      ...(audioDestination?.stream.getAudioTracks() ?? []),
    ];
    outputStream = new MediaStream(tracks);
    const bitsPerSecond = Math.max(
      2_000_000,
      Math.min(12_000_000, size.width * size.height * 5)
    );
    recorder = new MediaRecorder(outputStream, {
      mimeType,
      videoBitsPerSecond: bitsPerSecond,
      ...(audioDestination ? { audioBitsPerSecond: 192_000 } : {}),
    });
    const chunks: Blob[] = [];
    recorder.addEventListener('dataavailable', (event) => {
      if (event.data.size > 0) chunks.push(event.data);
    });
    recordingResult = recordedBlob(recorder, chunks);
    void recordingResult.catch(() => undefined);

    context.fillStyle = '#000';
    context.fillRect(0, 0, size.width, size.height);
    recorder.start(250);
    recorderStarted = true;
    const startedAt = performance.now();
    let current: LoadedTimelineClip | null = null;

    await new Promise<void>((resolve, reject) => {
      const cancel = () => {
        if (animationFrame !== null) cancelAnimationFrame(animationFrame);
        reject(aborted());
      };
      options.signal?.addEventListener('abort', cancel, { once: true });

      const render = (now: number) => {
        try {
          throwIfAborted(options.signal);
          const timeMs = Math.min(plan.durationMs, now - startedAt);
          const active = activeLoadedClip(loaded, timeMs);
          if (active !== current) {
            if (current?.kind === 'video') current.element.pause();
            current = active;
            if (current?.kind === 'video') {
              const expected =
                (current.clip.sourceStartMs + timeMs - current.clip.startMs) /
                1_000;
              current.element.currentTime = Math.max(0, expected);
              void current.element.play().catch(() => undefined);
            }
          }

          if (!current) {
            context.fillStyle = '#000';
            context.fillRect(0, 0, size.width, size.height);
          } else {
            if (current.kind === 'video') {
              const expected =
                (current.clip.sourceStartMs + timeMs - current.clip.startMs) /
                1_000;
              if (Math.abs(current.element.currentTime - expected) > 0.5) {
                current.element.currentTime = Math.max(0, expected);
              }
            }
            drawContained(
              context,
              current.element,
              current.width,
              current.height,
              size.width,
              size.height
            );
          }

          options.onProgress?.(Math.min(1, timeMs / plan.durationMs));
          if (timeMs >= plan.durationMs) {
            options.signal?.removeEventListener('abort', cancel);
            resolve();
            return;
          }
          animationFrame = requestAnimationFrame(render);
        } catch (error) {
          options.signal?.removeEventListener('abort', cancel);
          reject(error);
        }
      };
      animationFrame = requestAnimationFrame(render);
    });

    for (const item of loaded) {
      if (item.kind === 'video') item.element.pause();
    }
    if (recorder.state !== 'inactive') recorder.stop();
    const blob = await recordingResult;
    options.onProgress?.(1);
    return {
      blob,
      mimeType: blob.type || mimeType,
      extension: format.extension,
      width: size.width,
      height: size.height,
      durationMs: plan.durationMs,
    };
  } catch (error) {
    if (recorder && recorder.state !== 'inactive') recorder.stop();
    if (recordingResult && recorderStarted) {
      await recordingResult.catch(() => undefined);
    }
    throw error;
  } finally {
    if (animationFrame !== null) cancelAnimationFrame(animationFrame);
    for (const item of loaded) {
      if (item.kind === 'video') {
        item.element.pause();
        item.element.removeAttribute('src');
        item.element.load();
      }
    }
    for (const source of audioSources) source.disconnect();
    for (const track of outputStream?.getTracks() ?? []) track.stop();
    for (const track of canvasStream?.getTracks() ?? []) track.stop();
    if (audioContext && audioContext.state !== 'closed') {
      await audioContext.close().catch(() => undefined);
    }
  }
}

export function downloadTimelineComposition(
  result: TimelineCompositionExportResult,
  title: string
): void {
  const url = URL.createObjectURL(result.blob);
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = `${safeFileName(title)}.${result.extension}`;
  anchor.rel = 'noopener noreferrer';
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  window.setTimeout(() => URL.revokeObjectURL(url), 1_000);
}
