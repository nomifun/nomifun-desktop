/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { IKnowledgeBinding } from '@/common/adapter/ipcBridge';
import type { IIdmmConfig } from '@/common/types/idmm';
import type { ConversationId } from '@/common/types/ids';
import { Message } from '@arco-design/web-react';
import { useCallback, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { AutoWorkDraftValue } from '@/renderer/pages/conversation/components/AutoWorkControl';
import { defaultKnowledgeBinding } from '@/renderer/pages/conversation/components/KnowledgeControl';
import { defaultIdmmConfig } from '@/renderer/pages/conversation/components/IdmmControl';

export type GuidSessionOptions = {
  knowledge: IKnowledgeBinding;
  setKnowledge: (next: IKnowledgeBinding) => void;
  autoWork: AutoWorkDraftValue;
  setAutoWork: (next: AutoWorkDraftValue) => void;
  idmm: IIdmmConfig;
  setIdmm: (next: IIdmmConfig) => void;
  setIdmmDefault: (next: IIdmmConfig) => void;
  applyToConversation: (
    conversationId: ConversationId,
    options?: { allowKnowledgeBinding?: boolean; allowAutomation?: boolean }
  ) => Promise<void>;
  reset: () => void;
};

/**
 * Session-specific drafts for the Guid page. A selected AgentPreset freezes
 * reusable capabilities; plain Nomi keeps its own explicit model selection.
 */
export const useGuidSessionOptions = (): GuidSessionOptions => {
  const { t } = useTranslation();
  const [knowledge, setKnowledge] = useState<IKnowledgeBinding>(
    defaultKnowledgeBinding
  );
  const [autoWork, setAutoWork] = useState<AutoWorkDraftValue>({
    enabled: false,
  });
  const [idmm, setIdmmState] = useState<IIdmmConfig>(defaultIdmmConfig);
  const idmmOverriddenRef = useRef(false);
  const setIdmm = useCallback((next: IIdmmConfig) => {
    idmmOverriddenRef.current = true;
    setIdmmState(next);
  }, []);
  const setIdmmDefault = useCallback((next: IIdmmConfig) => {
    idmmOverriddenRef.current = false;
    setIdmmState(structuredClone(next));
  }, []);

  const draftsRef = useRef({ knowledge, autoWork, idmm });
  draftsRef.current = { knowledge, autoWork, idmm };

  const applyToConversation = useCallback(
    async (
      conversationId: ConversationId,
      options?: { allowKnowledgeBinding?: boolean; allowAutomation?: boolean }
    ) => {
      const { knowledge: kb, autoWork: aw, idmm: idmmConfig } = draftsRef.current;
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
      const report = (
        pendingTasks: Array<{ label: string }>,
        results: PromiseSettledResult<unknown>[]
      ) => {
        results.forEach((result, index) => {
          if (result.status !== 'rejected') return;
          console.error(
            `[GuidSessionOptions] Failed to apply ${pendingTasks[index].label}:`,
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

      if (options?.allowAutomation !== false && idmmOverriddenRef.current) {
        const idmmTask = {
          label: t('idmm.title'),
          run: () =>
            ipcBridge.idmm.setConfig.invoke({
              agent_session_id: conversationId,
              config: idmmConfig,
            }),
        };
        const [result] = await Promise.allSettled([idmmTask.run()]);
        report([idmmTask], [result]);
      }

      if (options?.allowAutomation !== false && aw.enabled && aw.tag) {
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
        const [result] = await Promise.allSettled([autoWorkTask.run()]);
        report([autoWorkTask], [result]);
        // AutoWork is the launch mode itself, not an optional decoration. A
        // failed binding must abort the Guid handoff so the caller can delete
        // the just-created, still-empty Session instead of navigating to a
        // conversation that will never consume its queue.
        if (result.status === 'rejected') throw result.reason;
      }
    },
    [t]
  );

  const reset = useCallback(() => {
    setKnowledge(defaultKnowledgeBinding());
    setAutoWork({ enabled: false });
    idmmOverriddenRef.current = false;
    setIdmmState(defaultIdmmConfig());
  }, []);

  return {
    knowledge,
    setKnowledge,
    autoWork,
    setAutoWork,
    idmm,
    setIdmm,
    setIdmmDefault,
    applyToConversation,
    reset,
  };
};
