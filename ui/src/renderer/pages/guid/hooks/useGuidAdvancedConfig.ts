/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { IIdmmConfig, IKnowledgeBinding } from '@/common/adapter/ipcBridge';
import type { ConversationId } from '@/common/types/ids';
import { Message } from '@arco-design/web-react';
import { useCallback, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { AutoWorkDraftValue } from '@/renderer/pages/conversation/components/AutoWorkControl';
import { defaultIdmmConfig } from '@/renderer/pages/conversation/components/IdmmControl';
import { defaultKnowledgeBinding } from '@/renderer/pages/conversation/components/KnowledgeControl';

export type GuidAdvancedConfig = {
  knowledge: IKnowledgeBinding;
  setKnowledge: (next: IKnowledgeBinding) => void;
  autoWork: AutoWorkDraftValue;
  setAutoWork: (next: AutoWorkDraftValue) => void;
  idmm: IIdmmConfig;
  setIdmm: (next: IIdmmConfig) => void;
  applyToConversation: (
    conversationId: ConversationId,
    options?: { allowKnowledgeBinding?: boolean }
  ) => Promise<void>;
  reset: () => void;
};

/**
 * Session-specific drafts for the Guid page. A selected AgentPreset freezes
 * reusable capabilities; plain Nomi keeps its own explicit model selection.
 */
export const useGuidAdvancedConfig = (): GuidAdvancedConfig => {
  const { t } = useTranslation();
  const [knowledge, setKnowledge] = useState<IKnowledgeBinding>(
    defaultKnowledgeBinding
  );
  const [autoWork, setAutoWork] = useState<AutoWorkDraftValue>({
    enabled: false,
  });
  const [idmm, setIdmm] = useState<IIdmmConfig>(defaultIdmmConfig);

  const draftsRef = useRef({ knowledge, autoWork, idmm });
  draftsRef.current = { knowledge, autoWork, idmm };

  const applyToConversation = useCallback(
    async (
      conversationId: ConversationId,
      options?: { allowKnowledgeBinding?: boolean }
    ) => {
      const { knowledge: kb, autoWork: aw, idmm: idm } = draftsRef.current;
      const tasks: Array<{ label: string; run: () => Promise<unknown> }> = [];

      if (
        options?.allowKnowledgeBinding !== false &&
        (kb.enabled ||
          kb.writeback ||
          kb.kb_ids.length > 0 ||
          kb.writeback_eagerness !== 'manual')
      ) {
        tasks.push({
          label: t('knowledge.control.label'),
          run: () =>
            ipcBridge.knowledge.setBinding.invoke({
              kind: 'conversation',
              target_id: conversationId,
              ...kb,
            }),
        });
      }
      if (idm.fault_watch.enabled || idm.decision_watch.enabled) {
        tasks.push({
          label: t('idmm.label'),
          run: () =>
            ipcBridge.idmm.set.invoke({
              kind: 'conversation',
              target_id: conversationId,
              ...idm,
            }),
        });
      }

      const report = (
        pendingTasks: Array<{ label: string }>,
        results: PromiseSettledResult<unknown>[]
      ) => {
        results.forEach((result, index) => {
          if (result.status !== 'rejected') return;
          console.error(
            `[GuidAdvancedConfig] Failed to apply ${pendingTasks[index].label}:`,
            result.reason
          );
          Message.warning(
            t('guid.advanced.applyFailed', {
              feature: pendingTasks[index].label,
            })
          );
        });
      };

      if (tasks.length > 0) {
        report(tasks, await Promise.allSettled(tasks.map((task) => task.run())));
      }

      if (aw.enabled && aw.tag) {
        const autoWorkTask = {
          label: t('requirements.autowork.label'),
          run: () =>
            ipcBridge.requirements.setAutoWork.invoke({
              kind: 'conversation',
              target_id: conversationId,
              enabled: true,
              tag: aw.tag,
            }),
        };
        report(
          [autoWorkTask],
          await Promise.allSettled([autoWorkTask.run()])
        );
      }
    },
    [t]
  );

  const reset = useCallback(() => {
    setKnowledge(defaultKnowledgeBinding());
    setAutoWork({ enabled: false });
    setIdmm(defaultIdmmConfig());
  }, []);

  return {
    knowledge,
    setKnowledge,
    autoWork,
    setAutoWork,
    idmm,
    setIdmm,
    applyToConversation,
    reset,
  };
};
