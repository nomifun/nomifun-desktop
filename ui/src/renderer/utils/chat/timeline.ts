/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Timeline utility functions for conversation history grouping
 * 会话历史分组的时间线工具函数
 */

import type { TChatConversation } from '@/common/config/storage';

/**
 * Get the activity time (most recent) from a conversation
 * 获取会话的活动时间（最近的时间）
 */
export const getActivityTime = (conversation: TChatConversation): number => {
  return conversation.modified_at || conversation.created_at || 0;
};
