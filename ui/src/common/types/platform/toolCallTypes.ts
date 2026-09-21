/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { PersistedArtifactId } from '@/common/types/ids';

/** Shared base — every session update notification carries a session id. */
interface BaseSessionUpdate {
  session_id: string;
}

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

/** Plan session update */
export interface PlanUpdate extends BaseSessionUpdate {
  update: {
    sessionUpdate: 'plan';
    entries: Array<{
      content: string;
      status: 'pending' | 'in_progress' | 'completed';
      priority?: 'low' | 'medium' | 'high';
    }>;
  };
}
