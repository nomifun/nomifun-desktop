/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IBrowserLane } from '@/common/browser/browserTypes';
import type { BrowserConversationGroup } from './browserInventoryModel';

const errorMessage = (error: unknown): string =>
  error instanceof Error ? error.message : String(error);

interface BrowserCloseFailure {
  laneId?: string;
  code?: string;
  message: string;
}

interface BrowserCloseOutcome {
  partial: boolean;
  failures: BrowserCloseFailure[];
}

const asRecord = (value: unknown): Record<string, unknown> | null =>
  value && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;

const firstString = (
  value: Record<string, unknown>,
  ...keys: string[]
): string | undefined => {
  for (const key of keys) {
    const candidate = value[key];
    if (typeof candidate === 'string' && candidate.trim()) return candidate.trim();
  }
  return undefined;
};

const firstNumber = (
  value: Record<string, unknown>,
  ...keys: string[]
): number | undefined => {
  for (const key of keys) {
    const candidate = value[key];
    if (typeof candidate === 'number' && Number.isFinite(candidate)) return candidate;
  }
  return undefined;
};

const closeResultPayload = (result: unknown): Record<string, unknown> | null => {
  const root = asRecord(result);
  if (!root) return null;
  return (
    asRecord(root.data) ??
    asRecord(root.result) ??
    asRecord(root.outcome) ??
    root
  );
};

const normalizeCloseFailure = (
  value: unknown,
  fallbackLaneId?: string
): BrowserCloseFailure | null => {
  if (typeof value === 'string' && value.trim()) {
    return { laneId: fallbackLaneId, message: value.trim() };
  }
  const record = asRecord(value);
  if (!record) return null;
  const laneId =
    firstString(record, 'lane_id', 'laneId', 'id') ?? fallbackLaneId;
  const code = firstString(record, 'code', 'error_code', 'reason_code');
  const message =
    firstString(record, 'message', 'error', 'error_message', 'reason') ??
    code ??
    'Browser lane close failed.';
  return { laneId, code, message };
};

const browserCloseOutcome = (result: unknown): BrowserCloseOutcome => {
  const payload = closeResultPayload(result);
  if (!payload) return { partial: false, failures: [] };

  const failures: BrowserCloseFailure[] = [];
  const rawFailures =
    payload.failures ??
    payload.failed_lanes ??
    payload.failed ??
    payload.errors;
  if (Array.isArray(rawFailures)) {
    for (const failure of rawFailures) {
      const normalized = normalizeCloseFailure(failure);
      if (normalized) failures.push(normalized);
    }
  } else {
    const failureMap = asRecord(rawFailures);
    if (failureMap) {
      for (const [laneId, failure] of Object.entries(failureMap)) {
        const normalized = normalizeCloseFailure(failure, laneId);
        if (normalized) failures.push(normalized);
      }
    }
  }

  const failedCount =
    firstNumber(payload, 'failed_count', 'failure_count', 'failed') ??
    failures.length;
  const partialFlag =
    payload.partial === true ||
    payload.partially_closed === true ||
    payload.completed_with_failures === true ||
    firstString(payload, 'status')?.toLowerCase() === 'partial';
  return {
    partial: partialFlag || failedCount > 0 || failures.length > 0,
    failures,
  };
};

export const browserClosePartialFailureMessage = (
  result: unknown
): string | null => {
  const outcome = browserCloseOutcome(result);
  if (!outcome.partial) return null;
  if (outcome.failures.length === 0) {
    return 'Some browser lanes could not be closed. The inventory was refreshed with the latest state.';
  }
  const details = outcome.failures
    .map((failure) => {
      const label = failure.laneId ? `Lane ${failure.laneId}` : 'Browser lane';
      const code = failure.code ? ` (${failure.code})` : '';
      return `${label}${code}: ${failure.message}`;
    })
    .join('; ');
  return `Some browser lanes could not be closed: ${details}`;
};

export interface BrowserInstallationWideCloseCopy {
  button: string;
  title: string;
  warning: string;
  action: string;
  success: string;
}

export const browserInstallationWideCloseCopy = (
  language?: string
): BrowserInstallationWideCloseCopy => {
  if (language?.toLowerCase().startsWith('zh')) {
    return {
      button: '全局关闭所有浏览器',
      title: '关闭整个安装中的所有浏览器通道？',
      warning: '这是整个安装范围的全局操作，会影响此 NomiFun 安装中的所有用户。',
      action: '全局关闭所有浏览器',
      success: '已全局关闭整个安装中的所有浏览器通道。',
    };
  }
  return {
    button: 'Close all globally',
    title: 'Close every browser lane across this installation?',
    warning:
      "This is an installation-wide global action affecting every user's browser lanes.",
    action: 'Close all globally',
    success: 'All browser lanes across this installation were closed.',
  };
};

