/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { PersistedArtifactId } from '@/common/types/ids';

/** Shared base — every session update notification carries a session id. */
export interface BaseSessionUpdate {
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

/** Tool call 内容项类型 / Tool call content item type */
export type ToolCallContentItem =
  | {
      type: 'content';
      content: {
        type: 'text';
        text: string;
      };
    }
  | {
      type: 'diff';
      path: string;
      old_text?: string | null;
      new_text: string;
    }
  | {
      type: 'artifact';
      artifact: PersistedToolArtifact;
      source_uri?: string;
    }
  | {
      type: 'resource_link';
      name: string;
      uri: string;
      title?: string;
      description?: string;
      mime_type?: string;
      size_bytes?: number;
    }
  | {
      type: 'terminal';
      terminal_id: string;
    }
  | {
      type: 'artifact_error';
      message: string;
    };

/** Tool call 位置项类型 / Tool call location item type */
export interface ToolCallLocationItem {
  path: string;
}

/** Tool call session update */
export interface ToolCallUpdate extends BaseSessionUpdate {
  /** Persistence-only two-phase delivery marker; absent on live frames. */
  artifact_delivery_committed?: boolean;
  update: {
    sessionUpdate: 'tool_call' | 'tool_call_update';
    tool_call_id: string;
    /** `tool_call_update` fields are partial and must not synthesize defaults. */
    status?: 'pending' | 'in_progress' | 'completed' | 'failed';
    title?: string;
    kind?: 'read' | 'edit' | 'execute';
    rawInput?: Record<string, unknown>;
    rawOutput?: unknown;
    content?: ToolCallContentItem[];
    locations?: ToolCallLocationItem[];
  };
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
