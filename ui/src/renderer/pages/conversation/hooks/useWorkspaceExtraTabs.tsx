/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { TChatConversation } from '@/common/config/storage';
import ConversationTerminalPanel from '@/renderer/pages/conversation/components/ConversationTerminalPanel';
import type { SessionKnowledgeSource } from '@/renderer/pages/conversation/Workspace/KnowledgePanel/knowledgeBindingTarget';
import { useSessionKnowledgeTab } from '@/renderer/pages/conversation/Workspace/KnowledgePanel/useSessionKnowledgeTab';
import MiniAppPanel from '@/renderer/pages/conversation/Workspace/MiniAppPanel';
import type { WorkspaceExtraTab } from '@/renderer/pages/conversation/Workspace/types';
import { ApplicationOne, Terminal } from '@icon-park/react';
import React, { useMemo } from 'react';
import { useTranslation } from 'react-i18next';

/**
 * The conversation-owned resources shown in the shared right-hand tool rail.
 *
 * This exists as a hook because `ChatConversation` builds the rail's extra tabs
 * in TWO independent places — the nomi layout and the generic layout that also
 * serves companion sessions and retained attempt transcripts. They never shared
 * a helper, so a tab added to one silently missed half the conversation kinds
 * and no test caught it. Both now call this.
 *
 * Gated on the session having a workspace, matching the rail's own visibility
 * (`workspaceEnabled` in ChatLayout, and ChatSlider's empty-div early return):
 * producing tabs a collapsed-away rail can never show is just dead state.
 */
export function useWorkspaceExtraTabs(conversation?: TChatConversation): WorkspaceExtraTab[] {
  const { t } = useTranslation();

  const conversationId = conversation?.id;
  const extra = conversation?.extra as Record<string, unknown> | undefined;
  const hasWorkspace = Boolean(extra?.workspace);

  const knowledgeSource = useMemo<SessionKnowledgeSource | undefined>(
    () => (conversationId && hasWorkspace ? { kind: 'conversation', conversationId, extra } : undefined),
    [conversationId, hasWorkspace, extra]
  );
  const knowledgeTabs = useSessionKnowledgeTab(knowledgeSource);

  return useMemo(() => {
    if (!conversationId || !hasWorkspace) return [];

    return [
      {
        key: 'conversation-terminals',
        title: t('terminal.conversationPanel.tab'),
        icon: <Terminal size={18} />,
        content: <ConversationTerminalPanel conversationId={conversationId} />,
      },
      // Directly below the terminal: a permanent, read-only way to USE a
      // solidified mini-app without leaving the conversation. Unconditional on
      // purpose — a tab that appears asynchronously would make WorkspaceRailBody
      // persist its `files` fallback over the user's stored selection. Loading,
      // empty and error states are handled inside the panel.
      {
        key: 'conversation-miniapps',
        title: t('miniApps.nav.entry'),
        icon: <ApplicationOne size={18} />,
        content: <MiniAppPanel />,
      },
      ...knowledgeTabs,
    ];
  }, [conversationId, hasWorkspace, knowledgeTabs, t]);
}