export interface BrowserConfirmationRequest {
  title: string;
  content: string;
  okText: string;
  cancelText: string;
  onOk: () => Promise<void>;
}

export type BrowserConfirm = (request: BrowserConfirmationRequest) => void;

interface BrowserCloseFeedback {
  refresh: () => Promise<void>;
  notifySuccess: (message: string) => void;
  notifyError: (message: string) => void;
  successMessage: string;
}

interface BrowserLaneCloseDependencies extends BrowserCloseFeedback {
  invoke: (request: { lane_id: string }) => Promise<unknown>;
  setBusyLaneId: (laneId: string | null) => void;
}

interface BrowserConversationCloseDependencies extends BrowserCloseFeedback {
  invoke: (request: { conversation_id: string }) => Promise<unknown>;
  setBusyConversationId: (conversationId: string | null) => void;
}

interface BrowserCloseAllDependencies extends BrowserCloseFeedback {
  invoke: () => Promise<unknown>;
  setClosingAll: (closing: boolean) => void;
}

export const browserLaneHasActiveWork = (lane: IBrowserLane): boolean =>
  Boolean(lane.active_operation || (lane.active_operation_count ?? 0) > 0);

export const runBrowserLaneClose = async (
  lane: IBrowserLane,
  dependencies: BrowserLaneCloseDependencies
): Promise<void> => {
  dependencies.setBusyLaneId(lane.lane_id);
  try {
    const result = await dependencies.invoke({ lane_id: lane.lane_id });
    const partialFailure = browserClosePartialFailureMessage(result);
    if (partialFailure) {
      dependencies.notifyError(partialFailure);
    } else {
      dependencies.notifySuccess(dependencies.successMessage);
    }
  } catch (error) {
    dependencies.notifyError(errorMessage(error));
  } finally {
    try {
      await dependencies.refresh();
    } catch (error) {
      dependencies.notifyError(errorMessage(error));
    }
    dependencies.setBusyLaneId(null);
  }
};

export const runBrowserConversationClose = async (
  conversationId: string,
  dependencies: BrowserConversationCloseDependencies
): Promise<void> => {
  dependencies.setBusyConversationId(conversationId);
  try {
    const result = await dependencies.invoke({ conversation_id: conversationId });
    const partialFailure = browserClosePartialFailureMessage(result);
    if (partialFailure) {
      dependencies.notifyError(partialFailure);
    } else {
      dependencies.notifySuccess(dependencies.successMessage);
    }
  } catch (error) {
    dependencies.notifyError(errorMessage(error));
  } finally {
    try {
      await dependencies.refresh();
    } catch (error) {
      dependencies.notifyError(errorMessage(error));
    }
    dependencies.setBusyConversationId(null);
  }
};

export const runBrowserCloseAll = async (
  dependencies: BrowserCloseAllDependencies
): Promise<void> => {
  dependencies.setClosingAll(true);
  try {
    const result = await dependencies.invoke();
    const partialFailure = browserClosePartialFailureMessage(result);
    if (partialFailure) {
      dependencies.notifyError(partialFailure);
    } else {
      dependencies.notifySuccess(dependencies.successMessage);
    }
  } catch (error) {
    dependencies.notifyError(errorMessage(error));
  } finally {
    try {
      await dependencies.refresh();
    } catch (error) {
      dependencies.notifyError(errorMessage(error));
    }
    dependencies.setClosingAll(false);
  }
};

export const requestBrowserLaneClose = (
  lane: IBrowserLane,
  close: (lane: IBrowserLane) => Promise<void>,
  confirm: BrowserConfirm,
  copy: Omit<BrowserConfirmationRequest, 'onOk'>
): Promise<void> | undefined => {
  if (!browserLaneHasActiveWork(lane)) return close(lane);
  confirm({ ...copy, onOk: () => close(lane) });
  return undefined;
};

export const requestBrowserConversationClose = (
  group: BrowserConversationGroup,
  close: (conversationId: string) => Promise<void>,
  confirm: BrowserConfirm,
  copy: Omit<BrowserConfirmationRequest, 'onOk'>
): void => {
  if (!group.conversationId) return;
  const conversationId = group.conversationId;
  confirm({ ...copy, onOk: () => close(conversationId) });
};

export const requestBrowserCloseAll = (
  close: () => Promise<void>,
  confirm: BrowserConfirm,
  copy: Omit<BrowserConfirmationRequest, 'onOk'>
): void => {
  confirm({ ...copy, onOk: close });
};
