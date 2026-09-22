/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useMemo, useRef, useState } from 'react';
import { Message } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import { useModelsForTask } from '@/renderer/hooks/agent/useModelsForTask';
import { refreshConversationCache } from '@/renderer/pages/conversation/utils/conversationCache';
import type { IProvider, TChatConversation } from '@/common/config/storage';
import NomiChat from '@/renderer/pages/conversation/platforms/nomi/NomiChat';
import { useNomiModelSelection } from '@/renderer/pages/conversation/platforms/nomi/useNomiModelSelection';
import type { useCompanion } from '../useNomi';
import CompanionCapabilityControls from './CompanionCapabilityControls';
import CompanionAgentIndicator from './CompanionAgentIndicator';
import CompanionAvatar from '@/renderer/pages/companion/CompanionAvatar';
import { customFigureMetaOf } from '@/renderer/pages/companion/characters/customMeta';

type NomiConversation = Extract<TChatConversation, { type: 'nomi' }>;

interface Props {
  /** 该伙伴的唯一专属 nomi 会话（由 CompanionChatPanel 载入后传入）。 */
  conversation: NomiConversation;
  /** 伙伴 profile + 乐观 patch 通道（模型唯一事实源入口）。 */
  companion: ReturnType<typeof useCompanion>;
  /** Dedicated /nomi cohabit surface: configuration lives beside the chat. */
  compact?: boolean;
}

/** Standard chat UI with companion-owned model and skills plus a fixed product Agent. */
const CompanionConversation: React.FC<Props> = ({ conversation, companion, compact = false }) => {
  const { t } = useTranslation();
  const { profile, patchCompanion } = companion;
  const { groups } = useModelsForTask('chat');
  const [modelSaving, setModelSaving] = useState(false);
  const savingRef = useRef(false);
  const initialModel = useMemo(() => {
    if (!profile?.model) return undefined;
    const provider = groups.find(({ provider }) => provider.id === profile.model?.provider_id)?.provider;
    if (provider) return { ...provider, use_model: profile.model.model };
    return { ...conversation.model, id: profile.model.provider_id, use_model: profile.model.model };
  }, [profile?.model?.provider_id, profile?.model?.model, groups, conversation.model]);
  const refreshSession = useCallback(() => {
    void refreshConversationCache(conversation.id).catch(() => {
      Message.error(t('agentSettings.productBinding.loadFailed'));
    });
  }, [conversation.id, t]);
  const onSelectModel = useCallback(async (provider: IProvider, modelName: string) => {
    if (savingRef.current) return false;
    savingRef.current = true;
    setModelSaving(true);
    try {
      const saved = await patchCompanion({ model: { provider_id: provider.id, model: modelName } });
      if (!saved) return false;
      refreshSession();
      return true;
    } catch {
      Message.error(t('nomi.chat.modelSaveFailed'));
      return false;
    } finally {
      savingRef.current = false;
      setModelSaving(false);
    }
  }, [patchCompanion, refreshSession, t]);
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
      modelSelectionHint={t('nomi.chat.modelConfigHint')}
      modelSelectionDisabled={modelSaving}
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
      agentSelectorNode={compact ? undefined : <CompanionAgentIndicator />}
      capabilityControls={compact ? undefined : <CompanionCapabilityControls companion={companion} conversation={conversation} />}
      agent_name={profile?.name}
      creationEnabled={false}
      compactProductComposer={compact}
    />
  );
};

export default CompanionConversation;
