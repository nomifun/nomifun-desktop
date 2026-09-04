/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  FullScreen,
  OffScreen,
  Pause,
  PlayOne,
  VolumeMute,
  VolumeSmall,
} from '@icon-park/react';
import React, { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { CreativeNodeAssetPresentation, CreativeNodeOfKind } from './types';
import styles from './CreativeNodeViews.module.css';

interface CreativeVideoNodeMediaProps {
  node: CreativeNodeOfKind<'video'>;
  asset: CreativeNodeAssetPresentation;
  title: string;
  selected?: boolean;
  onActivate?: () => void;
}

type FullscreenDocument = Document & {
  webkitExitFullscreen?: () => Promise<void> | void;
  webkitFullscreenElement?: Element | null;
};

type FullscreenElement = HTMLDivElement & {
  webkitRequestFullscreen?: () => Promise<void> | void;
};

const safeMediaTime = (value: number) =>
  Number.isFinite(value) && value >= 0 ? value : 0;

const safeMediaVolume = (value: number) =>
  Math.min(1, Math.max(0, Number.isFinite(value) ? value : 1));

export const formatVideoNodeTime = (seconds: number): string => {
  const total = Math.floor(safeMediaTime(seconds));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const remainder = total % 60;
  return hours > 0
    ? `${hours}:${minutes.toString().padStart(2, '0')}:${remainder.toString().padStart(2, '0')}`
    : `${minutes}:${remainder.toString().padStart(2, '0')}`;
};

/** Canvas-specific controls leave the media surface available for node dragging.
 * Playback state stays transient and never enters the persisted Canvas document.
 */
const CreativeVideoNodeMedia: React.FC<CreativeVideoNodeMediaProps> = ({
  node,
  asset,
  title,
  selected,
  onActivate,
}) => {
  const { t } = useTranslation();
  const contentRef = useRef<HTMLDivElement>(null);
  const videoRef = useRef<HTMLVideoElement>(null);
  const lastAudibleVolumeRef = useRef(1);
  const [playing, setPlaying] = useState(false);
  const [currentTime, setCurrentTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const [volume, setVolume] = useState(1);
  const [muted, setMuted] = useState(node.data.muted);
  const [fullscreen, setFullscreen] = useState(false);

  useEffect(() => {
    const video = videoRef.current;
    if (video) video.muted = node.data.muted;
    setMuted(node.data.muted);
  }, [node.data.muted]);

  useEffect(() => {
    const ownerDocument = contentRef.current?.ownerDocument;
    if (!ownerDocument) return;
    const handleFullscreenChange = () => {
      const fullscreenDocument = ownerDocument as FullscreenDocument;
      const fullscreenElement = fullscreenDocument.fullscreenElement ?? fullscreenDocument.webkitFullscreenElement;
      setFullscreen(fullscreenElement === contentRef.current);
    };
    ownerDocument.addEventListener('fullscreenchange', handleFullscreenChange);
    ownerDocument.addEventListener('webkitfullscreenchange', handleFullscreenChange);
    return () => {
      ownerDocument.removeEventListener('fullscreenchange', handleFullscreenChange);
      ownerDocument.removeEventListener('webkitfullscreenchange', handleFullscreenChange);
    };
  }, []);

  const togglePlayback = () => {
    const video = videoRef.current;
    if (!video) return;
    if (video.paused) {
      if (!selected) onActivate?.();
      void video.play().catch(() => setPlaying(false));
    } else {
      video.pause();
    }
  };

  const toggleMuted = () => {
    const video = videoRef.current;
    if (!video) return;
    const silent = video.muted || video.volume <= 0;
    if (silent) {
      if (video.volume <= 0) video.volume = lastAudibleVolumeRef.current;
      video.muted = false;
    } else {
      lastAudibleVolumeRef.current = safeMediaVolume(video.volume) || 1;
      video.muted = true;
    }
    setVolume(safeMediaVolume(video.volume));
    setMuted(video.muted);
  };

  const changeVolume = (value: number) => {
    const video = videoRef.current;
    if (!video) return;
    const nextVolume = safeMediaVolume(value);
    if (nextVolume > 0) lastAudibleVolumeRef.current = nextVolume;
    video.volume = nextVolume;
    video.muted = nextVolume <= 0;
    setVolume(nextVolume);
    setMuted(video.muted);
  };

  const syncVolume = (video: HTMLVideoElement) => {
    const nextVolume = safeMediaVolume(video.volume);
    if (nextVolume > 0) lastAudibleVolumeRef.current = nextVolume;
    setVolume(nextVolume);
    setMuted(video.muted);
  };

  const toggleFullscreen = async () => {
    const content = contentRef.current as FullscreenElement | null;
    if (!content) return;
    const ownerDocument = content.ownerDocument as FullscreenDocument;
    try {
      if (ownerDocument.fullscreenElement || ownerDocument.webkitFullscreenElement) {
        if (ownerDocument.exitFullscreen) await ownerDocument.exitFullscreen();
        else await ownerDocument.webkitExitFullscreen?.();
      } else if (content.requestFullscreen) {
        await content.requestFullscreen();
      } else {
        await content.webkitRequestFullscreen?.();
      }
    } catch {
      // Embedded webviews may deny fullscreen; playback remains usable.
    }
  };

  const isolatePointer: React.PointerEventHandler<HTMLElement> = (event) => {
    event.stopPropagation();
  };
  const isolateClick: React.MouseEventHandler<HTMLElement> = (event) => {
    event.stopPropagation();
  };
  const isolateKey: React.KeyboardEventHandler<HTMLElement> = (event) => {
    event.stopPropagation();
  };
  const isolateFullscreenCanvasEvent: React.EventHandler<
    React.SyntheticEvent<HTMLDivElement>
  > = (event) => {
    const ownerDocument = event.currentTarget.ownerDocument as FullscreenDocument;
    const fullscreenElement =
      ownerDocument.fullscreenElement ?? ownerDocument.webkitFullscreenElement;
    if (fullscreenElement === event.currentTarget) event.stopPropagation();
  };

  const playLabel = t(
    playing
      ? 'creativeStudio.canvas.nodes.video.pause'
      : 'creativeStudio.canvas.nodes.video.play'
  );
  const silent = muted || volume <= 0;
  const effectiveVolume = silent ? 0 : volume;
  const volumePercentage = Math.round(effectiveVolume * 100);
  const muteLabel = t(
    silent
      ? 'creativeStudio.canvas.nodes.video.unmute'
      : 'creativeStudio.canvas.nodes.video.mute'
  );
  const fullscreenLabel = t(
    fullscreen
      ? 'creativeStudio.canvas.nodes.video.exitFullscreen'
      : 'creativeStudio.canvas.nodes.video.fullscreen'
  );

  return (
    <div
      ref={contentRef}
      className={styles.videoContent}
      data-video-node-player
      data-video-node-playing={playing}
      onPointerDown={isolateFullscreenCanvasEvent}
      onClick={isolateFullscreenCanvasEvent}
      onDoubleClick={isolateFullscreenCanvasEvent}
      onContextMenu={isolateFullscreenCanvasEvent}
    >
      <video
        ref={videoRef}
        className={styles.videoMedia}
        src={asset.src}
        poster={asset.posterSrc}
        controls={false}
        disablePictureInPicture
        muted={muted}
        loop={node.data.loop}
        autoPlay={node.data.autoplay}
        playsInline
        preload='metadata'
        draggable={false}
        tabIndex={-1}
        aria-label={asset.alt ?? asset.label ?? title}
        onLoadedMetadata={(event) => {
          setDuration(safeMediaTime(event.currentTarget.duration));
          setCurrentTime(safeMediaTime(event.currentTarget.currentTime));
        }}
        onDurationChange={(event) => setDuration(safeMediaTime(event.currentTarget.duration))}
        onTimeUpdate={(event) => setCurrentTime(safeMediaTime(event.currentTarget.currentTime))}
        onVolumeChange={(event) => syncVolume(event.currentTarget)}
        onPlay={() => setPlaying(true)}
        onPause={() => setPlaying(false)}
        onEnded={() => setPlaying(false)}
      />

      <div
        className={styles.videoDragSurface}
        data-video-node-drag-surface
        title={t('creativeStudio.canvas.nodes.video.drag')}
      />

      {!playing ? (
        <button
          type='button'
          className={styles.videoCenterPlay}
          data-video-node-center-play
          aria-label={t('creativeStudio.canvas.nodes.video.play')}
          onPointerDown={isolatePointer}
          onClick={(event) => {
            isolateClick(event);
            togglePlayback();
          }}
          onDoubleClick={isolateClick}
          onKeyDown={isolateKey}
        >
          <PlayOne theme='filled' size={18} fill='currentColor' strokeWidth={3} />
        </button>
      ) : null}

      <div
        className={styles.videoControls}
        data-video-node-controls
        onPointerDown={isolatePointer}
        onClick={isolateClick}
        onDoubleClick={isolateClick}
        onKeyDown={isolateKey}
      >
        <input
          className={styles.videoSeek}
          data-video-node-seek
          type='range'
          min={0}
          max={duration || 0}
          step='0.01'
          value={Math.min(currentTime, duration || 0)}
          disabled={duration <= 0}
          aria-label={t('creativeStudio.canvas.nodes.video.seek')}
          aria-valuetext={`${formatVideoNodeTime(currentTime)} / ${formatVideoNodeTime(duration)}`}
          style={{ '--video-progress': `${duration > 0 ? (currentTime / duration) * 100 : 0}%` } as React.CSSProperties}
          onInput={(event) => {
            const video = videoRef.current;
            const nextTime = safeMediaTime(Number(event.currentTarget.value));
            if (video) video.currentTime = nextTime;
            setCurrentTime(nextTime);
          }}
        />
        <div className={styles.videoControlRow}>
          <button
            type='button'
            data-video-node-playback-toggle
            aria-label={playLabel}
            aria-pressed={playing}
            title={playLabel}
            onClick={togglePlayback}
          >
            {playing ? <Pause theme='filled' size={13} fill='currentColor' /> : <PlayOne theme='filled' size={13} fill='currentColor' />}
          </button>
          <span className={styles.videoTime} data-video-node-time>
            {formatVideoNodeTime(currentTime)} / {formatVideoNodeTime(duration)}
          </span>
          <span className={styles.videoControlSpacer} />
          <button
            type='button'
            data-video-node-mute
            aria-label={muteLabel}
            aria-pressed={silent}
            title={muteLabel}
            onClick={toggleMuted}
          >
            {silent ? <VolumeMute theme='outline' size={13} fill='currentColor' /> : <VolumeSmall theme='outline' size={13} fill='currentColor' />}
          </button>
          <input
            className={styles.videoVolume}
            data-video-node-volume
            type='range'
            min={0}
            max={1}
            step='0.01'
            value={effectiveVolume}
            aria-label={t('creativeStudio.canvas.properties.volume')}
            aria-valuetext={t('creativeStudio.canvas.editor.volumePercent', {
              percent: volumePercentage,
            })}
            style={{ '--video-volume': `${volumePercentage}%` } as React.CSSProperties}
            onInput={(event) => changeVolume(Number(event.currentTarget.value))}
          />
          <button
            type='button'
            data-video-node-fullscreen
            aria-label={fullscreenLabel}
            aria-pressed={fullscreen}
            title={fullscreenLabel}
            onClick={() => void toggleFullscreen()}
          >
            {fullscreen ? (
              <OffScreen theme='outline' size={13} fill='currentColor' />
            ) : (
              <FullScreen theme='outline' size={13} fill='currentColor' />
            )}
          </button>
        </div>
      </div>
    </div>
  );
};

export default CreativeVideoNodeMedia;
