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
import CreativeVideoMedia from './CreativeVideoMedia';

import styles from './CreativeVideoPlayer.module.css';

export interface CreativeVideoPlayerProps {
  src: string;
  poster?: string;
  label: string;
  className?: string;
  variant?: 'canvas' | 'preview';
  autoPlay?: boolean;
  loop?: boolean;
  muted?: boolean;
  onPlayRequest?: () => void;
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

export const formatCreativeVideoTime = (seconds: number): string => {
  const total = Math.floor(safeMediaTime(seconds));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const remainder = total % 60;
  return hours > 0
    ? `${hours}:${minutes.toString().padStart(2, '0')}:${remainder.toString().padStart(2, '0')}`
    : `${minutes}:${remainder.toString().padStart(2, '0')}`;
};

/** The same controls serve canvas nodes, library previews and workbench results.
 * Playback state remains local to this player.
 */
const VideoPlayer: React.FC<CreativeVideoPlayerProps> = ({
  src,
  poster,
  label,
  className,
  variant = 'preview',
  autoPlay = false,
  loop = false,
  muted: initialMuted = false,
  onPlayRequest,
}) => {
  const { t } = useTranslation();
  const contentRef = useRef<HTMLDivElement>(null);
  const videoRef = useRef<HTMLVideoElement>(null);
  const lastAudibleVolumeRef = useRef(1);
  const [playing, setPlaying] = useState(false);
  const [failed, setFailed] = useState(false);
  const [currentTime, setCurrentTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const [volume, setVolume] = useState(1);
  const [muted, setMuted] = useState(initialMuted);
  const [fullscreen, setFullscreen] = useState(false);

  useEffect(() => {
    const video = videoRef.current;
    if (video) video.muted = initialMuted;
    setMuted(initialMuted);
  }, [initialMuted]);

  useEffect(() => {
    const video = videoRef.current;
    return () => {
      // Closing a preview or changing its source must stop its audio immediately.
      video?.pause();
    };
  }, []);

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
      onPlayRequest?.();
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
    if (variant === 'canvas' || fullscreen) event.stopPropagation();
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
      className={[styles.videoContent, className].filter(Boolean).join(' ')}
      data-creative-video-player
      data-video-player-variant={variant}
      data-video-node-player
      data-video-node-playing={playing}
      onPointerDown={isolateFullscreenCanvasEvent}
      onClick={isolateFullscreenCanvasEvent}
      onDoubleClick={isolateFullscreenCanvasEvent}
      onContextMenu={isolateFullscreenCanvasEvent}
    >
      <CreativeVideoMedia
        ref={videoRef}
        className={styles.videoMedia}
        src={src}
        poster={poster}
        controls={false}
        disablePictureInPicture
        muted={muted}
        loop={loop}
        autoPlay={autoPlay}
        playsInline
        preload='metadata'
        draggable={false}
        tabIndex={-1}
        aria-label={label}
        onLoadedMetadata={(event) => {
          setDuration(safeMediaTime(event.currentTarget.duration));
          setCurrentTime(safeMediaTime(event.currentTarget.currentTime));
        }}
        onLoadedData={() => setFailed(false)}
        onError={() => { setFailed(true); setPlaying(false); }}
        onDurationChange={(event) => setDuration(safeMediaTime(event.currentTarget.duration))}
        onTimeUpdate={(event) => setCurrentTime(safeMediaTime(event.currentTarget.currentTime))}
        onVolumeChange={(event) => syncVolume(event.currentTarget)}
        onPlay={() => setPlaying(true)}
        onPause={() => setPlaying(false)}
        onEnded={() => setPlaying(false)}
      />

      {failed ? (
        <span className={styles.videoError} role='status'>
          {t('creativeStudio.assets.picker.mediaUnavailable', { defaultValue: '素材文件不可用' })}
        </span>
      ) : null}

      {variant === 'canvas' ? (
        <div
          className={styles.videoDragSurface}
          data-video-node-drag-surface
          title={t('creativeStudio.canvas.nodes.video.drag')}
        />
      ) : null}

      {!playing && !failed ? (
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
        style={failed ? { display: 'none' } : undefined}
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
          aria-valuetext={`${formatCreativeVideoTime(currentTime)} / ${formatCreativeVideoTime(duration)}`}
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
            {formatCreativeVideoTime(currentTime)} / {formatCreativeVideoTime(duration)}
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

const CreativeVideoPlayer: React.FC<CreativeVideoPlayerProps> = (props) => (
  <VideoPlayer key={props.src} {...props} />
);

export default CreativeVideoPlayer;
