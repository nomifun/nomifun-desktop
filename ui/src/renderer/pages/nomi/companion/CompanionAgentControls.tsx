/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useState } from 'react';
import { Button, Message } from '@arco-design/web-react';
import { Link, useNavigate } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import { parseError, uuidv7 } from '@/common/utils';
import type { CompanionId, ConversationId } from '@/common/types/ids';
import type { AgentHandoffMode, PreviewAgentSessionSwitchResponse } from '@/common/types/agentPlatform';
import AgentSwitchDialog from '@/renderer/pages/conversation/components/AgentSwitchDialog';
import { seedConversationCache } from '@/renderer/pages/conversation/utils/conversationCache';
import CompanionAgentIndicator from './CompanionAgentIndicator';

interface Props {
  companionId: CompanionId;
  sessionId?: ConversationId;
  surface?: 'composer' | 'settings';
}

interface UpdateState {
  sessionId?: ConversationId;
  preview?: PreviewAgentSessionSwitchResponse;
  idempotencyKey?: string;
  mode: AgentHandoffMode;
  loading: boolean;
  applying: boolean;
  error?: string;
  errorCode?: string;
  needsPreview?: boolean;
}

const selection = { kind: 'template', template_key: 'companion.default' } as const;
const workbenchPath = '/agent?template=companion.default';

/** The Companion keeps its product identity; saved edits require an explicit Session transition. */
const CompanionAgentControls: React.FC<Props> = ({ companionId, sessionId, surface = 'composer' }) => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [update, setUpdate] = useState<UpdateState | null>(null);
  const label = t('agentSettings.template.companion.default.name');

  const showFailure = (error: unknown) => {
    const errorCode = isBackendHttpError(error) ? error.code : undefined;
    const message = errorCode === 'AGENT_SESSION_AGENT_UNCHANGED'
      ? t('nomi.chat.agentAlreadyCurrent')
      : errorCode
        ? t(`conversation.chat.agentSwitch.blockers.${errorCode}`, { defaultValue: parseError(error) })
        : parseError(error);
    setUpdate((current) => current && ({ ...current, loading: false, applying: false, error: message, errorCode,
      needsPreview: errorCode === 'AGENT_SESSION_BINDING_CHANGED',
    }));
  };

  const previewUpdate = async () => {
    if (update?.loading || update?.applying) return;
    setUpdate({ loading: true, applying: false, mode: 'context_only' });
    try {
      const currentSessionId = sessionId ?? (await ipcBridge.companion.getCompanionSession.invoke({ companion_id: companionId })).conversation_id;
      if (!currentSessionId) {
        setUpdate(null);
        Message.info(t('nomi.chat.agentNoSession'));
        return;
      }
      const preview = await ipcBridge.agentPlatform.sessions.previewAgentSwitch.invoke({
        agent_session_id: currentSessionId,
        request: { selection },
      });
      setUpdate({ sessionId: currentSessionId, preview, idempotencyKey: uuidv7(), loading: false, applying: false, mode: 'context_only' });
    } catch (error) {
      showFailure(error);
    }
  };

  const applyUpdate = async () => {
    if (!update?.sessionId || !update.preview?.can_apply || !update.idempotencyKey || update.applying || update.needsPreview) return;
    const current = update;
    setUpdate({ ...current, applying: true, error: undefined, errorCode: undefined });
    try {
      const result = await ipcBridge.agentPlatform.sessions.applyAgentSwitch.invoke({
        agent_session_id: current.sessionId!,
        idempotency_key: current.idempotencyKey!,
        request: {
          selection,
          handoff_mode: current.mode,
          expected_binding_version: current.preview!.expected_binding_version,
          expected_resolved_snapshot_ref: current.preview!.target.resolved_snapshot_ref,
        },
      });
      await seedConversationCache(result.conversation);
      setUpdate(null);
      Message.success(t('nomi.chat.agentUpdateSuccess'));
    } catch (error) {
      showFailure(error);
    }
  };

  return (
    <div className='min-w-0 flex flex-wrap items-center gap-6px'>
      <CompanionAgentIndicator surface={surface} />
      <Link className='text-12px text-primary' to={workbenchPath}>{t('nomi.chat.editAgent')}</Link>
      <Button size='mini' type='text' disabled={Boolean(update?.loading || update?.applying)} onClick={() => { void previewUpdate(); }}>
        {t('nomi.chat.applyAgentUpdates')}
      </Button>
      {update && <AgentSwitchDialog
        visible
        title={t('nomi.chat.agentUpdateTitle')}
        confirmLabel={t('nomi.chat.applyAgentUpdates')}
        preview={update.needsPreview && update.preview ? { ...update.preview, can_apply: false } : update.preview}
        currentAgentLabel={label}
        targetAgentLabel={label}
        loading={update.loading}
        applying={update.applying}
        mode={update.mode}
        error={update.error}
        errorCode={update.errorCode}
        recoveryLabel={update.needsPreview ? t('nomi.chat.previewAgentUpdates') : undefined}
        onModeChange={(mode) => setUpdate((current) => current && ({ ...current, mode }))}
        onConfirm={() => { void applyUpdate(); }}
        onCancel={() => { if (!update.applying) setUpdate(null); }}
        onRecovery={(code) => {
          if (update.needsPreview) { void previewUpdate(); return; }
          navigate(code === 'AGENT_SESSION_MODEL_INCOMPATIBLE' ? '/models?section=chat' : workbenchPath);
        }}
      />}
    </div>
  );
};

export default CompanionAgentControls;
