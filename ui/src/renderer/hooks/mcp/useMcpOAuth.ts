import { useCallback, useEffect, useRef, useState } from 'react';
import { mcpService } from '@/common/adapter/ipcBridge';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import type { IMcpServer } from '@/common/config/storage';
import type { McpServerId } from '@/common/types/ids';
import type { McpAuthDiagnostic, McpAuthStatus, McpOAuthAttempt, McpOAuthClientConfig } from '@/common/types/mcpAuth';
import { isDesktopShell, openExternalUrl } from '@/renderer/utils/platform';
import { supportsMcpOAuthLogin } from './mcpAuthConfig';
import { useTranslation } from 'react-i18next';

export interface McpOAuthStatus {
  auth?: McpAuthStatus;
  isChecking: boolean;
  error?: string;
  diagnostic?: McpAuthDiagnostic;
}

export interface McpOAuthResult {
  success: boolean;
  cancelled?: boolean;
  error?: string;
  diagnostic?: McpAuthDiagnostic;
}

interface LoginControl {
  cancelled: boolean;
  attemptId?: string;
  popup: Window | null;
  wake?: () => void;
}

const AUTH_ERROR_CODES = new Set([
  'INVALID_CONFIGURATION', 'RESOURCE_METADATA_UNAVAILABLE', 'METADATA_INVALID', 'ISSUER_MISMATCH', 'RESOURCE_MISMATCH',
  'PKCE_UNSUPPORTED', 'CLIENT_REGISTRATION_REQUIRED', 'CLIENT_REGISTRATION_REJECTED', 'CLIENT_AUTHENTICATION_UNSUPPORTED',
  'REDIRECT_UNAVAILABLE', 'USER_DENIED', 'CALLBACK_TIMEOUT', 'STATE_MISMATCH', 'CALLBACK_INVALID', 'TOKEN_EXCHANGE_REJECTED',
  'REAUTHORIZATION_REQUIRED', 'INSUFFICIENT_SCOPE', 'UPSTREAM_UNAVAILABLE', 'CONNECTION_CHANGED', 'CANCELLED', 'SECRET_STORAGE_FAILED', 'INTERNAL',
]);
const AUTH_STAGES = new Set(['discovery', 'client_registration', 'authorization', 'callback', 'token_exchange', 'refresh', 'storage', 'connection']);
const AUTH_ACTIONS = new Set(['none', 'retry', 'configure_client', 'reauthorize', 'grant_scope']);

/** Accept only the auth API's authored contract, never HTTP text or upstream JSON. */
const backendAuthDiagnostic = (error: unknown): McpAuthDiagnostic | undefined => {
  if (!isBackendHttpError(error) || !error.details || typeof error.details !== 'object' || Array.isArray(error.details)) return;
  const value = error.details as Record<string, unknown>;
  if (typeof value.code !== 'string' || !AUTH_ERROR_CODES.has(value.code) || value.code !== error.code
    || typeof value.stage !== 'string' || !AUTH_STAGES.has(value.stage)
    || typeof value.user_action !== 'string' || !AUTH_ACTIONS.has(value.user_action)
    || typeof value.retryable !== 'boolean'
    || typeof value.trace_id !== 'string' || !/^[a-zA-Z0-9][a-zA-Z0-9._:-]{0,127}$/.test(value.trace_id)
    || typeof value.message !== 'string' || !value.message.trim() || value.message.length > 2048
    || /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/.test(value.message)) return;
  return {
    code: value.code, stage: value.stage as McpAuthDiagnostic['stage'], retryable: value.retryable,
    user_action: value.user_action as McpAuthDiagnostic['user_action'], trace_id: value.trace_id, message: value.message,
    ...(value.details === undefined ? {} : { details: value.details }),
  };
};

const failedAuthOperation = (error: unknown, fallback: string): McpOAuthResult => {
  const diagnostic = backendAuthDiagnostic(error);
  return { success: false, error: diagnostic?.message ?? fallback, ...(diagnostic ? { diagnostic } : {}) };
};

export const isMcpOAuthAttemptPending = (attempt: McpOAuthAttempt): boolean =>
  attempt.status === 'waiting_for_user' || attempt.status === 'exchanging';

