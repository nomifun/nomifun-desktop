/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CompanionId } from '@/common/types/ids';
import type { useCompanion } from '../useNomi';

/** The seven workspace tabs, in render order. */
export const WORKSPACE_TABS = [
  'overview',
  'memory',
  'remote',
  'evolution',
  'skills',
  'history',
  'other',
] as const;

export type WorkspaceTabKey = (typeof WORKSPACE_TABS)[number];

export const isWorkspaceTabKey = (value: string | null): value is WorkspaceTabKey =>
  value != null && (WORKSPACE_TABS as readonly string[]).includes(value);

/** Live view of one companion: profile, derived status and the optimistic patcher. */
export type CompanionHandle = ReturnType<typeof useCompanion>;

/**
 * Props every workspace tab receives. Tabs are pure per-companion surfaces: they
 * never read the roster and never navigate — the shell owns selection and URL
 * state, so a tab stays testable in isolation.
 */
export interface WorkspaceTabProps {
  companionId: CompanionId;
  companion: CompanionHandle;
  /**
   * Attention signal for this tab's segment in the strip. A tab reports whether
   * it has something awaiting the user; the shell renders the dot.
   */
  onAttentionChange?: (hasAttention: boolean) => void;
}
