/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { FileText, Pic, VideoTwo, Voice } from '@icon-park/react';
import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { CreativeAssetKind } from '../types';
import CreativeVideoMedia from './CreativeVideoMedia';
import styles from './CreativeMediaPreview.module.css';

export const creativeAssetKindIcon = (kind: CreativeAssetKind, size = 20): React.ReactNode => {
  const props = { theme: 'outline' as const, size, fill: 'currentColor', strokeWidth: 3 };
  switch (kind) {
    case 'image': return <Pic {...props} />;
    case 'video': return <VideoTwo {...props} />;
    case 'audio': return <Voice {...props} />;
    case 'text': return <FileText {...props} />;
  }
};

export interface CreativeMediaPreviewProps {
  kind: CreativeAssetKind;
  /** Original media URL. A video/audio URL is never used as an image source. */
  src?: string | null;
  /** An actual image thumbnail, including a video's optional poster. */
  posterSrc?: string | null;
  alt?: string;
  className?: string;
  unavailableLabel?: string;
  /** Canvas nodes can explicitly opt into their persisted fit setting. */
  fit?: React.CSSProperties['objectFit'];
}

const MediaPreview: React.FC<CreativeMediaPreviewProps> = ({
  kind, src, posterSrc, alt = '', className, unavailableLabel, fit,
}) => {
  const { t } = useTranslation();
  const [failedSources, setFailedSources] = useState<string[]>([]);
  const imageSrc = [posterSrc, kind === 'image' ? src : null].find(
    (candidate): candidate is string => Boolean(candidate && !failedSources.includes(candidate))
  );
  const videoSrc = kind === 'video' && src && !failedSources.includes(src) ? src : undefined;
  const nonVisual = kind === 'audio' || kind === 'text';
  const failed = !nonVisual && !imageSrc && !videoSrc;
  const markFailed = (url: string) => setFailedSources((current) => [...current, url]);
  const label = unavailableLabel ?? t('creativeStudio.assets.picker.mediaUnavailable', { defaultValue: '素材文件不可用' });

  return (
    <span className={[styles.preview, className].filter(Boolean).join(' ')} data-creative-media-preview={kind} data-asset-media-state={failed ? 'missing' : kind}>
      {imageSrc ? (
        <img className={styles.media} src={imageSrc} alt={alt} loading='lazy' draggable={false} style={fit ? { objectFit: fit } : undefined} onError={() => markFailed(imageSrc)} />
      ) : videoSrc ? (
        <CreativeVideoMedia className={styles.media} src={videoSrc} muted controls={false} tabIndex={-1} draggable={false} aria-label={alt} onError={() => markFailed(videoSrc)} />
      ) : (
        <span className={styles.fallback} role={failed ? 'status' : undefined} title={failed ? label : alt}>
          <span aria-hidden='true'>{creativeAssetKindIcon(kind)}</span>
          {failed ? <span>{label}</span> : null}
        </span>
      )}
    </span>
  );
};

const CreativeMediaPreview: React.FC<CreativeMediaPreviewProps> = (props) => (
  <MediaPreview key={JSON.stringify([props.kind, props.src, props.posterSrc])} {...props} />
);

export default CreativeMediaPreview;
