/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import SessionKnowledgePanel from '@/renderer/pages/conversation/Workspace/KnowledgePanel';
import type { SessionKnowledgeSource } from '@/renderer/pages/conversation/Workspace/KnowledgePanel/knowledgeBindingTarget';
import { useSessionKnowledgeMounts } from '@/renderer/pages/conversation/Workspace/KnowledgePanel/useSessionKnowledgeMounts';
import type { WorkspaceExtraTab } from '@/renderer/pages/conversation/Workspace/types';
import { BookOne } from '@icon-park/react';
import { useMemo } from 'react';
import { useTranslation } from 'react-i18next';

/** The rail tab key the knowledge panel is registered under. */
export const SESSION_KNOWLEDGE_TAB_KEY = 'session-knowledge';

/**
 * The knowledge rail entry for one session, or `[]` when nothing is mounted.
 *
 * Lives here rather than inline at each host so the conversation surface and the
 * terminal surface cannot drift: both call this with their own
 * {@link SessionKnowledgeSource} and splice the result into their extra tabs.
 */
export function useSessionKnowledgeTab(source: SessionKnowledgeSource | undefined): WorkspaceExtraTab[] {
  const { t } = useTranslation();
  const { mounted, bases } = useSessionKnowledgeMounts(source);

  return useMemo(() => {
    if (!mounted) return [];
    return [
      {
        key: SESSION_KNOWLEDGE_TAB_KEY,
        title: t('knowledge.control.label'),
        icon: <BookOne size={18} />,
        content: <SessionKnowledgePanel bases={bases} />,
      },
    ];
  }, [bases, mounted, t]);
}
