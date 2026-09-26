/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  Add,
  CloseOne,
  Delete,
  Download,
  FolderOpen,
  FullScreen,
  Left,
  Loading,
  OffScreen,
  Pause,
  Pic,
  Play,
  Right,
  Upload,
  VideoTwo,
  VolumeMute,
  VolumeUp,
} from '@icon-park/react';
import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { useTranslation } from 'react-i18next';

import type { CreativeTimelineNodeData } from '../../domain';
import CreativeNodeFrame from './CreativeNodeFrame';
import {
  downloadTimelineComposition,
  exportTimelineComposition,
  TimelineExportError,
  type CreativeTimelineAssetPresentation,
} from './timelineExport';
import type { CreativeNodePresentationProps } from './types';
import {
  moveTimelineClip,
  removeTimelineClip,
  resolveTimelineClipDuration,
  timelineClipAtTime,
  timelineDurationMs,
  timelineScaleDurationMs,
  timelineTickValues,
  trimTimelineClip,
} from './timelineModel';
import styles from './CreativeTimelineNode.module.css';

export type { CreativeTimelineAssetPresentation } from './timelineExport';

const iconProps = {
  theme: 'outline' as const,
  size: 15,
  fill: 'currentColor',
  strokeWidth: 3,
};

interface CreativeTimelineNodeProps
  extends CreativeNodePresentationProps<'timeline'> {
  assets: ReadonlyMap<string, CreativeTimelineAssetPresentation>;
  libraryAssets?: readonly CreativeTimelineAssetPresentation[];
  libraryLoading?: boolean;
  onChange?(data: CreativeTimelineNodeData, mergeKey?: string): void;
  onDelete?(): void;
  onAddAsset?(assetId: string): void;
  onRequestAssets?(popupContainer: HTMLElement | null): void;
  onUploadFiles?(files: readonly File[]): void | Promise<void>;
}

type ClipGesture = {
  pointerId: number;
  clipId: string;
  mode: 'move' | 'trim-start' | 'trim-end';
  clientX: number;
  data: CreativeTimelineNodeData;
};

