/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import { parseError, uuidv7 } from '@/common/utils';
import type { ConversationId } from '@/common/types/ids';
import type {
  AgentSwitchSelection,
  ApplyAgentSessionSwitchRequest,
  ApplyAgentSessionSwitchResponse,
  PreviewAgentSessionSwitchResponse,
} from '@/common/types/agentPlatform';
import { refreshConversationCache } from '@/renderer/pages/conversation/utils/conversationCache';
import type { GuidAgentSelection } from '@/renderer/pages/guid/types';
import { TEMPLATE_I18N_PATH } from '@/renderer/pages/agentSettings/model';
import { Message } from '@arco-design/web-react';
import { useCallback, useEffect, useRef, useState, type MutableRefObject } from 'react';
import { useTranslation } from 'react-i18next';

interface AgentSwitchServices {
  preview: (sessionId: ConversationId, selection: AgentSwitchSelection) => Promise<PreviewAgentSessionSwitchResponse>;
  apply: (sessionId: ConversationId, request: ApplyAgentSessionSwitchRequest, idempotencyKey: string) => Promise<ApplyAgentSessionSwitchResponse>;
  refresh: (sessionId: ConversationId) => Promise<void>;
}

const services: AgentSwitchServices = {
  preview: (agent_session_id, selection) => ipcBridge.agentPlatform.sessions.previewAgentSwitch.invoke({
    agent_session_id, request: { selection },
  }),
  apply: (agent_session_id, request, idempotency_key) => ipcBridge.agentPlatform.sessions.applyAgentSwitch.invoke({
    agent_session_id, request, idempotency_key,
  }),
  refresh: refreshConversationCache,
};

export const useConversationAgentSwitch = ({
  sessionId,
  currentSelection,
  processing,
  switchingRef,
  configurationChangingRef,
  reasoningUpdatingRef,
}: {
  sessionId: ConversationId;
  currentSelection: GuidAgentSelection;
  processing: boolean;
  switchingRef: MutableRefObject<boolean>;
  configurationChangingRef: MutableRefObject<boolean>;
  reasoningUpdatingRef: MutableRefObject<boolean>;
}, operations: AgentSwitchServices = services) => {
  const { t } = useTranslation();
  const [pendingSessionId, setPendingSessionId] = useState<ConversationId | null>(null);
  const requestRef = useRef<{ sessionId: ConversationId; idempotencyKey: string; generation: number } | null>(null);
  const sessionIdRef = useRef(sessionId);
  const generationRef = useRef(0);
  const mountedRef = useRef(true);
  if (sessionIdRef.current !== sessionId) {
    sessionIdRef.current = sessionId;
    generationRef.current += 1;
  }

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      switchingRef.current = false;
    };
  }, [switchingRef]);
  useEffect(() => {
    if (requestRef.current?.generation !== generationRef.current) {
      requestRef.current = null;
      switchingRef.current = false;
      setPendingSessionId(null);
    }
  }, [sessionId, switchingRef]);

  const switchAgent = useCallback(async (selection: GuidAgentSelection) => {
    if (processing) {
      Message.warning(t('conversation.chat.agentSwitch.waitForTurn'));
      return;
    }
    if (configurationChangingRef.current || reasoningUpdatingRef.current
      || (switchingRef.current && requestRef.current?.sessionId === sessionId)) return;
    if ((selection.kind === 'preset' && currentSelection.kind === 'preset'
      && selection.presetId === currentSelection.presetId)
      || (selection.kind === 'template' && currentSelection.kind === 'template'
      && selection.templateKey === currentSelection.templateKey)) return;

    const request = { sessionId, idempotencyKey: uuidv7(), generation: generationRef.current };
    requestRef.current = request;
    switchingRef.current = true;
    setPendingSessionId(sessionId);
    const current = () => mountedRef.current && sessionIdRef.current === sessionId
      && generationRef.current === request.generation
      && requestRef.current === request;
    const wireSelection: AgentSwitchSelection = selection.kind === 'preset'
      ? { kind: 'preset', preset_id: selection.presetId }
      : { kind: 'template', template_key: selection.templateKey };

    try {
      const preview = await operations.preview(sessionId, wireSelection);
      if (!current()) return;
      if (!preview.can_apply) {
        const blocker = preview.blockers[0];
        const reason = blocker
          ? t(`conversation.chat.agentSwitch.blockers.${blocker.code}`, { defaultValue: blocker.message })
          : t('conversation.chat.agentSwitch.unavailable');
        Message.warning(t('conversation.chat.agentSwitch.blocked', { reason }));
        return;
      }
      const result = await operations.apply(sessionId, {
        selection: wireSelection,
        handoff_mode: preview.handoff.available ? 'continue_task' : 'context_only',
        expected_binding_version: preview.expected_binding_version,
      }, request.idempotencyKey);
      // A completed switch remains successful even if its projection refresh fails.
      await operations.refresh(sessionId).catch((error) => {
        console.error('[ChatConversation] Failed to refresh switched Agent:', error);
      });
      if (!current()) return;
      Message.success(t('conversation.chat.agentSwitch.success', {
        agent: selection.kind === 'template'
          ? t(`agentSettings.template.${TEMPLATE_I18N_PATH[selection.templateKey]}.name`)
          : result.current_agent_label,
      }));
    } catch (error) {
      if (!current()) return;
      const message = isBackendHttpError(error)
        ? t(`conversation.chat.agentSwitch.blockers.${error.code}`, { defaultValue: parseError(error) })
        : parseError(error);
      Message.error(t('conversation.chat.agentSwitch.failed', { error: message }));
    } finally {
      if (current()) {
        requestRef.current = null;
        switchingRef.current = false;
        setPendingSessionId(null);
      }
    }
  }, [configurationChangingRef, currentSelection, operations, processing, reasoningUpdatingRef, sessionId, switchingRef, t]);

  return { switching: pendingSessionId === sessionId && requestRef.current?.generation === generationRef.current, switchAgent };
};
