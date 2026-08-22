/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  validateCreativeAssetManualUpload,
  type CreativeAssetUploadRejection,
} from '../../assets/page/model';
import { clientToCanvas, type CanvasPoint, type CanvasViewport } from '../core';
import type { CanvasIntegrationIntent } from './types';

export interface CanvasDropImportRejection {
  file: File;
  reason: CreativeAssetUploadRejection;
}

export interface CanvasDropImportValidation {
  intent: Extract<CanvasIntegrationIntent, { type: 'asset/import-file' }> | null;
  rejected: CanvasDropImportRejection[];
  /** Source behavior imports the first supported file from a single drop. */
  ignoredAcceptedFiles: File[];
}

const audioByName = (name: string): boolean => /\.(mp3|wav)$/i.test(name);

/**
 * Validate against the current real NomiFun manual-upload contract. Audio is
 * recognized like the source product but rejected honestly until that backend
 * capability exists.
 */
export function validateCanvasDropImport(
  files: readonly File[],
  localClientPosition: CanvasPoint,
  viewport: CanvasViewport
): CanvasDropImportValidation {
  const rejected: CanvasDropImportRejection[] = [];
  const accepted: Array<{ file: File; kind: 'image' | 'video' }> = [];

  for (const file of files) {
    const candidate = audioByName(file.name) && !file.type.startsWith('audio/')
      ? { name: file.name, type: 'audio/unknown', size: file.size }
      : file;
    const validation = validateCreativeAssetManualUpload(candidate);
    if (!validation.accepted && validation.rejection) {
      rejected.push({ file, reason: validation.rejection });
      continue;
    }
    accepted.push({ file, kind: file.type.toLocaleLowerCase().startsWith('video/') ? 'video' : 'image' });
  }

  const [first, ...rest] = accepted;
  return {
    intent: first
      ? {
          type: 'asset/import-file',
          file: first.file,
          kind: first.kind,
          worldPosition: clientToCanvas(localClientPosition, viewport),
          panoramaChoice: first.kind === 'image' ? 'after-upload-if-2-to-1' : 'not-applicable',
        }
      : null,
    rejected,
    ignoredAcceptedFiles: rest.map(({ file }) => file),
  };
}
