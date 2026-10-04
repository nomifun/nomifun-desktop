/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { PersistedArtifactId } from '@/common/types/ids';

export interface PersistedToolArtifact {
  id: PersistedArtifactId;
  kind: 'image' | 'audio' | 'video' | 'text' | 'file';
  mime_type: string;
  /** Canonical native path on the current host. */
  path: string;
  /** Portable path relative to the conversation workspace. */
  relative_path: string;
  size_bytes: number;
  sha256: string;
}
