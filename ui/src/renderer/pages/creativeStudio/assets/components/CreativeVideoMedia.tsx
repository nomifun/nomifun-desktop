/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { forwardRef, useEffect, useImperativeHandle, useRef, useState } from 'react';

/** Decode an actual preview frame in WebKit even while the video is paused.
 * Metadata alone does not guarantee pixels. Never seek an active player or
 * overwrite a caller's starting position, and never play just to obtain a cover.
 */
function preparePreviewFrame(video: HTMLVideoElement): void {
  if (video.getAttribute('poster') || video.autoplay || !video.paused || video.currentTime !== 0) return;
  if (!Number.isFinite(video.duration) || video.duration <= 0) return;
  try {
    video.currentTime = Math.min(0.01, video.duration / 2);
  } catch {
    // Some streams are not seekable yet; preload still requests the first frame.
  }
}

const VideoMedia = forwardRef<HTMLVideoElement, React.VideoHTMLAttributes<HTMLVideoElement>>(
  ({ poster, preload, onLoadedMetadata, onLoadedData, ...props }, ref) => {
    const videoRef = useRef<HTMLVideoElement>(null);
    const [failedPoster, setFailedPoster] = useState<string>();
    const [frameReady, setFrameReady] = useState(false);
    const [visible, setVisible] = useState(() => typeof IntersectionObserver === 'undefined');
    const resolvedPoster = poster && poster !== failedPoster ? poster : undefined;
    useImperativeHandle(ref, () => videoRef.current!, []);

    useEffect(() => {
      const video = videoRef.current;
      if (!video || typeof IntersectionObserver === 'undefined') return;
      const observer = new IntersectionObserver((entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          setVisible(true);
          observer.disconnect();
        }
      }, { rootMargin: '200px' });
      observer.observe(video);
      return () => observer.disconnect();
    }, []);

    useEffect(() => {
      const video = videoRef.current;
      if (!resolvedPoster && video && video.readyState >= 1) preparePreviewFrame(video);
    }, [resolvedPoster]);

    useEffect(() => {
      if (!poster) return;
      // Video elements don't emit an error when only their poster fails.
      const image = new Image();
      image.onerror = () => setFailedPoster(poster);
      image.src = poster;
      return () => { image.onerror = null; };
    }, [poster]);

    return (
      <video
        {...props}
        ref={videoRef}
        poster={resolvedPoster}
        playsInline
        preload={!visible && !props.autoPlay ? 'none' : !resolvedPoster && !frameReady ? 'auto' : preload ?? 'metadata'}
        onLoadedMetadata={(event) => {
          preparePreviewFrame(event.currentTarget);
          onLoadedMetadata?.(event);
        }}
        onLoadedData={(event) => {
          preparePreviewFrame(event.currentTarget);
          setFrameReady(true);
          onLoadedData?.(event);
        }}
      />
    );
  }
);

/** Shared by library covers, native previews and the canvas custom player.
 * All state is presentation-only; URLs and playback never enter asset records.
 */
const CreativeVideoMedia = forwardRef<HTMLVideoElement, React.VideoHTMLAttributes<HTMLVideoElement>>(
  (props, ref) => <VideoMedia key={props.src} {...props} ref={ref} />
);

export default CreativeVideoMedia;
