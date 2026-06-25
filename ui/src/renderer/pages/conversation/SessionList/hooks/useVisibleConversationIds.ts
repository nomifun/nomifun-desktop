/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useMemo } from 'react';
import { useConversationHistoryContext } from '@/renderer/hooks/context/ConversationHistoryContext';
import { useTerminalSessions } from '@/renderer/pages/terminal/useTerminalSessions';
import { useWorkpathUiState } from './useWorkpathUiState';
import { buildWorkpathTree } from '../utils/workpathTree';

/**
 * Interactive conversation ids in the unified workpath session list's visual
 * order (workpath node order → in-group pinned/activity order). Used by the
 * Ctrl+Tab conversation-cycling shortcut.
 *
 * Unlike the old GroupedHistory implementation this does not skip
 * conversations inside collapsed drawers — cycling reaches every conversation,
 * matching the flat-data simplification of the session-list unification.
 * Terminals are fed into the tree only so node (drawer) ordering matches the
 * sidebar exactly; terminal sessions themselves are never cycled.
 */
export const useVisibleConversationIds = (): number[] => {
  const { conversations } = useConversationHistoryContext();
  const { sessions: terminals } = useTerminalSessions();
  const { pinnedKeys } = useWorkpathUiState();

  return useMemo(() => {
    return buildWorkpathTree(conversations, terminals, pinnedKeys).flatMap((node) =>
      node.interactive.map((entry) => entry.id)
    );
  }, [conversations, terminals, pinnedKeys]);
};