/** Never navigate the user's browser to an executable or credential-bearing URL. */
export const validatedMcpAuthorizationUrl = (value: string): string => {
  const url = new URL(value);
  const local = ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname);
  if ((url.protocol !== 'https:' && !(url.protocol === 'http:' && local)) || url.username || url.password) {
    throw new Error('The MCP authorization URL is invalid');
  }
  return url.toString();
};

/** Login attempts, server authentication, and connection probes have separate state. */
export const useMcpOAuth = ({ pollIntervalMs = 1000 }: { pollIntervalMs?: number } = {}) => {
  const { t } = useTranslation();
  const [oauthStatus, setOAuthStatus] = useState<Record<string, McpOAuthStatus>>({});
  const [attempts, setAttempts] = useState<Record<string, McpOAuthAttempt | undefined>>({});
  const [loggingIn, setLoggingIn] = useState<Record<string, boolean>>({});
  const controls = useRef(new Map<McpServerId, LoginControl>());
  const mounted = useRef(true);
  const statusVersions = useRef(new Map<McpServerId, number>());

  const recordFailure = useCallback((id: McpServerId, result: McpOAuthResult) => {
    // A status response started before this failure must not clear its details.
    statusVersions.current.set(id, (statusVersions.current.get(id) ?? 0) + 1);
    if (mounted.current) setOAuthStatus((previous) => ({ ...previous, [id]: {
      ...previous[id], isChecking: false, error: result.error, diagnostic: result.diagnostic,
    } }));
    return result;
  }, []);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      for (const [id, control] of controls.current) {
        control.cancelled = true;
        control.wake?.();
        control.popup?.close();
        if (control.attemptId) void mcpService.cancelMcpOAuth.invoke({ mcp_server_id: id, attempt_id: control.attemptId }).catch(() => {});
      }
    };
  }, []);

  const checkOAuthStatus = useCallback(async (server: IMcpServer) => {
    if (!supportsMcpOAuthLogin(server.transport)) return;
    const id = server.mcp_server_id;
    const version = (statusVersions.current.get(id) ?? 0) + 1;
    statusVersions.current.set(id, version);
    setOAuthStatus((previous) => ({ ...previous, [id]: { ...previous[id], isChecking: true } }));
    try {
      const auth = await mcpService.getMcpAuth.invoke({ mcp_server_id: id });
      if (mounted.current && statusVersions.current.get(id) === version) {
        setOAuthStatus((previous) => ({ ...previous, [id]: { auth, isChecking: false } }));
      }
    } catch (error) {
      if (mounted.current && statusVersions.current.get(id) === version) {
        const result = failedAuthOperation(error, t('settings.mcpOAuthStatusFailed'));
        setOAuthStatus((previous) => ({ ...previous, [id]: {
          ...previous[id], isChecking: false, error: result.error, diagnostic: result.diagnostic,
        } }));
      }
    }
  }, [t]);

  const login = useCallback(async (server: IMcpServer, client?: McpOAuthClientConfig): Promise<McpOAuthResult> => {
    const id = server.mcp_server_id;
    if (!supportsMcpOAuthLogin(server.transport)) return { success: false, error: t('settings.mcpOAuthConfiguredCredentials') };
    if (controls.current.has(id)) return { success: false, error: t('settings.mcpOAuthAlreadyRunning') };

    // Reserve a browser tab during the click gesture, before discovery awaits.
    // The desktop shell uses its local shell instead of a remote host browser.
    const control: LoginControl = { cancelled: false, popup: isDesktopShell() ? null : window.open('about:blank', '_blank') };
    if (control.popup) control.popup.opener = null;
    controls.current.set(id, control);
    setLoggingIn((previous) => ({ ...previous, [id]: true }));
    setAttempts((previous) => ({ ...previous, [id]: undefined }));
    try {
      let attempt = await mcpService.startMcpOAuth.invoke({ mcp_server_id: id, client });
      control.attemptId = attempt.attempt_id;
      if (control.cancelled) {
        await mcpService.cancelMcpOAuth.invoke({ mcp_server_id: id, attempt_id: attempt.attempt_id });
        return { success: false, cancelled: true };
      }
      if (mounted.current) setAttempts((previous) => ({ ...previous, [id]: attempt }));
      if (attempt.authorization_url && isMcpOAuthAttemptPending(attempt)) {
        const url = validatedMcpAuthorizationUrl(attempt.authorization_url);
        if (control.popup) {
          control.popup.location.replace(url);
          control.popup = null;
        } else if (isDesktopShell()) {
          await openExternalUrl(url);
        }
        // A blocked popup leaves a visible, user-clickable continuation link.
      }
      while (!control.cancelled && isMcpOAuthAttemptPending(attempt)) {
        await new Promise<void>((resolve) => {
          const timer = window.setTimeout(() => { control.wake = undefined; resolve(); }, pollIntervalMs);
          control.wake = () => { window.clearTimeout(timer); resolve(); };
        });
        if (control.cancelled) break;
        const next = await Promise.race([
          mcpService.getMcpOAuthAttempt.invoke({ mcp_server_id: id, attempt_id: attempt.attempt_id }),
          new Promise<undefined>((resolve) => { control.wake = () => resolve(undefined); }),
        ]);
        control.wake = undefined;
        if (control.cancelled || !next) break;
        attempt = next;
        if (mounted.current) setAttempts((previous) => ({ ...previous, [id]: attempt }));
      }
      if (control.cancelled || attempt.status === 'cancelled') return { success: false, cancelled: true };
      await checkOAuthStatus(server);
      return attempt.status === 'authorized'
        ? { success: true }
        : { success: false, error: attempt.diagnostic?.message ?? t('settings.mcpOAuthNotCompleted'), diagnostic: attempt.diagnostic };
    } catch (error) {
      if (control.cancelled) return { success: false, cancelled: true };
      // If tracking or browser launch fails, stop this exact attempt where
      // possible. A failed cancellation never claims the grant was removed.
      if (control.attemptId) {
        try { await mcpService.cancelMcpOAuth.invoke({ mcp_server_id: id, attempt_id: control.attemptId }); } catch { /* Backend expiry and explicit removal remain authoritative. */ }
      }
      await checkOAuthStatus(server);
      return recordFailure(id, failedAuthOperation(error, t('settings.mcpOAuthRequestFailed')));
    } finally {
      control.popup?.close();
      control.wake?.();
      if (controls.current.get(id) === control) controls.current.delete(id);
      if (mounted.current) setLoggingIn((previous) => ({ ...previous, [id]: false }));
    }
  }, [checkOAuthStatus, pollIntervalMs, recordFailure, t]);

  const cancelLogin = useCallback(async (server: IMcpServer): Promise<McpOAuthResult> => {
    const control = controls.current.get(server.mcp_server_id);
    if (!control) return { success: true };
    try {
      if (control.attemptId) await mcpService.cancelMcpOAuth.invoke({ mcp_server_id: server.mcp_server_id, attempt_id: control.attemptId });
      control.cancelled = true;
      control.wake?.();
      control.popup?.close();
      await checkOAuthStatus(server);
      return { success: true };
    } catch (error) {
      return recordFailure(server.mcp_server_id, failedAuthOperation(error, t('settings.mcpOAuthCancelFailed')));
    }
  }, [checkOAuthStatus, recordFailure, t]);

  const logout = useCallback(async (server: IMcpServer): Promise<McpOAuthResult> => {
    try {
      await mcpService.logoutMcpOAuth.invoke({ mcp_server_id: server.mcp_server_id });
      const control = controls.current.get(server.mcp_server_id);
      if (control) { control.cancelled = true; control.wake?.(); control.popup?.close(); }
      await checkOAuthStatus(server);
      return { success: true };
    } catch (error) {
      return recordFailure(server.mcp_server_id, failedAuthOperation(error, t('settings.mcpOAuthLogoutFailed')));
    }
  }, [checkOAuthStatus, recordFailure, t]);

  return { oauthStatus, attempts, loggingIn, checkOAuthStatus, login, cancelLogin, logout };
};
