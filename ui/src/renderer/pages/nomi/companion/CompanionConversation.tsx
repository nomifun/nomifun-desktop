/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useMemo, useRef, useState } from 'react';
import { Message } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import ProductAgentBindingSelect from '@/renderer/components/agent/ProductAgentBindingSelect';
import { useModelsForTask } from '@/renderer/hooks/agent/useModelsForTask';
import { refreshConversationCache } from '@/renderer/pages/conversation/utils/conversationCache';
import type { IProvider, TChatConversation } from '@/common/config/storage';
import NomiChat from '@/renderer/pages/conversation/platforms/nomi/NomiChat';
import { useNomiModelSelection } from '@/renderer/pages/conversation/platforms/nomi/useNomiModelSelection';
import type { useCompanion } from '../useNomi';
import CompanionCapabilityControls from './CompanionCapabilityControls';

type NomiConversation = Extract<TChatConversation, { type: 'nomi' }>;

interface Props {
  /** 该伙伴的唯一专属 nomi 会话（由 CompanionChatPanel 载入后传入）。 */
  conversation: NomiConversation;
  /** 伙伴 profile + 乐观 patch 通道（模型唯一事实源入口）。 */
  companion: ReturnType<typeof useCompanion>;
}

/** Standard conversation UI with companion-owned model, Agent and skill settings. */
const CompanionConversation: React.FC<Props> = ({ conversation, companion }) => {
  const { t } = useTranslation();
  const { profile, patchCompanion } = companion;
  const { groups } = useModelsForTask('chat');
  const [modelSaving, setModelSaving] = useState(false);
  const [agentSaving, setAgentSaving] = useState(false);
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
    if (savingRef.current || agentSaving) return false;
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
  }, [agentSaving, patchCompanion, refreshSession, t]);
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
      modelSelectionDisabled={modelSaving || agentSaving}
      emptySlot={!profile?.model ? <div className='p-20px text-center text-t-secondary'>{t('nomi.chat.modelMissing')}</div> : undefined}
      agentSelectorNode={profile && <ProductAgentBindingSelect
        compact
        targetKind='companion'
        targetId={profile.companion_id}
        defaultTemplateKey='companion.default'
        conversationId={conversation.id}
        model={initialModel}
        disabled={modelSaving}
        onSavingChange={setAgentSaving}
        onChanged={refreshSession}
      />}
      capabilityControls={<CompanionCapabilityControls companion={companion} conversation={conversation} />}
      agent_name={profile?.name}
    />
  );
};

export default CompanionConversation;
