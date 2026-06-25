/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 * Based on AionUi (https://github.com/iOfficeAI/AionUi)
 */

import { isBackendHttpError } from '@/common/adapter/httpBridge';
import { getWorkspacePathFromErrorDetails, normalizeWorkspacePathErrorCode } from '../../utils/conversationCreateError';
import type { AgentStreamErrorInfo } from '@/common/chat/chatLib';

const isConversationBusyError = (error: unknown): boolean => {
  if (!isBackendHttpError(error)) return false;
  if (error.status !== 409 || error.code !== 'CONFLICT') return false;
  return error.backendMessage.toLowerCase().includes('already processing');
};

export const buildSendFailureError = (error: unknown, message: string): AgentStreamErrorInfo => {
  const workspacePathErrorCode = normalizeWorkspacePathErrorCode(error);
  if (workspacePathErrorCode) {
    const workspacePath = getWorkspacePathFromErrorDetails(error);
    return {
      message,
      code: workspacePathErrorCode,
      ownership: 'nomifun',
      detail: message,
      ...(workspacePath ? { workspacePath } : {}),
      retryable: false,
      feedback_recommended: false,
    };
  }

  if (isBackendHttpError(error) && error.code === 'BAD_GATEWAY') {
    return {
      message,
      code: 'UNKNOWN_UPSTREAM_ERROR',
      ownership: 'unknown_upstream',
      detail: message,
      retryable: true,
      feedback_recommended: true,
    };
  }

  if (isConversationBusyError(error)) {
    return {
      message,
      code: 'NOMIFUN_CONVERSATION_BUSY',
      ownership: 'nomifun',
      detail: message,
      retryable: false,
      feedback_recommended: false,
      resolution: { kind: 'wait_for_current_response' },
    };
  }

  return {
    message,
    code: 'NOMIFUN_INTERNAL_ERROR',
    ownership: 'nomifun',
    detail: message,
    retryable: true,
    feedback_recommended: true,
  };
};