const formatTime = (milliseconds: number): string => {
  const totalSeconds = Math.max(0, Math.floor(milliseconds / 1_000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes.toString().padStart(2, '0')}:${seconds
    .toString()
    .padStart(2, '0')}`;
};

const hasDraggedFiles = (dataTransfer: DataTransfer): boolean =>
  Array.from(dataTransfer.types).includes('Files');

const TIMELINE_ASSET_DRAG_TYPE = 'application/x-nomifun-timeline-asset';

const hasDraggedTimelineAsset = (dataTransfer: DataTransfer): boolean =>
  Array.from(dataTransfer.types).includes(TIMELINE_ASSET_DRAG_TYPE);

const mediaAspectRatio = (
  media: Pick<CreativeTimelineAssetPresentation, 'width' | 'height'> | null,
  fallback: { width: number; height: number } | null
): number => {
  const width = media?.width && media.width > 0
    ? media.width
    : fallback?.width ?? 0;
  const height = media?.height && media.height > 0
    ? media.height
    : fallback?.height ?? 0;
  return Number.isFinite(width) && Number.isFinite(height) && width > 0 && height > 0
    ? width / height
    : 16 / 9;
};

const CreativeTimelineNode: React.FC<CreativeTimelineNodeProps> = ({
  node,
  assets,
  libraryAssets = [],
  libraryLoading = false,
  selected,
  placement,
  runtime,
  className,
  style,
  inputHandle,
  outputHandle,
  onActivate,
  onOpen,
  onToggleLock,
  onPointerDown,
  onContextMenu,
  onChange,
  onDelete,
  onAddAsset,
  onRequestAssets,
  onUploadFiles,
}) => {
  const { t } = useTranslation();
  const rootRef = useRef<HTMLDivElement>(null);
  const trackRef = useRef<HTMLDivElement>(null);
  const previewVideoRef = useRef<HTMLVideoElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const gestureRef = useRef<ClipGesture | null>(null);
  const seekPointerRef = useRef<number | null>(null);
  const animationRef = useRef<number | null>(null);
  const exportAbortRef = useRef<AbortController | null>(null);
  const exportSequenceRef = useRef(0);
  const lastExportPercentRef = useRef(-1);
  const currentTimeRef = useRef(0);
  const [currentTimeMs, setCurrentTimeMs] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [selectedClipId, setSelectedClipId] = useState<string | null>(null);
  const [fullscreen, setFullscreen] = useState(false);
  const [addMenuOpen, setAddMenuOpen] = useState(false);
  const [assetPanelCollapsed, setAssetPanelCollapsed] = useState(false);
  const [projectMediaSize, setProjectMediaSize] = useState<{
    width: number;
    height: number;
  } | null>(null);
  const [exportProgress, setExportProgress] = useState<number | null>(null);
  const [exportError, setExportError] = useState<string | null>(null);

  const totalDurationMs = useMemo(
    () => timelineDurationMs(node.data.clips),
    [node.data.clips]
  );
  const scaleDurationMs = useMemo(
    () => timelineScaleDurationMs(node.data.clips),
    [node.data.clips]
  );
  const ticks = useMemo(
    () => timelineTickValues(scaleDurationMs),
    [scaleDurationMs]
  );
  const minorTicks = useMemo(
    () => Array.from(
      { length: Math.floor(scaleDurationMs / 1_000) + 1 },
      (_, index) => index * 1_000
    ).filter((value) => value % 5_000 !== 0),
    [scaleDurationMs]
  );
  const firstClip = useMemo(
    () => [...node.data.clips].sort(
      (left, right) =>
        left.startMs - right.startMs ||
        node.data.clips.indexOf(left) - node.data.clips.indexOf(right)
    )[0] ?? null,
    [node.data.clips]
  );
  const firstAsset = firstClip ? assets.get(firstClip.assetId) ?? null : null;
  const projectAspectRatio = mediaAspectRatio(firstAsset, projectMediaSize);
  const activeClip = useMemo(
    () => timelineClipAtTime(node.data.clips, currentTimeMs),
    [currentTimeMs, node.data.clips]
  );
  const activeAsset = activeClip ? assets.get(activeClip.assetId) ?? null : null;

  useEffect(() => {
    setProjectMediaSize(null);
  }, [firstAsset?.assetId, firstAsset?.src]);

  const commit = useCallback(
    (data: CreativeTimelineNodeData, mergeKey?: string) => {
      if (node.locked) return;
      onChange?.(data, mergeKey);
    },
    [node.locked, onChange]
  );

  const recordProjectMediaSize = useCallback(
    (assetId: string, width: number, height: number) => {
      if (
        assetId !== firstAsset?.assetId ||
        !Number.isFinite(width) ||
        !Number.isFinite(height) ||
        width <= 0 ||
        height <= 0
      ) return;
      setProjectMediaSize((current) =>
        current?.width === width && current.height === height
          ? current
          : { width, height }
      );
    },
    [firstAsset?.assetId]
  );

  const setTime = useCallback(
    (value: number) => {
      const next = Math.min(Math.max(0, value), scaleDurationMs);
      currentTimeRef.current = next;
      setCurrentTimeMs(next);
    },
    [scaleDurationMs]
  );

  useEffect(() => {
    currentTimeRef.current = currentTimeMs;
  }, [currentTimeMs]);

  useEffect(() => {
    if (currentTimeRef.current <= scaleDurationMs) return;
    setTime(scaleDurationMs);
  }, [scaleDurationMs, setTime]);

  useEffect(() => {
    if (!playing || totalDurationMs <= 0) return;
    const startedAt = performance.now();
    const initialTime = currentTimeRef.current >= totalDurationMs
      ? 0
      : currentTimeRef.current;
    if (initialTime !== currentTimeRef.current) setTime(initialTime);

    const tick = (now: number) => {
      const next = initialTime + now - startedAt;
      if (next >= totalDurationMs) {
        setTime(totalDurationMs);
        setPlaying(false);
        return;
      }
      setTime(next);
      animationRef.current = requestAnimationFrame(tick);
    };
    animationRef.current = requestAnimationFrame(tick);
    return () => {
      if (animationRef.current !== null) cancelAnimationFrame(animationRef.current);
      animationRef.current = null;
    };
  }, [playing, setTime, totalDurationMs]);

  useEffect(() => {
    const video = previewVideoRef.current;
    if (!video || !activeClip || activeClip.kind !== 'video' || !activeAsset?.src) return;
    const sync = () => {
      const sourceTime =
        (activeClip.sourceStartMs + Math.max(0, currentTimeRef.current - activeClip.startMs)) /
        1_000;
      if (Number.isFinite(sourceTime) && Math.abs(video.currentTime - sourceTime) > 0.35) {
        video.currentTime = sourceTime;
      }
      video.muted = node.data.muted;
      if (playing) void video.play().catch(() => undefined);
      else video.pause();
    };
    if (video.readyState >= 1) sync();
    else video.addEventListener('loadedmetadata', sync, { once: true });
    return () => video.removeEventListener('loadedmetadata', sync);
  }, [activeAsset?.src, activeClip, node.data.muted, playing]);

  useEffect(() => {
    if (playing || !activeClip || activeClip.kind !== 'video') return;
    const video = previewVideoRef.current;
    if (!video || video.readyState < 1) return;
    const sourceTime =
      (activeClip.sourceStartMs + Math.max(0, currentTimeMs - activeClip.startMs)) /
      1_000;
    if (Number.isFinite(sourceTime)) video.currentTime = sourceTime;
  }, [activeClip, currentTimeMs, playing]);

  useEffect(() => {
    const onFullscreenChange = () => {
      const nextFullscreen = document.fullscreenElement === rootRef.current;
      setFullscreen(nextFullscreen);
      if (!nextFullscreen) setAddMenuOpen(false);
    };
    document.addEventListener('fullscreenchange', onFullscreenChange);
    return () => document.removeEventListener('fullscreenchange', onFullscreenChange);
  }, []);

  useEffect(
    () => () => {
      exportSequenceRef.current += 1;
      exportAbortRef.current?.abort();
      exportAbortRef.current = null;
    },
    []
  );

  const beginGesture = (
    event: React.PointerEvent<HTMLElement>,
    clipId: string,
    mode: ClipGesture['mode']
  ) => {
    if (node.locked || event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();
    event.currentTarget.setPointerCapture(event.pointerId);
    gestureRef.current = {
      pointerId: event.pointerId,
      clipId,
      mode,
      clientX: event.clientX,
      data: structuredClone(node.data),
    };
    setSelectedClipId(clipId);
  };

  const updateGesture = (event: React.PointerEvent<HTMLElement>) => {
    const gesture = gestureRef.current;
    if (!gesture || gesture.pointerId !== event.pointerId) return;
    const width = trackRef.current?.getBoundingClientRect().width ?? 0;
    if (width <= 0) return;
    event.preventDefault();
    event.stopPropagation();
    const deltaMs = Math.round(
      (((event.clientX - gesture.clientX) / width) * scaleDurationMs) / 50
    ) * 50;
    const original = gesture.data.clips.find((clip) => clip.id === gesture.clipId);
    if (!original) return;
    const next = gesture.mode === 'move'
      ? moveTimelineClip(gesture.data, gesture.clipId, original.startMs + deltaMs)
      : trimTimelineClip(
          gesture.data,
          gesture.clipId,
          gesture.mode === 'trim-start' ? 'start' : 'end',
          deltaMs
        );
    commit(next, `timeline:${node.id}:${gesture.clipId}:${gesture.mode}`);
  };

  const finishGesture = (event: React.PointerEvent<HTMLElement>) => {
    const gesture = gestureRef.current;
    if (!gesture || gesture.pointerId !== event.pointerId) return;
    event.preventDefault();
    event.stopPropagation();
    gestureRef.current = null;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };

  const setTimeFromPointer = (clientX: number) => {
    const rect = trackRef.current?.getBoundingClientRect();
    if (!rect || rect.width <= 0) return;
    setTime(((clientX - rect.left) / rect.width) * scaleDurationMs);
  };

  const beginSeek = (event: React.PointerEvent<HTMLElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();
    setPlaying(false);
    seekPointerRef.current = event.pointerId;
    event.currentTarget.setPointerCapture?.(event.pointerId);
    setTimeFromPointer(event.clientX);
  };

  const updateSeek = (event: React.PointerEvent<HTMLElement>) => {
    if (seekPointerRef.current !== event.pointerId) return;
    event.preventDefault();
    event.stopPropagation();
    setTimeFromPointer(event.clientX);
  };

  const finishSeek = (event: React.PointerEvent<HTMLElement>) => {
    if (seekPointerRef.current !== event.pointerId) return;
    event.preventDefault();
    event.stopPropagation();
    setTimeFromPointer(event.clientX);
    seekPointerRef.current = null;
    if (event.currentTarget.hasPointerCapture?.(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };

  const toggleFullscreen = async () => {
    try {
      if (document.fullscreenElement === rootRef.current) {
        await document.exitFullscreen();
      } else {
        await rootRef.current?.requestFullscreen();
      }
    } catch {
      // The browser owns fullscreen permission; keeping the editor usable is enough.
    }
  };

  const exportTimeline = async () => {
    if (exportAbortRef.current) {
      exportAbortRef.current.abort();
      return;
    }
    if (node.data.clips.length === 0 || totalDurationMs <= 0) return;

    const sequence = ++exportSequenceRef.current;
    const controller = new AbortController();
    exportAbortRef.current = controller;
    lastExportPercentRef.current = 0;
    setPlaying(false);
    setExportError(null);
    setExportProgress(0);
    try {
      const result = await exportTimelineComposition({
        data: structuredClone(node.data),
        assets: new Map(assets),
        signal: controller.signal,
        onProgress: (progress) => {
          const percent = Math.round(progress * 100);
          if (
            exportSequenceRef.current === sequence &&
            lastExportPercentRef.current !== percent
          ) {
            lastExportPercentRef.current = percent;
            setExportProgress(percent / 100);
          }
        },
      });
      if (exportSequenceRef.current !== sequence || controller.signal.aborted) return;
      downloadTimelineComposition(result, node.data.title);
    } catch (error) {
      if (controller.signal.aborted || (error instanceof DOMException && error.name === 'AbortError')) {
        return;
      }
      const code = error instanceof TimelineExportError ? error.code : null;
      const message = code === 'asset-unavailable'
        ? t('creativeStudio.canvas.timeline.exportErrors.assetUnavailable', {
            defaultValue: '时间线包含不可用素材，无法导出。',
          })
        : code === 'recording-unsupported' || code === 'render-unsupported'
          ? t('creativeStudio.canvas.timeline.exportErrors.unsupported', {
              defaultValue: '当前浏览器不支持合成录制。',
            })
          : code === 'audio-unsupported'
            ? t('creativeStudio.canvas.timeline.exportErrors.audioUnsupported', {
                defaultValue: '当前浏览器无法混合视频原声。',
              })
            : code === 'media-load-failed'
              ? t('creativeStudio.canvas.timeline.exportErrors.mediaLoadFailed', {
                  defaultValue: '导出素材加载失败，请检查素材后重试。',
                })
              : t('creativeStudio.canvas.timeline.exportErrors.failed', {
                  defaultValue: '合成导出失败，请重试。',
                });
      setExportError(message);
    } finally {
      if (exportSequenceRef.current === sequence) {
        exportAbortRef.current = null;
        setExportProgress(null);
      }
    }
  };

  const removeSelectedClip = () => {
    if (!selectedClipId) return;
    commit(removeTimelineClip(node.data, selectedClipId));
    setSelectedClipId(null);
  };

  const requestAssets = () => {
    setAddMenuOpen(false);
    onRequestAssets?.(
      document.fullscreenElement === rootRef.current ? rootRef.current : null
    );
  };

  const addLibraryAsset = (assetId: string) => {
    if (node.locked || !onAddAsset) return;
    setAddMenuOpen(false);
    onAddAsset(assetId);
  };

  const title = node.data.title || t('creativeStudio.canvas.nodeKinds.timeline');
  const exporting = exportProgress !== null;
  const exportPercent = Math.round((exportProgress ?? 0) * 100);
  const frameActions = onDelete ? (
    <button
      type='button'
      className={styles.headerButton}
      disabled={node.locked}
      aria-label={t('creativeStudio.canvas.timeline.close', {
        defaultValue: '删除时间线节点',
      })}
      title={t('creativeStudio.canvas.timeline.close', {
        defaultValue: '删除时间线节点',
      })}
      onPointerDown={(event) => event.stopPropagation()}
      onClick={(event) => {
        event.stopPropagation();
        onDelete();
      }}
    >
      <CloseOne {...iconProps} />
    </button>
  ) : null;

  return (
    <CreativeNodeFrame
      node={node}
      title={title}
      selected={selected}
      placement={placement}
      runtime={runtime}
      className={className}
      style={style}
      headerActions={frameActions}
      inputHandle={inputHandle}
      outputHandle={outputHandle}
      onActivate={onActivate ? () => onActivate(node) : undefined}
      onOpen={onOpen ? () => onOpen(node) : undefined}
      onToggleLock={onToggleLock ? () => onToggleLock(node) : undefined}
      onPointerDown={onPointerDown}
      onContextMenu={onContextMenu}
    >
      <div
        ref={rootRef}
        className={styles.timeline}
        data-timeline-node
        tabIndex={0}
        onPointerDown={(event) => {
          event.stopPropagation();
          if (
            addMenuOpen &&
            event.target instanceof Element &&
            !event.target.closest('[data-timeline-add-surface]')
          ) {
            setAddMenuOpen(false);
          }
          onActivate?.(node);
        }}
        onClick={(event) => event.stopPropagation()}
        onDoubleClick={(event) => event.stopPropagation()}
        onContextMenu={(event) => event.stopPropagation()}
        onKeyDown={(event) => {
          if ((event.key === 'Delete' || event.key === 'Backspace') && selectedClipId) {
            event.preventDefault();
            removeSelectedClip();
          }
        }}
        onDragOver={(event) => {
          if (node.locked) return;
          const acceptsFiles = Boolean(onUploadFiles && hasDraggedFiles(event.dataTransfer));
          const acceptsAsset = Boolean(
            onAddAsset && hasDraggedTimelineAsset(event.dataTransfer)
          );
          if (!acceptsFiles && !acceptsAsset) return;
          event.preventDefault();
          event.stopPropagation();
          event.dataTransfer.dropEffect = 'copy';
        }}
        onDrop={(event) => {
          if (node.locked) return;
          const assetId = event.dataTransfer.getData(TIMELINE_ASSET_DRAG_TYPE);
          if (assetId && onAddAsset) {
            event.preventDefault();
            event.stopPropagation();
            addLibraryAsset(assetId);
            return;
          }
          if (!onUploadFiles || !hasDraggedFiles(event.dataTransfer)) return;
          event.preventDefault();
          event.stopPropagation();
          const files = Array.from(event.dataTransfer.files).filter(
            (file) => file.type.startsWith('image/') || file.type.startsWith('video/')
          );
          if (files.length > 0) void onUploadFiles(files);
        }}
      >
        <input
          ref={fileInputRef}
          className={styles.fileInput}
          type='file'
          accept='image/*,video/*'
          multiple
          tabIndex={-1}
          aria-hidden='true'
          onChange={(event) => {
            const files = Array.from(event.currentTarget.files ?? []);
            event.currentTarget.value = '';
            setAddMenuOpen(false);
            if (files.length > 0) void onUploadFiles?.(files);
          }}
        />
        <div className={styles.fullscreenHeader}>
          <strong>{title}</strong>
          <span>{formatTime(currentTimeMs)} / {formatTime(totalDurationMs)}</span>
        </div>

        <div
          className={styles.previewWorkspace}
          data-asset-panel-collapsed={assetPanelCollapsed || undefined}
        >
          <aside className={styles.assetPanel} aria-label={t(
            'creativeStudio.canvas.timeline.assetLibrary',
            { defaultValue: '资产库' }
          )}>
            <div className={styles.assetPanelHeader}>
              <strong>{t('creativeStudio.canvas.timeline.assetLibrary', {
                defaultValue: '资产库',
              })}</strong>
              <button
                type='button'
                className={styles.assetPanelToggle}
                aria-label={assetPanelCollapsed
                  ? t('creativeStudio.canvas.timeline.expandAssets', {
                      defaultValue: '展开素材区',
                    })
                  : t('creativeStudio.canvas.timeline.collapseAssets', {
                      defaultValue: '收起素材区',
                    })}
                title={assetPanelCollapsed
                  ? t('creativeStudio.canvas.timeline.expandAssets', {
                      defaultValue: '展开素材区',
                    })
                  : t('creativeStudio.canvas.timeline.collapseAssets', {
                      defaultValue: '收起素材区',
                    })}
                onClick={(event) => {
                  event.stopPropagation();
                  setAssetPanelCollapsed((value) => !value);
                }}
              >
                {assetPanelCollapsed ? <Right {...iconProps} /> : <Left {...iconProps} />}
              </button>
            </div>
            <div className={styles.assetGrid}>
              {libraryAssets.map((asset) => (
                <button
                  key={asset.assetId}
                  type='button'
                  className={styles.assetCard}
                  disabled={node.locked || !onAddAsset}
                  draggable={!node.locked && Boolean(onAddAsset)}
                  aria-label={t('creativeStudio.canvas.timeline.addLibraryAsset', {
                    title: asset.title,
                    defaultValue: '添加素材 {{title}} 到时间线',
                  })}
                  title={asset.title}
                  onDragStart={(event) => {
                    event.stopPropagation();
                    event.dataTransfer.effectAllowed = 'copy';
                    event.dataTransfer.setData(TIMELINE_ASSET_DRAG_TYPE, asset.assetId);
                  }}
                  onClick={(event) => {
                    event.stopPropagation();
                    addLibraryAsset(asset.assetId);
                  }}
                >
                  <span className={styles.assetCardMedia} aria-hidden='true'>
                    {asset.thumbnailSrc ? (
                      <img src={asset.thumbnailSrc} alt='' draggable={false} />
                    ) : asset.kind === 'image' && asset.src ? (
                      <img src={asset.src} alt='' draggable={false} />
                    ) : asset.kind === 'video' && asset.src ? (
                      <video
                        src={asset.src}
                        muted
                        playsInline
                        preload='metadata'
                        onLoadedMetadata={(event) => {
                          const video = event.currentTarget;
                          video.currentTime = Math.min(0.01, video.duration || 0.01);
                        }}
                      />
                    ) : asset.kind === 'image' ? (
                      <Pic {...iconProps} />
                    ) : (
                      <VideoTwo {...iconProps} />
                    )}
                  </span>
                  <span className={styles.assetCardTitle}>{asset.title}</span>
                </button>
              ))}
              {libraryLoading ? (
                <div className={styles.assetPanelState}>
                  <Loading className={styles.spin} {...iconProps} />
                  {t('creativeStudio.canvas.timeline.assetLibraryLoading', {
                    defaultValue: '正在读取资产库…',
                  })}
                </div>
              ) : libraryAssets.length === 0 ? (
                <div className={styles.assetPanelState}>
                  {t('creativeStudio.canvas.timeline.assetLibraryEmpty', {
                    defaultValue: '资产库暂无图片或视频',
                  })}
                </div>
              ) : null}
            </div>
          </aside>

          <div className={styles.previewStage}>
            <div
              className={styles.preview}
              data-project-canvas
              aria-live='off'
              style={{
                '--timeline-project-aspect': projectAspectRatio,
              } as React.CSSProperties}
            >
              {activeClip && activeAsset && !activeAsset.deleted ? (
                activeClip.kind === 'image' ? (
                  <img
                    key={activeClip.id}
                    src={activeAsset.src}
                    alt={activeAsset.title}
                    draggable={false}
                    onLoad={(event) => recordProjectMediaSize(
                      activeAsset.assetId,
                      event.currentTarget.naturalWidth,
                      event.currentTarget.naturalHeight
                    )}
                  />
                ) : (
                  <video
                    key={activeClip.id}
                    ref={previewVideoRef}
                    src={activeAsset.src}
                    poster={activeAsset.thumbnailSrc ?? undefined}
                    muted={node.data.muted}
                    playsInline
                    preload='metadata'
                    aria-label={activeAsset.title}
                    onLoadedMetadata={(event) => recordProjectMediaSize(
                      activeAsset.assetId,
                      event.currentTarget.videoWidth,
                      event.currentTarget.videoHeight
                    )}
                  />
                )
              ) : (
                <div className={styles.previewEmpty}>
                  {t('creativeStudio.canvas.timeline.previewEmpty', {
                    defaultValue: '移动时间指针以预览素材',
                  })}
                </div>
              )}
            </div>
          </div>

          {firstClip && firstAsset &&
          (!firstAsset.width || !firstAsset.height) ? (
            firstClip.kind === 'image' ? (
              <img
                className={styles.metadataMedia}
                src={firstAsset.src}
                alt=''
                aria-hidden='true'
                onLoad={(event) => recordProjectMediaSize(
                  firstAsset.assetId,
                  event.currentTarget.naturalWidth,
                  event.currentTarget.naturalHeight
                )}
              />
            ) : (
              <video
                className={styles.metadataMedia}
                src={firstAsset.src}
                muted
                preload='metadata'
                aria-hidden='true'
                onLoadedMetadata={(event) => recordProjectMediaSize(
                  firstAsset.assetId,
                  event.currentTarget.videoWidth,
                  event.currentTarget.videoHeight
                )}
              />
            )
          ) : null}
        </div>

        <div className={styles.controls}>
          <div className={styles.playbackControls}>
            <button
              type='button'
              className={styles.iconButton}
              disabled={totalDurationMs <= 0}
              aria-label={playing
                ? t('creativeStudio.canvas.timeline.pause', { defaultValue: '暂停' })
                : t('creativeStudio.canvas.timeline.play', { defaultValue: '播放' })}
              title={playing
                ? t('creativeStudio.canvas.timeline.pause', { defaultValue: '暂停' })
                : t('creativeStudio.canvas.timeline.play', { defaultValue: '播放' })}
              onClick={(event) => {
                event.stopPropagation();
                if (totalDurationMs <= 0) return;
                if (!playing && currentTimeRef.current >= totalDurationMs) setTime(0);
                setPlaying((value) => !value);
              }}
            >
              {playing ? <Pause {...iconProps} /> : <Play {...iconProps} />}
            </button>
            <span className={styles.timeReadout} aria-live='off'>
              <strong>{formatTime(currentTimeMs)}</strong>
              <span>/ {formatTime(totalDurationMs)}</span>
            </span>
          </div>
          <div className={styles.actionControls}>
            <button
              type='button'
              className={styles.iconButton}
              disabled={!exporting && totalDurationMs <= 0}
              aria-label={exporting
                ? t('creativeStudio.canvas.timeline.cancelExport', {
                    defaultValue: '取消导出',
                  })
                : t('creativeStudio.canvas.timeline.export', {
                    defaultValue: '导出合成视频',
                  })}
              title={exporting
                ? t('creativeStudio.canvas.timeline.exporting', {
                    percent: exportPercent,
                    defaultValue: '正在合成 {{percent}}%',
                  })
                : t('creativeStudio.canvas.timeline.export', {
                    defaultValue: '导出合成视频',
                  })}
              onClick={(event) => {
                event.stopPropagation();
                void exportTimeline();
              }}
            >
              {exporting ? (
                <Loading className={styles.spin} {...iconProps} />
              ) : (
                <Download {...iconProps} />
              )}
            </button>
            {exporting ? (
              <span className={styles.exportProgress} aria-live='polite'>
                {exportPercent}%
              </span>
            ) : null}
            <button
              type='button'
              className={styles.iconButton}
              aria-label={fullscreen
                ? t('creativeStudio.canvas.timeline.exitFullscreen', { defaultValue: '退出全屏编辑' })
                : t('creativeStudio.canvas.timeline.fullscreen', { defaultValue: '全屏编辑' })}
              title={fullscreen
                ? t('creativeStudio.canvas.timeline.exitFullscreen', { defaultValue: '退出全屏编辑' })
                : t('creativeStudio.canvas.timeline.fullscreen', { defaultValue: '全屏编辑' })}
              onClick={(event) => {
                event.stopPropagation();
                void toggleFullscreen();
              }}
            >
              {fullscreen ? <OffScreen {...iconProps} /> : <FullScreen {...iconProps} />}
            </button>
            <button
              type='button'
              className={styles.iconButton}
              disabled={node.locked}
              aria-pressed={node.data.muted}
              aria-label={node.data.muted
                ? t('creativeStudio.canvas.timeline.unmute', { defaultValue: '开启声音' })
                : t('creativeStudio.canvas.timeline.mute', { defaultValue: '关闭声音' })}
              title={node.data.muted
                ? t('creativeStudio.canvas.timeline.unmute', { defaultValue: '开启声音' })
                : t('creativeStudio.canvas.timeline.mute', { defaultValue: '关闭声音' })}
              onClick={(event) => {
                event.stopPropagation();
                commit({ ...node.data, muted: !node.data.muted });
              }}
            >
              {node.data.muted ? <VolumeMute {...iconProps} /> : <VolumeUp {...iconProps} />}
            </button>
          </div>
        </div>

        {exportError ? (
          <div className={styles.exportError} role='alert'>
            {exportError}
          </div>
        ) : null}

        <div className={styles.trackShell}>
          <div className={styles.trackLabel} aria-hidden='true'>
            {node.data.muted ? <VolumeMute {...iconProps} /> : <VolumeUp {...iconProps} />}
          </div>
          <div className={styles.trackViewport}>
            <div
              className={styles.ruler}
              onPointerDown={beginSeek}
              onPointerMove={updateSeek}
              onPointerUp={finishSeek}
              onPointerCancel={finishSeek}
            >
              {minorTicks.map((tick) => (
                <span
                  key={`minor-${tick}`}
                  className={styles.minorTick}
                  style={{ left: `${(tick / scaleDurationMs) * 100}%` }}
                  aria-hidden='true'
                />
              ))}
              {ticks.map((tick) => (
                <span
                  key={tick}
                  className={styles.tick}
                  style={{ left: `${(tick / scaleDurationMs) * 100}%` }}
                >
                  {formatTime(tick)}
                </span>
              ))}
            </div>
            <div
              ref={trackRef}
              className={styles.track}
              data-timeline-track
              data-empty={node.data.clips.length === 0 || undefined}
              onPointerDown={node.data.clips.length > 0 ? beginSeek : undefined}
              onPointerMove={updateSeek}
              onPointerUp={finishSeek}
              onPointerCancel={finishSeek}
            >
              {node.data.clips.length === 0 ? (
                <button
                  type='button'
                  className={styles.emptyTrack}
                  data-timeline-add-surface
                  disabled={node.locked || (!onRequestAssets && !onUploadFiles)}
                  onClick={(event) => {
                    event.stopPropagation();
                    setAddMenuOpen((value) => !value);
                  }}
                >
                  <Add {...iconProps} />
                  {t('creativeStudio.canvas.timeline.addAssets', {
                    defaultValue: '添加素材到时间线',
                  })}
                </button>
              ) : null}

              {node.data.clips.map((clip) => {
                const asset = assets.get(clip.assetId);
                const left = (clip.startMs / scaleDurationMs) * 100;
                const width = Math.max(1.2, (clip.durationMs / scaleDurationMs) * 100);
                const isSelected = selectedClipId === clip.id;
                return (
                  <div
                    key={clip.id}
                    className={styles.clip}
                    role='button'
                    tabIndex={0}
                    data-kind={clip.kind}
                    data-timeline-clip-id={clip.id}
                    data-selected={isSelected || undefined}
                    data-unavailable={!asset || asset.deleted || undefined}
                    aria-label={t('creativeStudio.canvas.timeline.clipLabel', {
                      title: asset?.title ?? clip.assetId,
                      duration: formatTime(clip.durationMs),
                      defaultValue: '{{title}}，时长 {{duration}}',
                    })}
                    style={{ left: `${left}%`, width: `${width}%` }}
                    title={asset?.title ?? clip.assetId}
                    onPointerDown={(event) => beginGesture(event, clip.id, 'move')}
                    onPointerMove={updateGesture}
                    onPointerUp={finishGesture}
                    onPointerCancel={finishGesture}
                    onClick={(event) => {
                      event.stopPropagation();
                      setSelectedClipId(clip.id);
                    }}
                    onKeyDown={(event) => {
                      if (node.locked) return;
                      if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') {
                        event.preventDefault();
                        event.stopPropagation();
                        const direction = event.key === 'ArrowLeft' ? -1 : 1;
                        commit(
                          moveTimelineClip(
                            node.data,
                            clip.id,
                            clip.startMs + direction * (event.shiftKey ? 1_000 : 100)
                          ),
                          `timeline:${node.id}:${clip.id}:keyboard-move`
                        );
                      }
                    }}
                  >
                    <button
                      type='button'
                      className={`${styles.trimHandle} ${styles.trimHandleStart}`}
                      aria-label={t('creativeStudio.canvas.timeline.trimStart', {
                        defaultValue: '裁切片段起点',
                      })}
                      disabled={node.locked}
                      onPointerDown={(event) => beginGesture(event, clip.id, 'trim-start')}
                      onPointerMove={updateGesture}
                      onPointerUp={finishGesture}
                      onPointerCancel={finishGesture}
                    />
                    <div className={styles.clipMedia} aria-hidden='true'>
                      {asset && !asset.deleted && asset.thumbnailSrc ? (
                        <img
                          src={asset.thumbnailSrc}
                          alt=''
                          draggable={false}
                        />
                      ) : asset && !asset.deleted && asset.kind === 'image' && asset.src ? (
                        <img src={asset.src} alt='' draggable={false} />
                      ) : asset && !asset.deleted && asset.kind === 'video' && asset.src ? (
                        <video
                          src={asset.src}
                          muted
                          playsInline
                          preload='metadata'
                          onLoadedMetadata={(event) => {
                            const video = event.currentTarget;
                            video.currentTime = Math.min(0.01, video.duration || 0.01);
                          }}
                        />
                      ) : clip.kind === 'image' ? (
                        <Pic {...iconProps} />
                      ) : (
                        <VideoTwo {...iconProps} />
                      )}
                    </div>
                    <span className={styles.clipTitle}>
                      {asset?.title ?? t('creativeStudio.canvas.timeline.assetUnavailable', {
                        defaultValue: '素材不可用',
                      })}
                    </span>
                    <span className={styles.clipDuration}>{formatTime(clip.durationMs)}</span>
                    {isSelected ? (
                      <button
                        type='button'
                        className={styles.removeClip}
                        disabled={node.locked}
                        aria-label={t('creativeStudio.canvas.timeline.removeClip', {
                          defaultValue: '移除片段',
                        })}
                        title={t('creativeStudio.canvas.timeline.removeClip', {
                          defaultValue: '移除片段',
                        })}
                        onPointerDown={(event) => event.stopPropagation()}
                        onClick={(event) => {
                          event.stopPropagation();
                          removeSelectedClip();
                        }}
                      >
                        <Delete {...iconProps} size={12} />
                      </button>
                    ) : null}
                    <button
                      type='button'
                      className={`${styles.trimHandle} ${styles.trimHandleEnd}`}
                      aria-label={t('creativeStudio.canvas.timeline.trimEnd', {
                        defaultValue: '裁切片段终点',
                      })}
                      disabled={node.locked}
                      onPointerDown={(event) => beginGesture(event, clip.id, 'trim-end')}
                      onPointerMove={updateGesture}
                      onPointerUp={finishGesture}
                      onPointerCancel={finishGesture}
                    />
                    {clip.kind === 'video' && asset?.src ? (
                      <video
                        className={styles.metadataVideo}
                        src={asset.src}
                        muted
                        preload='metadata'
                        onLoadedMetadata={(event) => {
                          const duration = event.currentTarget.duration * 1_000;
                          if (!Number.isFinite(duration) || duration <= 0) return;
                          const next = resolveTimelineClipDuration(node.data, clip.id, duration);
                          const resolved = next.clips.find((item) => item.id === clip.id);
                          if (
                            resolved?.sourceDurationMs !== clip.sourceDurationMs ||
                            resolved?.durationMs !== clip.durationMs
                          ) {
                            commit(next, `timeline:${node.id}:${clip.id}:metadata`);
                          }
                        }}
                      />
                    ) : null}
                  </div>
                );
              })}

              {node.data.clips.length > 0 ? (
                <button
                  type='button'
                  className={styles.addClip}
                  data-timeline-add-surface
                  disabled={node.locked || (!onRequestAssets && !onUploadFiles)}
                  aria-label={t('creativeStudio.canvas.timeline.addAssets', {
                    defaultValue: '添加素材到时间线',
                  })}
                  title={t('creativeStudio.canvas.timeline.addAssets', {
                    defaultValue: '添加素材到时间线',
                  })}
                  onPointerDown={(event) => event.stopPropagation()}
                  onClick={(event) => {
                    event.stopPropagation();
                    setAddMenuOpen((value) => !value);
                  }}
                  style={{
                    left: `${Math.min(
                      96,
                      Math.max(1, (totalDurationMs / scaleDurationMs) * 100 + 0.6)
                    )}%`,
                  }}
                >
                  <Add {...iconProps} />
                </button>
              ) : null}
            </div>
            <button
              type='button'
              className={styles.playhead}
              data-timeline-playhead
              role='slider'
              aria-label={t('creativeStudio.canvas.timeline.playhead', {
                defaultValue: '播放头',
              })}
              aria-valuemin={0}
              aria-valuemax={Math.round(scaleDurationMs / 1_000)}
              aria-valuenow={Math.round(currentTimeMs / 1_000)}
              aria-valuetext={formatTime(currentTimeMs)}
              style={{ left: `${(currentTimeMs / scaleDurationMs) * 100}%` }}
              onPointerDown={beginSeek}
              onPointerMove={updateSeek}
              onPointerUp={finishSeek}
              onPointerCancel={finishSeek}
              onKeyDown={(event) => {
                if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return;
                event.preventDefault();
                event.stopPropagation();
                setPlaying(false);
                const direction = event.key === 'ArrowLeft' ? -1 : 1;
                setTime(currentTimeRef.current + direction * (event.shiftKey ? 1_000 : 100));
              }}
            />
          </div>
        </div>

        {addMenuOpen ? (
          <div
            className={styles.addMenu}
            data-timeline-add-surface
            role='menu'
            style={{
              left: `${Math.min(
                78,
                Math.max(4, (totalDurationMs / scaleDurationMs) * 100 + 1)
              )}%`,
            }}
            aria-label={t('creativeStudio.canvas.timeline.addAssets', {
              defaultValue: '添加素材到时间线',
            })}
          >
            <button
              type='button'
              role='menuitem'
              disabled={!onUploadFiles}
              onClick={(event) => {
                event.stopPropagation();
                fileInputRef.current?.click();
              }}
            >
              <Upload {...iconProps} />
              {t('creativeStudio.canvas.timeline.uploadLocal', {
                defaultValue: '从本地添加',
              })}
            </button>
            <button
              type='button'
              role='menuitem'
              disabled={!onRequestAssets}
              onClick={(event) => {
                event.stopPropagation();
                requestAssets();
              }}
            >
              <FolderOpen {...iconProps} />
              {t('creativeStudio.canvas.timeline.addFromLibrary', {
                defaultValue: '从资产库添加',
              })}
            </button>
          </div>
        ) : null}
      </div>
    </CreativeNodeFrame>
  );
};

export default CreativeTimelineNode;
