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

export interface BrowserClosePartialFailureCopy {
  withoutDetails: string;
  withDetails: (details: string) => string;
}

const DEFAULT_CLOSE_PARTIAL_FAILURE_COPY: BrowserClosePartialFailureCopy = {
  withoutDetails: 'Some browser lanes could not be closed.',
  withDetails: (details) => `Some browser lanes could not be closed: ${details}`,
};

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
  result: unknown,
  copy: BrowserClosePartialFailureCopy = DEFAULT_CLOSE_PARTIAL_FAILURE_COPY
): string | null => {
  const outcome = browserCloseOutcome(result);
  if (!outcome.partial) return null;
  if (outcome.failures.length === 0) return copy.withoutDetails;
  const details = outcome.failures
    .map((failure) => {
      const label = failure.laneId ? `Lane ${failure.laneId}` : 'Browser lane';
      const code = failure.code ? ` (${failure.code})` : '';
      return `${label}${code}: ${failure.message}`;
    })
    .join('; ');
  return copy.withDetails(details);
};

/**
 * A successful HTTP response is not enough to claim that a close happened.
 * The backend's idempotent no-op is explicit (`already_closed: true`); an
 * explicit zero-close response without that marker is treated as unconfirmed.
 */
export const browserCloseResultIsUnconfirmed = (result: unknown): boolean => {
  const payload = closeResultPayload(result);
  if (!payload) return true;
  const closed = firstNumber(payload, 'closed');
  return (
    !browserCloseOutcome(result).partial &&
    payload.already_closed !== true &&
    !(closed != null && closed > 0)
  );
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
  formatPartialFailure?: (result: unknown) => string | null;
  formatRefreshFailure?: (message: string) => string;
  unconfirmedMessage?: string;
}

interface BrowserLaneCloseDependencies extends BrowserCloseFeedback {
  invoke: (request: { lane_id: string }) => Promise<unknown>;
  setBusyLaneId: (laneId: string | null) => void;
}

interface BrowserLaneForegroundDependencies {
  invoke: (request: { lane_id: string }) => Promise<unknown>;
  refresh: () => Promise<void>;
  setForegroundingLaneId: (laneId: string | null) => void;
  notifySuccess: (message: string) => void;
  notifyError: (message: string) => void;
  successMessage: string;
  formatRefreshFailure?: (message: string) => string;
  unconfirmedMessage?: string;
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

/** Foregrounding is deliberately narrower than lifecycle management. */
export const canForegroundBrowserLane = (lane: IBrowserLane): boolean =>
  lane.lifecycle_state === 'running' && lane.identity?.mode === 'primary';

export const runBrowserLaneForeground = async (
  lane: IBrowserLane,
  dependencies: BrowserLaneForegroundDependencies
): Promise<void> => {
  if (!canForegroundBrowserLane(lane)) return;

  dependencies.setForegroundingLaneId(lane.lane_id);
  try {
    let operationError: string | null = null;
    let unconfirmed = false;
    try {
      const result = await dependencies.invoke({ lane_id: lane.lane_id });
      const payload = closeResultPayload(result);
      unconfirmed = payload?.foregrounded !== true;
    } catch (error) {
      operationError = errorMessage(error);
    }

    let refreshError: string | null = null;
    try {
      await dependencies.refresh();
    } catch (error) {
      const message = errorMessage(error);
      refreshError = dependencies.formatRefreshFailure?.(message) ?? message;
    }

    if (operationError) {
      dependencies.notifyError(
        refreshError ? `${operationError}; ${refreshError}` : operationError
      );
    } else if (unconfirmed) {
      const message =
        dependencies.unconfirmedMessage ??
        'The browser foreground request was not confirmed. Review its status and try again.';
      dependencies.notifyError(refreshError ? `${message}; ${refreshError}` : message);
    } else if (refreshError) {
      dependencies.notifyError(refreshError);
    } else {
      dependencies.notifySuccess(dependencies.successMessage);
    }
  } finally {
    dependencies.setForegroundingLaneId(null);
  }
};

const reportCloseAttempt = async (
  result: unknown,
  operationError: string | null,
  dependencies: BrowserCloseFeedback
): Promise<void> => {
  let refreshError: string | null = null;
  try {
    await dependencies.refresh();
  } catch (error) {
    const message = errorMessage(error);
    refreshError = dependencies.formatRefreshFailure?.(message) ?? message;
  }

  if (operationError) {
    dependencies.notifyError(
      refreshError ? `${operationError}; ${refreshError}` : operationError
    );
    return;
  }

  const partialFailure =
    dependencies.formatPartialFailure?.(result) ?? browserClosePartialFailureMessage(result);
  if (partialFailure) {
    dependencies.notifyError(refreshError ? `${partialFailure}; ${refreshError}` : partialFailure);
    return;
  }
  if (browserCloseResultIsUnconfirmed(result)) {
    const message =
      dependencies.unconfirmedMessage ??
      'The browser manager did not confirm the close. Review the inventory and try again.';
    dependencies.notifyError(
      refreshError ? `${message}; ${refreshError}` : message
    );
    return;
  }
  if (refreshError) {
    dependencies.notifyError(refreshError);
    return;
  }
  dependencies.notifySuccess(dependencies.successMessage);
};

export const runBrowserLaneClose = async (
  lane: IBrowserLane,
  dependencies: BrowserLaneCloseDependencies
): Promise<void> => {
  dependencies.setBusyLaneId(lane.lane_id);
  let result: unknown;
  let operationError: string | null = null;
  try {
    result = await dependencies.invoke({ lane_id: lane.lane_id });
  } catch (error) {
    operationError = errorMessage(error);
  } finally {
    try {
      await reportCloseAttempt(result, operationError, dependencies);
    } catch (error) {
      dependencies.notifyError(errorMessage(error));
    } finally {
      dependencies.setBusyLaneId(null);
    }
  }
};

export const runBrowserConversationClose = async (
  conversationId: string,
  dependencies: BrowserConversationCloseDependencies
): Promise<void> => {
  dependencies.setBusyConversationId(conversationId);
  let result: unknown;
  let operationError: string | null = null;
  try {
    result = await dependencies.invoke({ conversation_id: conversationId });
  } catch (error) {
    operationError = errorMessage(error);
  } finally {
    try {
      await reportCloseAttempt(result, operationError, dependencies);
    } catch (error) {
      dependencies.notifyError(errorMessage(error));
    } finally {
      dependencies.setBusyConversationId(null);
    }
  }
};

export const runBrowserCloseAll = async (
  dependencies: BrowserCloseAllDependencies
): Promise<void> => {
  dependencies.setClosingAll(true);
  let result: unknown;
  let operationError: string | null = null;
  try {
    result = await dependencies.invoke();
  } catch (error) {
    operationError = errorMessage(error);
  } finally {
    try {
      await reportCloseAttempt(result, operationError, dependencies);
    } catch (error) {
      dependencies.notifyError(errorMessage(error));
    } finally {
      dependencies.setClosingAll(false);
    }
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
