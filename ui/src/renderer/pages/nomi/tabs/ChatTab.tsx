/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Message, Spin } from '@arco-design/web-react';
import useSWR from 'swr';
import { ipcBridge } from '@/common';
import type { ICompanionStatus } from '@/common/adapter/ipcBridge';
import { getConversationOrNull } from '@/renderer/pages/conversation/utils/conversationCache';
import CompanionConversation from '../companion/CompanionConversation';
import CompanionModelControl from '../CompanionModelControl';
import type { useCompanion } from '../useNomi';

interface Props {
  companionId: string;
  companionName: string;
  status: ICompanionStatus | null;
  /** 伙伴 profile + 乐观 patch 通道（模型唯一事实源写入口）。 */
  companion: ReturnType<typeof useCompanion>;
}

/**
 * 桌面伙伴「聊天」Tab —— 单会话体验，平移自工作台会话页的完整交互能力，
 * 但隐藏高级功能（AutoWork/IDMM/知识库/多智能体）、锁定模型与工作路径（详见
 * CompanionConversation）。本组件只负责：① 模型未配置态的引导（含模型配置入口）；
 * ② 解析（幂等 ensure）该伙伴的唯一专属会话 id；③ 载入会话对象并交给
 * CompanionConversation 渲染。会话头部（含模型配置）已收敛进会话布局，避免叠头。
 *
 * 不变量：单会话契约（仅 ensureCompanionSession，无新建/重命名/多线程入口）；
 * 切伙伴由父级 `key` + 本组件 effect 先置空再解析防 stale；未配置模型不挂载会话主体。
 */
const ChatTab: React.FC<Props> = ({ companionId, status, companion }) => {
  const { t } = useTranslation();
  const { profile } = companion;

  const [conversationId, setConversationId] = useState<number | null>(null);
  const [resolving, setResolving] = useState(false);

  const modelConfigured = status ? status.model_configured : Boolean(profile?.model.provider_id && profile?.model.model);

  // 模型已配置后解析（幂等 ensure）该伙伴的唯一会话 id。切换伙伴 / 从未配置→已配置时触发。
  // ensure 同时会为旧伙伴补写固定工作目录（后端），故随后载入的会话对象必带 workspace。
  useEffect(() => {
    setConversationId(null);
    if (!modelConfigured) return;
    let cancelled = false;
    setResolving(true);
    void ipcBridge.companion.ensureCompanionSession
      .invoke({ companion_id: companionId })
      .then((thread) => {
        if (!cancelled) setConversationId(thread.conversation_id);
      })
      .catch((e) => {
        if (!cancelled) Message.error(String(e));
      })
      .finally(() => {
        if (!cancelled) setResolving(false);
      });
    return () => {
      cancelled = true;
    };
  }, [companionId, modelConfigured]);

  // 载入会话对象（含 type/model/extra.workspace），交给会话布局渲染。getConversationOrNull
  // 直连后端无缓存，确保拿到刚补写的 workspace。
  const { data: conversation, isLoading: convLoading } = useSWR(
    conversationId != null ? `companion-conversation/${conversationId}` : null,
    () => getConversationOrNull(conversationId as number)
  );

  if (!profile) {
    return (
      <div className='flex-1 flex justify-center items-center py-40px'>
        <Spin />
      </div>
    );
  }

  // 模型未配置：把模型配置入口（唯一事实源）放在引导态，配置后才会解析会话并进入对话。
  if (!modelConfigured) {
    return (
      <div className='flex flex-col h-full min-h-0 items-center justify-center gap-14px px-16px text-center'>
        <CompanionModelControl companion={companion} />
        <div className='text-13px text-t-tertiary'>{t('nomi.chat.modelMissing')}</div>
      </div>
    );
  }

  // 仍在解析会话 id 或载入会话对象：Spin。
  if (conversationId == null || convLoading) {
    return (
      <div className='flex-1 flex justify-center items-center py-40px'>{resolving || convLoading ? <Spin /> : null}</div>
    );
  }

  // 载入完毕但拿不到会话（被带外删除→404）或类型异常：兜底文案，避免空白面板。
  // 伙伴会话恒为 type='nomi'（companion.rs 固定）。
  if (!conversation || conversation.type !== 'nomi') {
    return (
      <div className='flex-1 flex items-center justify-center text-13px text-t-tertiary px-16px text-center'>
        {t('nomi.companion.chatError')}
      </div>
    );
  }

  return <CompanionConversation key={conversationId} conversation={conversation} companion={companion} />;
};

export default ChatTab;
