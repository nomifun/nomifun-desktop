/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { saveUrlAs, type SaveAsResult } from '@/renderer/utils/file/saveAs';
import { creativeAssetDownloadName } from './page/model';
import { isCreativeAssetDeleted, type CreativeAsset } from './types';

/** Save an original creative asset through the platform's Save As workflow. */
export function saveCreativeAssetAs(
  asset: CreativeAsset,
  dialogTitle?: string
): Promise<SaveAsResult> {
  if (isCreativeAssetDeleted(asset) || !asset.originalUrl.trim()) {
    throw new Error('Asset file is unavailable');
  }
  return saveUrlAs(asset.originalUrl, {
    suggestedName: creativeAssetDownloadName(asset),
    mimeType: asset.mimeType,
    dialogTitle,
  });
}
