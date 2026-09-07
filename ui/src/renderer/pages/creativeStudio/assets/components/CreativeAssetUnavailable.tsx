/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useTranslation } from 'react-i18next';
import type { CreativeAssetAvailability } from '../useCreativeAssetAvailability';
import styles from './CreativeAssetLibrary.module.css';

export function CreativeAssetUnavailable({ status }: { status: CreativeAssetAvailability }) {
  const { t } = useTranslation();
  return (
    <div className={styles.mediaFallback} data-asset-media-state={status} role='status'>
      {status === 'deleted'
        ? t('creativeStudio.assets.deleted', { defaultValue: '素材已删除' })
        : status === 'loading'
          ? t('creativeStudio.assets.library.loading', { defaultValue: '正在加载素材…' })
          : t('creativeStudio.assets.library.mediaUnavailable', { defaultValue: '素材暂时无法预览' })}
    </div>
  );
}
