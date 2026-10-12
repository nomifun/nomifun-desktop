/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { MessageId } from '../ids';

/** Display projection of canonical Turn facts; never a Runtime receipt. */
export interface TurnFileSummary {
  turn_id: MessageId;
  complete: boolean;
  files: Array<{
    identity: string;
    relative_path: string;
    file_name: string;
    path?: string;
    operation: 'create' | 'modify' | 'delete';
    size_bytes?: number;
    sha256?: string;
    published?: {
      /** Content-addressed workspace publication, distinct from image receipt UUIDs. */
      artifact_id: string;
      path: string;
      mime_type: string;
      size_bytes: number;
      sha256: string;
    };
  }>;
}
