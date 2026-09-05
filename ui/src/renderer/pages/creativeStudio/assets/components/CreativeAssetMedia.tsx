/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';

import { isCreativeAssetDeleted, type CreativeAsset } from '../types';
import CreativeMediaPreview, { creativeAssetKindIcon } from './CreativeMediaPreview';
import styles from './CreativeAssetLibrary.module.css';

export { creativeAssetKindIcon } from './CreativeMediaPreview';

export interface CreativeAssetMediaProps {
  asset: CreativeAsset;
  unavailableLabel: string;
  compact?: boolean;
}

const CreativeAssetMedia: React.FC<CreativeAssetMediaProps> = ({ asset, unavailableLabel, compact = false }) => {
  const { t } = useTranslation();
  const deleted = isCreativeAssetDeleted(asset);
  if (deleted) {
    return (
      <div className={styles.mediaFallback} data-asset-media-state='deleted' role='status'>
        <span aria-hidden='true'>{creativeAssetKindIcon(asset.kind, compact ? 18 : 26)}</span>
        <span>{t('creativeStudio.assets.deleted', { defaultValue: '素材已删除' })}</span>
      </div>
    );
  }

  if (asset.kind === 'image' || asset.kind === 'video') {
    return (
      <CreativeMediaPreview
        kind={asset.kind}
        src={asset.originalUrl}
        posterSrc={asset.thumbnailUrl}
        alt={asset.title}
        unavailableLabel={unavailableLabel}
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
