/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { NOMIFUN_FILES_MARKER } from '@/common/config/constants';

export type ParsedMessageFileMarker = {
  text: string;
  files: string[];
};

/**
 * Decode the private attachment marker only on the trusted user-message side.
 * Assistant/model text is untrusted and must remain ordinary visible text;
 * otherwise a model can forge a successful-looking local file preview without
 * a committed artifact receipt.
 */
export const parseMessageFileMarker = (content: string, position?: string): ParsedMessageFileMarker => {
  if (position !== 'right') {
    return { text: content, files: [] };
  }

  const markerIndex = content.indexOf(NOMIFUN_FILES_MARKER);
  if (markerIndex === -1) {
    return { text: content, files: [] };
  }
  const text = content.slice(0, markerIndex).trimEnd();
  const afterMarker = content.slice(markerIndex + NOMIFUN_FILES_MARKER.length).trim();
  const files = afterMarker
    ? afterMarker
        .split('\n')
        .map((line) => line.trim())
        .filter(Boolean)
    : [];
  return { text, files };
};
