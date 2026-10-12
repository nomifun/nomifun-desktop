/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Button, Message } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import { useModelsForTask } from '@/renderer/hooks/agent/useModelsForTask';
import { refreshConversationCache } from '@/renderer/pages/conversation/utils/conversationCache';
import type { IProvider, TChatConversation } from '@/common/config/storage';
import NomiChat from '@/renderer/pages/conversation/platforms/nomi/NomiChat';
import { useNomiModelSelection } from '@/renderer/pages/conversation/platforms/nomi/useNomiModelSelection';
import type { useCompanion } from '../useNomi';
import ProductAgentBindingSelect, { productAgentErrorReason } from '@/renderer/components/agent/ProductAgentBindingSelect';
import CompanionAvatar from '@/renderer/pages/companion/CompanionAvatar';
import { customFigureMetaOf } from '@/renderer/pages/companion/characters/customMeta';
import { getConversationRuntimeAuthority } from '@/renderer/pages/conversation/utils/conversationRuntime';
import { saveCompanionChatModel, type CompanionChatModel } from './saveCompanionChatModel';

type NomiConversation = Extract<TChatConversation, { type: 'nomi' }>;

interface Props {
  /** 该伙伴的唯一专属 nomi 会话。 */
  conversation: NomiConversation;
  /** 伙伴 profile defaults; the current Session owns its frozen model. */
  companion: ReturnType<typeof useCompanion>;
  /** Dedicated /nomi cohabit surface: configuration lives beside the chat. */
  compact?: boolean;
}

/** Standard chat UI with companion-owned model, skills and a compatible Agent. */
const CompanionConversation: React.FC<Props> = ({ conversation, companion, compact = false }) => {
  const { t } = useTranslation();
  const { profile, patchCompanion } = companion;
  const { groups } = useModelsForTask('chat');
  const [modelSaving, setModelSaving] = useState(false);
  const [agentSaving, setAgentSaving] = useState(false);
  const [retryModel, setRetryModel] = useState<{ provider: IProvider; model: string } | null>(null);
  const savingRef = useRef(false);
  const agentSavingRef = useRef(false);
  const sessionModelRef = useRef<CompanionChatModel>({ provider_id: conversation.model.id, model: conversation.model.use_model });
  useEffect(() => {
    sessionModelRef.current = { provider_id: conversation.model.id, model: conversation.model.use_model };
  }, [conversation.id, conversation.model.id, conversation.model.use_model]);
  useEffect(() => { setRetryModel(null); }, [conversation.id]);
  const initialModel = useMemo(() => {
    const provider = groups.find(({ provider }) => provider.id === conversation.model.id)?.provider;
    return provider ? { ...provider, use_model: conversation.model.use_model } : conversation.model;
  }, [groups, conversation.model]);
  const sessionBlocked = getConversationRuntimeAuthority(conversation) !== 'idle' || Boolean(conversation.runtime?.active_turn_id)
    || conversation.extra.execution_phase === 'paused' || Boolean(conversation.execution_attempt_id);
  const refreshSession = useCallback(() => {
    void refreshConversationCache(conversation.id).catch(() => {
      Message.error(t('agentSettings.productBinding.loadFailed'));
    });
  }, [conversation.id, t]);
  const onSelectModel = useCallback(async (provider: IProvider, modelName: string) => {
    if (savingRef.current || agentSavingRef.current || sessionBlocked) return false;
    savingRef.current = true;
    setModelSaving(true);
    try {
      const result = await saveCompanionChatModel({
        conversation,
        sessionModel: sessionModelRef.current,
        profileModel: profile?.model,
        next: { provider_id: provider.id, model: modelName },
        patchCompanion,
        onSessionChanged: (model) => { sessionModelRef.current = model; refreshSession(); },
      });
      if (result.status === 'default-save-failed') {
        setRetryModel({ provider, model: modelName });
        Message.error(t('nomi.cohabit.modelDefaultSaveFailed', { defaultValue: '当前会话模型已切换，默认模型保存失败，请重试。' }));
      } else {
        setRetryModel(null);
        refreshSession();
      }
      return true;
    } catch (error) {
      Message.error(t(`agentSettings.productBinding.${productAgentErrorReason(error)}`));
      return false;
    } finally {
      savingRef.current = false;
      setModelSaving(false);
    }
  }, [conversation, profile?.model, sessionBlocked, patchCompanion, refreshSession, t]);
  const modelSelection = useNomiModelSelection({
    initialModel,
    onSelectModel,
  });

  const workspace = conversation.extra?.workspace ?? '';

  return (
    <NomiChat
      conversation_id={conversation.id}
      workspace={workspace}
      modelSelection={modelSelection}
      modelSelectionHint={t('nomi.chat.sessionModelConfigHint', { defaultValue: '空闲时切换当前会话模型，下一条消息生效；同时保存为伙伴默认模型。' })}
      modelSelectionDisabled={modelSaving || agentSaving || sessionBlocked}
      emptySlot={!profile?.model
        ? <div className='p-20px text-center text-t-secondary'>{t('nomi.chat.modelMissing')}</div>
        : compact
          ? (
            <div className='h-full min-h-260px flex flex-col items-center justify-center gap-10px text-center text-t-tertiary'>
              <CompanionAvatar
                character={profile.character}
                companionId={profile.companion_id}
                customFigure={customFigureMetaOf(profile)}
                mood='content'
                activity='idle'
                size={88}
              />
              <strong className='text-15px text-t-primary'>
                {t('nomi.cohabit.emptyTitle', { name: profile.name, defaultValue: '从和 {{name}} 的第一句话开始' })}
              </strong>
              <span className='max-w-360px text-12px leading-19px'>
                {t('nomi.cohabit.emptyHint', { defaultValue: '这里会延续桌面、消息渠道和机器人中的同一段相处历史。' })}
              </span>
            </div>
          )
          : undefined}
      agentSelectorNode={compact || !profile ? undefined : <ProductAgentBindingSelect
        compact
        targetKind='companion'
        targetId={profile.companion_id}
        defaultTemplateKey='companion.default'
        model={conversation.model}
        conversationId={conversation.id}
        disabled={modelSaving || sessionBlocked}
        onChanged={refreshSession}
        onSavingChange={(saving) => { agentSavingRef.current = saving; setAgentSaving(saving); }}
      />}
      extraRightTools={!compact && retryModel ? <Button size='mini' type='text' loading={modelSaving} disabled={agentSaving || sessionBlocked}
        onClick={() => void onSelectModel(retryModel.provider, retryModel.model)}>{t('agentSettings.actions.retry')}</Button> : undefined}
      agent_name={profile?.name}
      creationEnabled={false}
      creationTasksEnabled={conversation.agent_snapshot?.enabled_capabilities.includes('creation.media') === true}
      compactProductComposer={compact}
      followTurnCompletion
    />
  );
};

export default CompanionConversation;
