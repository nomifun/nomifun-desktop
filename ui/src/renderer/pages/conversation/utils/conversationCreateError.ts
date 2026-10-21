/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { isBackendHttpError, redactSensitiveText, type BackendHttpError } from '@/common/adapter/httpBridge';
import { parseError } from '@/common/utils';
import type { TFunction } from 'i18next';
import { conversationConfigurationErrorCode } from './conversationConfigurationError';

type WorkspacePathErrorCode =
  | 'WORKSPACE_PATH_EDGE_WHITESPACE_UNSUPPORTED'
  | 'WORKSPACE_PATH_EDGE_WHITESPACE_RUNTIME_UNSUPPORTED'
  | 'WORKSPACE_DIRECTORY_UNAVAILABLE'
  | 'WORKSPACE_DIRECTORY_RUNTIME_UNAVAILABLE';

type ConversationCreateErrorCode =
  | 'WORKSPACE_PATH_EDGE_WHITESPACE_UNSUPPORTED'
  | 'WORKSPACE_DIRECTORY_UNAVAILABLE';
type ConversationRuntimeWorkspaceErrorCode =
  | 'WORKSPACE_PATH_EDGE_WHITESPACE_RUNTIME_UNSUPPORTED'
  | 'WORKSPACE_DIRECTORY_RUNTIME_UNAVAILABLE';

const BACKEND_ERROR_CODE_MAP: Record<string, WorkspacePathErrorCode> = {
  WORKSPACE_PATH_EDGE_WHITESPACE_UNSUPPORTED: 'WORKSPACE_PATH_EDGE_WHITESPACE_UNSUPPORTED',
  WORKSPACE_PATH_EDGE_WHITESPACE_RUNTIME_UNSUPPORTED: 'WORKSPACE_PATH_EDGE_WHITESPACE_RUNTIME_UNSUPPORTED',
  WORKSPACE_DIRECTORY_UNAVAILABLE: 'WORKSPACE_DIRECTORY_UNAVAILABLE',
  WORKSPACE_DIRECTORY_RUNTIME_UNAVAILABLE: 'WORKSPACE_DIRECTORY_RUNTIME_UNAVAILABLE',
};

const PROVIDER_ERROR_CODES = new Set(['PROVIDER_UNAVAILABLE']);

/** i18n key for a provider-config error code, or undefined if not one. */
export function providerErrorI18nKey(code: string): string | undefined {
  return PROVIDER_ERROR_CODES.has(code) ? `conversation.agentError.codes.${code}.body` : undefined;
}

type EmbeddedBackendErrorPayload = {
  code?: string;
  error?: string;
  details?: unknown;
};

type WorkspacePathErrorDetails = {
  workspace_path?: string;
};

const getEmbeddedBackendErrorPayload = (error: unknown): EmbeddedBackendErrorPayload | undefined => {
  const parsedError = parseError(error);
  const rawMessage =
    typeof error === 'string'
      ? error
      : error instanceof Error
        ? error.message
        : typeof parsedError === 'string'
          ? parsedError
          : '';

  if (!rawMessage) {
    return undefined;
  }

  const jsonStart = rawMessage.indexOf('{');
  if (jsonStart < 0) {
    return undefined;
  }

  try {
    const payload = JSON.parse(rawMessage.slice(jsonStart)) as EmbeddedBackendErrorPayload;
    if (payload && typeof payload === 'object') {
      return payload;
    }
  } catch {
    return undefined;
  }

  return undefined;
};

const getWorkspacePathFromDetails = (details: unknown): string | undefined => {
  if (!details || typeof details !== 'object') {
    return undefined;
  }

  const workspacePath = (details as WorkspacePathErrorDetails).workspace_path;
  return typeof workspacePath === 'string' && workspacePath.trim() ? workspacePath : undefined;
};

const getWorkspacePathErrorPayload = (error: unknown): EmbeddedBackendErrorPayload | undefined => {
  if (isBackendHttpError(error)) {
    return {
      code: error.code,
      error: error.backendMessage,
      details: error.details,
    };
  }

  return getEmbeddedBackendErrorPayload(error);
};

const getConversationConfigurationErrorKey = (payload: EmbeddedBackendErrorPayload | undefined): string | undefined => {
  const code = conversationConfigurationErrorCode(payload);
  return code ? `conversation.agentError.codes.${code}.body` : undefined;
};

