/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { TChatConversation } from '@/common/config/storage';
import ConversationTerminalPanel from '@/renderer/pages/conversation/components/ConversationTerminalPanel';
import SessionKnowledgePanel, {
  SESSION_KNOWLEDGE_TAB_KEY,
} from '@/renderer/pages/conversation/Workspace/KnowledgePanel';
import type { SessionKnowledgeSource } from '@/renderer/pages/conversation/Workspace/KnowledgePanel/knowledgeBindingTarget';
import { useSessionKnowledgeMounts } from '@/renderer/pages/conversation/Workspace/KnowledgePanel/useSessionKnowledgeMounts';
import type { WorkspaceExtraTab } from '@/renderer/pages/conversation/Workspace/types';
import { BookOne, Terminal } from '@icon-park/react';
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
    () => (conversationId ? { kind: 'conversation', conversationId, extra } : undefined),
    [conversationId, extra]
  );
  const { mounted: knowledgeMounted, bases: knowledgeBases } = useSessionKnowledgeMounts(knowledgeSource);

  return useMemo(() => {
    if (!conversationId || !hasWorkspace) return [];

    const tabs: WorkspaceExtraTab[] = [
      {
        key: 'conversation-terminals',
        title: t('terminal.conversationPanel.tab'),
        icon: <Terminal size={18} />,
        content: <ConversationTerminalPanel conversationId={conversationId} />,
      },
    ];

    // Knowledge is a conditional entry: no mounted bases, no icon.
    if (knowledgeMounted) {
      tabs.push({
        key: SESSION_KNOWLEDGE_TAB_KEY,
        title: t('knowledge.control.label'),
        icon: <BookOne size={18} />,
        content: <SessionKnowledgePanel bases={knowledgeBases} />,
      });
    }

    return tabs;
  }, [conversationId, hasWorkspace, knowledgeBases, knowledgeMounted, t]);
}
