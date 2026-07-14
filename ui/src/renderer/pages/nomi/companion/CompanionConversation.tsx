/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback } from 'react';
import type { IProvider, TChatConversation } from '@/common/config/storage';
import NomiChat from '@/renderer/pages/conversation/platforms/nomi/NomiChat';
import { useNomiModelSelection } from '@/renderer/pages/conversation/platforms/nomi/useNomiModelSelection';
import type { useCompanion } from '../useNomi';

type NomiConversation = Extract<TChatConversation, { type: 'nomi' }>;

interface Props {
  /** 该伙伴的唯一专属 nomi 会话（由 CompanionChatPanel 载入后传入）。 */
  conversation: NomiConversation;
  /** 伙伴 profile + 乐观 patch 通道（模型唯一事实源入口）。 */
  companion: ReturnType<typeof useCompanion>;
}

/**
 * 桌面伙伴会话的受限聊天主体：复用工作台会话页的完整交互能力
 * （MessageList 富渲染：工具卡/思考流/产物/文件变更/Markdown；NomiSendBox：附件 /
 * 斜杠命令 / 命令队列 / 停止 / 清空上下文；右侧工作区文件树；按需文档预览），
 * 但针对桌面伙伴做两处约束（不污染共享会话页，靠既有 props 开关达成）：
 *
 *  1) 锁定模型：不渲染会话页的 NomiModelSelector；`modelSelection` 锁定到会话行的模型
 *     —— 后端 `patch_companion` 已把会话行模型同步成 `profile.model`（唯一事实源），
 *     `onSelectModel` 空操作禁止 per-conversation 改写。模型配置入口仅保留头部
 *     CompanionModelControl（写 profile.model，全局跟随）。
 *  2) 锁定工作路径 + yolo：workspace = 后端固定的伙伴专属目录；session_mode 固定
 *     'yolo' 且 `hideModeSelector` 隐藏权限选择器
 *     （伙伴会话后端强制 yolo 无审批，详见 companion.rs）。
 *
 * 外层标准 ChatLayout、工作区、模型入口和 AgentExecution 投影统一由
 * CompanionChatPanel 持有，保证加载/异常/模型缺失状态也不会丢失执行画布。
 */
const CompanionConversation: React.FC<Props> = ({ conversation, companion }) => {
  // 锁定版 modelSelection：current_model = 会话行模型（= profile.model，后端同步保证），
  // 选择动作空操作（伙伴模型只经 CompanionModelControl → patchCompanion 修改，全局生效）。
  const lockedSelect = useCallback(async (_provider: IProvider, _modelName: string) => false, []);
  const modelSelection = useNomiModelSelection({
    initialModel: conversation.model,
    onSelectModel: lockedSelect,
  });

  const workspace = conversation.extra?.workspace ?? '';

  return (
    <NomiChat
      conversation_id={conversation.id}
      workspace={workspace}
      modelSelection={modelSelection}
      session_mode='yolo'
      hideModeSelector
      agent_name={companion.profile?.name}
    />
  );
};

export default CompanionConversation;
