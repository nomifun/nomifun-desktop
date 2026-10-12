/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useRef, useState } from 'react';
import { Button, Message } from '@arco-design/web-react';
import { Link, useNavigate } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import { parseError, uuidv7 } from '@/common/utils';
import type { CompanionId, ConversationId } from '@/common/types/ids';
import type { PreviewAgentSessionSwitchResponse } from '@/common/types/agentPlatform';
import { moduleI18nKey } from '@/renderer/pages/agentSettings/model';
import { seedConversationCache } from '@/renderer/pages/conversation/utils/conversationCache';
import CompanionAgentIndicator from './CompanionAgentIndicator';
import styles from './CompanionAgentControls.module.css';

interface Props {
  companionId: CompanionId;
  sessionId?: ConversationId;
  surface?: 'composer' | 'settings';
}

interface UpdateState {
  sessionId?: ConversationId;
  preview?: PreviewAgentSessionSwitchResponse;
  idempotencyKey?: string;
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
  const busyRef = useRef(false);
  const generationRef = useRef(0);
  useEffect(() => {
    generationRef.current += 1;
    busyRef.current = false;
    setUpdate(null);
    return () => { generationRef.current += 1; };
  }, [companionId, sessionId]);

  const showFailure = (error: unknown) => {
    const errorCode = isBackendHttpError(error) ? error.code : undefined;
    if (errorCode === 'AGENT_SESSION_AGENT_UNCHANGED') {
      setUpdate(null);
      Message.info(t('nomi.chat.agentAlreadyCurrent'));
      return;
    }
    const message = errorCode
        ? t(`conversation.chat.agentSwitch.blockers.${errorCode}`, { defaultValue: parseError(error) })
        : parseError(error);
    setUpdate((current) => current && ({ ...current, loading: false, applying: false, error: message, errorCode,
      needsPreview: errorCode === 'AGENT_SESSION_BINDING_CHANGED',
    }));
  };

  const previewUpdate = async () => {
    if (busyRef.current) return;
    busyRef.current = true;
    const generation = generationRef.current;
    setUpdate({ loading: true, applying: false });
    try {
      const currentSessionId = sessionId ?? (await ipcBridge.companion.getCompanionSession.invoke({ companion_id: companionId })).conversation_id;
      if (generation !== generationRef.current) return;
      if (!currentSessionId) {
        setUpdate(null);
        Message.info(t('nomi.chat.agentNoSession'));
        return;
      }
      const preview = await ipcBridge.agentPlatform.sessions.previewAgentSwitch.invoke({
        agent_session_id: currentSessionId,
        request: { selection },
      });
      if (generation !== generationRef.current) return;
      setUpdate({ sessionId: currentSessionId, preview, idempotencyKey: uuidv7(), loading: false, applying: false });
    } catch (error) {
      if (generation !== generationRef.current) return;
      showFailure(error);
    } finally {
      if (generation === generationRef.current) busyRef.current = false;
    }
  };

  const applyUpdate = async () => {
    if (!update?.sessionId || !update.preview?.can_apply || !update.idempotencyKey || busyRef.current || update.needsPreview) return;
    busyRef.current = true;
    const generation = generationRef.current;
    const current = update;
    setUpdate({ ...current, applying: true, error: undefined, errorCode: undefined });
    try {
      const result = await ipcBridge.agentPlatform.sessions.applyAgentSwitch.invoke({
        agent_session_id: current.sessionId!,
        idempotency_key: current.idempotencyKey!,
        request: {
          selection,
          handoff_mode: current.preview!.handoff.available ? 'continue_task' : 'context_only',
          expected_binding_version: current.preview!.expected_binding_version,
          expected_resolved_snapshot_ref: current.preview!.target.resolved_snapshot_ref,
        },
      });
      await seedConversationCache(result.conversation).catch(error => console.error('[Companion] Failed to refresh updated Agent:', error));
      if (generation !== generationRef.current) return;
      setUpdate(null);
      Message.success(t('nomi.chat.agentUpdateSuccess'));
    } catch (error) {
      if (generation !== generationRef.current) return;
      showFailure(error);
    } finally {
      if (generation === generationRef.current) busyRef.current = false;
    }
  };

  const moduleLabel = (id: string) => {
    const key = moduleI18nKey(id);
    return key ? t(`agentSettings.modules.${key}.name`, { defaultValue: id }) : id;
  };
  const blocker = update?.preview?.blockers[0];
  const errorCode = update?.errorCode ?? blocker?.code;
  const error = update?.error ?? (blocker ? t(`conversation.chat.agentSwitch.blockers.${blocker.code}`, { defaultValue: blocker.message }) : undefined);
  const dismissUpdate = () => {
    if (update?.applying) return;
    generationRef.current += 1;
    busyRef.current = false;
    setUpdate(null);
  };

  return (
    <div className={styles.controls}>
      <CompanionAgentIndicator surface={surface} />
      <Link className='text-12px text-primary' to={workbenchPath}>{t('nomi.chat.editAgent')}</Link>
      <Button size='mini' type='text' disabled={Boolean(update?.loading || update?.applying)} onClick={() => { void previewUpdate(); }}>
        {t('nomi.chat.applyAgentUpdates')}
      </Button>
      {update && <section className={styles.update} aria-label={t('nomi.chat.agentUpdateTitle')} data-testid='companion-agent-update'>
        {update.loading && <span role='status'>{t('conversation.chat.agentSwitch.loading')}</span>}
        {update.preview && <>
          <strong>{t('nomi.chat.agentUpdateRevision', { current: update.preview.current.preset_revision, target: update.preview.target.preset_revision })}</strong>
          <p>{t('nomi.chat.agentUpdateHint')}</p>
          {update.preview.capabilities.gained.length > 0 && <p>{t('nomi.chat.agentUpdateAdded', { names: update.preview.capabilities.gained.map(moduleLabel).join(' · ') })}</p>}
          {update.preview.capabilities.lost.length > 0 && <p>{t('nomi.chat.agentUpdateRemoved', { names: update.preview.capabilities.lost.map(moduleLabel).join(' · ') })}</p>}
        </>}
        {error && <p role='alert'>{error}</p>}
        <div className={styles.actions}>
          <Button size='mini' type='text' disabled={update.applying} onClick={dismissUpdate}>{t('common.close')}</Button>
          {update.needsPreview && <Button size='mini' type='text' onClick={() => { void previewUpdate(); }}>{t('nomi.chat.previewAgentUpdates')}</Button>}
          {errorCode === 'AGENT_SESSION_MODEL_INCOMPATIBLE' && <Button size='mini' type='text' onClick={() => navigate('/models?section=chat')}>{t('conversation.chat.agentSwitch.openModelHub')}</Button>}
          {errorCode === 'AGENT_SESSION_RESOURCE_REQUIRED' && <Button size='mini' type='text' onClick={() => navigate(workbenchPath)}>{t('conversation.chat.agentSwitch.openAgentSettings')}</Button>}
          {update.preview && <Button size='mini' type='primary' loading={update.applying}
            disabled={!update.preview.can_apply || update.loading || update.applying || update.needsPreview}
            onClick={() => { void applyUpdate(); }}>{t('nomi.chat.confirmAgentUpdates')}</Button>}
        </div>
      </section>}
    </div>
  );
};

export default CompanionAgentControls;