/** Only the bridge's redacted message and bounded typed diagnostics enter launch errors. */
export const getAgentPreparationDiagnostics = (error: BackendHttpError): { codes: string[]; reasons: string[] } => {
  const codes = error.code ? [error.code] : [];
  const reasons = [[error.code ? `[${error.code}]` : '', error.backendMessage].filter(Boolean).join(' ')];
  const diagnostics = (error.details as { diagnostics?: unknown[] } | undefined)?.diagnostics;
  if (Array.isArray(diagnostics)) {
    for (const entry of diagnostics.slice(0, 5)) {
      if (!entry || typeof entry !== 'object') continue;
      const diagnostic = entry as { code?: unknown; subject?: unknown; message?: unknown };
      if (typeof diagnostic.code === 'string') codes.push(diagnostic.code);
      reasons.push([diagnostic.code, diagnostic.subject, diagnostic.message]
        .filter((value): value is string => typeof value === 'string' && Boolean(value.trim()))
        .join(': '));
    }
  }
  return { codes, reasons };
};

const getWorkspacePathFromErrorDetails = (error: unknown): string | undefined => {
  const payload = getWorkspacePathErrorPayload(error);
  return getWorkspacePathFromDetails(payload?.details);
};

const normalizeWorkspacePathErrorCode = (error: unknown): WorkspacePathErrorCode | undefined => {
  const payload = getWorkspacePathErrorPayload(error);
  if (payload) {
    const mappedCode = payload.code ? BACKEND_ERROR_CODE_MAP[payload.code] : undefined;
    if (mappedCode) {
      return mappedCode;
    }
  }

  return undefined;
};

const normalizeConversationCreateErrorCode = (error: unknown): ConversationCreateErrorCode | undefined => {
  const code = normalizeWorkspacePathErrorCode(error);
  return code === 'WORKSPACE_PATH_EDGE_WHITESPACE_UNSUPPORTED' || code === 'WORKSPACE_DIRECTORY_UNAVAILABLE'
    ? code
    : undefined;
};

const normalizeConversationRuntimeWorkspaceErrorCode = (
  error: unknown
): ConversationRuntimeWorkspaceErrorCode | undefined => {
  const code = normalizeWorkspacePathErrorCode(error);
  return code === 'WORKSPACE_PATH_EDGE_WHITESPACE_RUNTIME_UNSUPPORTED'
    || code === 'WORKSPACE_DIRECTORY_RUNTIME_UNAVAILABLE'
    ? code
    : undefined;
};

export const getConversationCreateErrorMessage = (error: unknown, t: TFunction): string => {
  const normalizedCode = normalizeConversationCreateErrorCode(error);
  const payload = getWorkspacePathErrorPayload(error);
  const configurationCode = conversationConfigurationErrorCode(payload);
  const workspacePath = getWorkspacePathFromErrorDetails(error);
  const rawMessage = payload?.error || parseError(error) || t('conversation.createFailed');
  // Preparation has no Session to replace. The hint describes configuration,
  // while the original safe code and diagnostic remain available to investigate.
  const hint = configurationCode ? t('guid.agentEntries.contractChanged')
    : payload?.code === 'NOMIFUN_AGENT_RESOURCE_BINDING_INCOMPLETE'
      ? t('conversation.agentError.codes.NOMIFUN_AGENT_RESOURCE_BINDING_INCOMPLETE.body')
      : normalizedCode && workspacePath ? t(`conversation.createError.pathVariants.${normalizedCode}`, {
        workspacePath, defaultValue: rawMessage,
      }) : undefined;
  const reasons = isBackendHttpError(error) ? getAgentPreparationDiagnostics(error).reasons : [
    configurationCode
      ? redactSensitiveText([payload?.code ? `[${payload.code}]` : '', rawMessage].filter(Boolean).join(' '))
      : rawMessage,
  ];
  return [...new Set([hint, ...reasons].filter(Boolean))].join('\n');
};

export const getConversationRuntimeWorkspaceErrorMessage = (error: unknown, t: TFunction): string => {
  const payload = getWorkspacePathErrorPayload(error);
  if (payload?.code === 'NOMIFUN_AGENT_RESOURCE_BINDING_INCOMPLETE') return t('conversation.agentError.codes.NOMIFUN_AGENT_RESOURCE_BINDING_INCOMPLETE.body');
  const configurationKey = getConversationConfigurationErrorKey(payload);
  if (configurationKey) {
    return t(configurationKey);
  }
  const providerKey = payload?.code ? providerErrorI18nKey(payload.code) : undefined;
  if (providerKey) {
    return t(providerKey);
  }
  const normalizedCode = normalizeConversationRuntimeWorkspaceErrorCode(error);
  const workspacePath = getWorkspacePathFromErrorDetails(error);
  const rawMessage = payload?.error || parseError(error) || t('common.unknownError');

  if (normalizedCode) {
    if (workspacePath) {
      return t(`conversation.agentError.codes.${normalizedCode}.bodyWithPath`, {
        workspacePath,
        defaultValue: rawMessage,
      });
    }

    return t(`conversation.agentError.codes.${normalizedCode}.body`, {
      defaultValue: rawMessage,
    });
  }

  return rawMessage;
};
