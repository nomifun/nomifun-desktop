/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import i18n, { type TFunction } from 'i18next';
import {
  isBackendHttpError,
  isBackendRequestError,
  isHandledAuthExpiredHttpError,
  redactSensitiveText,
} from '@/common/adapter/httpBridge';

export type KnowledgeErrorContext = 'operation' | 'load' | 'export';

export interface KnowledgeErrorPresentation {
  readonly type: 'knowledge-error';
  readonly context: KnowledgeErrorContext;
  readonly title: string;
  readonly message: string;
  readonly technicalDetails: string;
  /** WebUI authentication feedback is already owned by the login flow. */
  readonly handled: boolean;
}

function isPresentation(error: unknown): error is KnowledgeErrorPresentation {
  if (!error || typeof error !== 'object') return false;
  const value = error as Partial<KnowledgeErrorPresentation>;
  return value.type === 'knowledge-error'
    && typeof value.title === 'string'
    && typeof value.message === 'string'
    && typeof value.technicalDetails === 'string'
    && typeof value.handled === 'boolean'
    && ['operation', 'load', 'export'].includes(value.context ?? '');
}

function technicalDetails(error: unknown): string {
  if (isBackendHttpError(error)) {
    // httpBridge has already redacted credentials in its structured body and
    // message. Preserve that diagnostic instead of showing it as user copy.
    return redactSensitiveText(`${error.name}: ${error.message}`);
  }
  if (error instanceof Error) return redactSensitiveText(`${error.name}: ${error.message}`);
  if (typeof error === 'string') return redactSensitiveText(error);
  // Arbitrary thrown objects may contain credentials. Do not dump them into
  // UI or clipboard outside the bridge's redacted response contract.
  return 'Unknown error';
}

/** A small presentation boundary shared by knowledge pages and their dialogs. */
export function normalizeKnowledgeError(
  error: unknown,
  context: KnowledgeErrorContext = 'operation',
  t: TFunction = i18n.t.bind(i18n)
): KnowledgeErrorPresentation {
  if (isPresentation(error)) return error;

  const text = (key: string, fallback: string): string => t(key, { defaultValue: fallback }) || fallback;
  const title = context === 'export'
    ? text('knowledge.errors.exportTitle', '资料导出失败')
    : context === 'load'
      ? text('knowledge.errors.loadTitle', '知识库加载失败')
      : text('knowledge.errors.operationTitle', '知识库操作失败');
  let message = context === 'export'
    ? text('knowledge.errors.exportFailed', '导出文件未能保存。请重新选择 ZIP 文件的保存位置后再试。')
    : context === 'load'
      ? text('knowledge.errors.loadFailed', '暂时无法加载知识库，请重试；如果仍然失败，可复制技术信息反馈。')
      : text('knowledge.errors.operationFailed', '操作未能完成，请稍后重试。');

  if (isBackendRequestError(error)) {
    if (error.kind === 'timeout') {
      message = context === 'export'
        ? text('knowledge.errors.exportTimeout', '等待导出结果超时。请先检查保存位置是否已生成文件，再决定是否重新导出。')
        : context === 'load'
          ? text('knowledge.errors.loadTimeout', '知识库加载超时，请检查服务连接后重试。')
          : text('knowledge.errors.operationTimeout', '等待操作结果超时。请先检查知识库当前状态，再决定是否重试。');
    } else {
      message = context === 'export'
        ? text('knowledge.errors.exportNetwork', '连接已中断，暂时无法确认导出结果。请先检查保存位置是否已生成文件，再决定是否重新导出。')
        : context === 'load'
          ? text('knowledge.errors.loadNetwork', '无法连接知识库服务，请检查连接后重试。')
          : text('knowledge.errors.operationNetwork', '连接已中断，暂时无法确认操作结果。请检查连接和知识库当前状态后再试。');
    }
  } else if (isBackendHttpError(error)) {
    const stage = error.details && typeof error.details === 'object'
      ? (error.details as { stage?: unknown }).stage
      : undefined;
    switch (error.code) {
      case 'KNOWLEDGE_EXPORT_DESTINATION_INVALID':
        message = text('knowledge.errors.exportDestinationInvalid', '保存位置无效。请选择 ZIP 文件的完整保存路径后再试。');
        break;
      case 'KNOWLEDGE_EXPORT_DESTINATION_EXISTS':
        message = text('knowledge.errors.exportDestinationExists', '保存位置已有同名文件。请更换文件名或确认覆盖后再试。');
        break;
      case 'KNOWLEDGE_EXPORT_ACCESS_DENIED':
        message = stage === 'read_files'
          ? text('knowledge.errors.exportSourceUnreadable', '无法读取知识库文件。请检查源目录及文件的访问权限后再试。')
          : text('knowledge.errors.exportAccessDenied', '无法写入所选保存位置。请选择有写入权限的文件夹后再试。');
        break;
      case 'KNOWLEDGE_EXPORT_STORAGE_FULL':
        message = text('knowledge.errors.exportStorageFull', '保存位置的可用空间不足。请释放空间或选择其他位置后再试。');
        break;
      case 'KNOWLEDGE_EXPORT_WRITE_FAILED':
        message = stage === 'read_files'
          ? text('knowledge.errors.exportSourceUnreadable', '无法读取知识库文件。请检查源目录及文件的访问权限后再试。')
          : text('knowledge.errors.exportFailed', '导出文件未能保存。请重新选择 ZIP 文件的保存位置后再试。');
        break;
      case 'PROVIDER_UNAVAILABLE':
        message = text('knowledge.errors.providerUnavailable', '没有可用的模型服务，请检查模型配置后再试。');
        break;
      default:
        if (error.status === 401 || error.authExpired) {
          message = text('knowledge.errors.authExpired', '登录状态已失效，请重新登录后再试。');
        } else if (error.status === 403) {
          message = text('knowledge.errors.forbidden', '当前操作未获授权，请检查访问权限后再试。');
        } else if (error.status === 404) {
          message = text('knowledge.errors.notFound', '知识库或文件已不存在，请刷新页面后再试。');
        } else if (error.status === 409) {
          message = text('knowledge.errors.conflict', '内容已发生变化，请刷新后再试。');
        } else if (error.status === 400 || error.status === 422) {
          message = text('knowledge.errors.invalidInput', '操作未完成，请检查填写的内容或所选文件后再试。');
        } else if (error.status === 429) {
          message = text('knowledge.errors.rateLimited', '操作过于频繁，请稍后再试。');
        }
    }
  }

  return {
    type: 'knowledge-error',
    context,
    title,
    message,
    technicalDetails: technicalDetails(error),
    handled: isHandledAuthExpiredHttpError(error),
  };
}

/** For small action toasts; detailed failure surfaces use the presentation. */
export function knowledgeErrorText(error: unknown, context?: KnowledgeErrorContext, t?: TFunction): string {
  return normalizeKnowledgeError(error, context, t).message;
}
