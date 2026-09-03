/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { FileText, Pic, VideoTwo, Voice } from '@icon-park/react';
import React, { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { isCreativeAssetDeleted, type CreativeAsset, type CreativeAssetKind } from '../types';
import styles from './CreativeAssetLibrary.module.css';

export interface CreativeAssetMediaProps {
  asset: CreativeAsset;
  unavailableLabel: string;
  compact?: boolean;
}

export const creativeAssetKindIcon = (kind: CreativeAssetKind, size = 20): React.ReactNode => {
  const props = { theme: 'outline' as const, size, fill: 'currentColor', strokeWidth: 3 };
  switch (kind) {
    case 'image':
      return <Pic {...props} />;
    case 'video':
      return <VideoTwo {...props} />;
    case 'audio':
      return <Voice {...props} />;
    case 'text':
      return <FileText {...props} />;
  }
};

const CreativeAssetMedia: React.FC<CreativeAssetMediaProps> = ({ asset, unavailableLabel, compact = false }) => {
  const { t } = useTranslation();
  const [failed, setFailed] = useState(false);

  useEffect(() => setFailed(false), [asset.id, asset.originalUrl, asset.thumbnailUrl]);

  const deleted = isCreativeAssetDeleted(asset);
  if (deleted || failed) {
    return (
      <div className={styles.mediaFallback} data-asset-media-state={deleted ? 'deleted' : 'missing'} role='status'>
        <span aria-hidden='true'>{creativeAssetKindIcon(asset.kind, compact ? 18 : 26)}</span>
        <span>{deleted ? t('creativeStudio.assets.deleted', { defaultValue: '素材已删除' }) : unavailableLabel}</span>
      </div>
    );
  }

  if (asset.kind === 'image') {
    return (
      <img
        className={styles.mediaImage}
        src={asset.thumbnailUrl ?? asset.originalUrl}
        alt={asset.title}
        loading='lazy'
        draggable={false}
        onError={() => setFailed(true)}
      />
    );
  }

  if (asset.kind === 'video') {
    return (
      <video
        className={styles.mediaVideo}
        src={asset.originalUrl}
        muted
        playsInline
        preload='metadata'
        aria-label={asset.title}
        onError={() => setFailed(true)}
      />
    );
  }

  if (asset.kind === 'text') {
    return (
      <div className={styles.textPreview} data-asset-media-state='text'>
        <span aria-hidden='true'>{creativeAssetKindIcon('text', compact ? 16 : 22)}</span>
        <p>{asset.textContent?.trim() || asset.title}</p>
      </div>
    );
  }

  return (
    <div className={styles.audioPreview} data-asset-media-state='audio'>
      <span className={styles.audioIcon} aria-hidden='true'>
        {creativeAssetKindIcon('audio', compact ? 18 : 28)}
      </span>
      <span className={styles.audioMetadata}>{asset.mimeType ?? asset.title}</span>
    </div>
  );
};

export default CreativeAssetMedia;
