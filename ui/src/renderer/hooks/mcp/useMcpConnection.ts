import { useState, useCallback, useEffect, useRef } from 'react';
import { Message } from '@arco-design/web-react';
import type { TFunction } from 'i18next';
import { useTranslation } from 'react-i18next';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import { mcpService } from '@/common/adapter/ipcBridge';
import { withRuntimePreparation, formatRuntimePreparation, type RuntimeToolSnapshot } from '@/common/adapter/runtimeTools';
import { uuidv7 } from '@/common/utils/uuidv7';
import { buildMcpConnectionTestRequest } from '@/common/adapter/mcpRequest';
import type { IMcpServer } from '@/common/config/storage';
import { getMcpConfigurationFields, supportsMcpOAuthLogin } from './mcpAuthConfig';
import { notifyMcpCatalogChanged } from './catalog';
import { resolveMcpDisplayName } from '@/renderer/services/mcp/mcpMarket';

/**
 * 截断过长的错误消息，保持可读性
 * Truncate long error messages to keep them readable
 */
const truncateErrorMessage = (message: string, maxLength: number = 260): string => {
  if (message.length <= maxLength) {
    return message;
  }
  return message.substring(0, maxLength) + '...';
};

type McpErrorPayload = {
  error?: string;
  code?: string;
  details?: unknown;
};

type McpErrorDetails = {
  command?: string;
  runtime?: string;
  timeout_seconds?: number;
  status?: number;
  method?: string;
  rpc_code?: number;
  owner_code?: string;
  failure_kind?: string;
  phase?: string;
  endpoint_scope?: 'local' | 'remote';
  host?: string;
  port?: number;
};

const getMcpErrorDetails = (details: unknown): McpErrorDetails => {
  if (!details || typeof details !== 'object' || Array.isArray(details)) {
    return {};
  }
  return details as McpErrorDetails;
};

const formatMcpErrorMessage = (t: TFunction, payload: McpErrorPayload): string => {
  const fallback = payload.error || t('settings.mcpError');
  const details = getMcpErrorDetails(payload.details);
  if (payload.error && ['MCP_COMMAND_NOT_FOUND', 'MCP_COMMAND_PERMISSION_DENIED', 'MCP_COMMAND_START_FAILED'].includes(payload.code ?? '')) {
    return payload.error;
  }

  switch (payload.code) {
    case 'MCP_COMMAND_NOT_FOUND':
      switch (details.runtime) {
        case 'node':
          return t('settings.mcpErrorNodeCommandNotFound', {
            command: details.command || 'npx',
            defaultValue: fallback,
          });
        case 'bun':
          return t('settings.mcpErrorBunCommandNotFound', {
            command: details.command || 'bunx',
            defaultValue: fallback,
          });
        case 'uv':
          return t('settings.mcpErrorUvCommandNotFound', {
            command: details.command || 'uvx',
            defaultValue: fallback,
          });
        case 'python':
          return t('settings.mcpErrorPythonCommandNotFound', {
            command: details.command || 'python',
            defaultValue: fallback,
          });
        case 'deno':
          return t('settings.mcpErrorDenoCommandNotFound', {
            command: details.command || 'deno',
            defaultValue: fallback,
          });
      }
      return t('settings.mcpErrorCommandNotFound', {
        command: details.command || 'command',
        defaultValue: fallback,
      });
    case 'MCP_COMMAND_PERMISSION_DENIED':
      return t('settings.mcpErrorCommandPermissionDenied', {
        command: details.command || 'command',
        defaultValue: fallback,
      });
    case 'MCP_COMMAND_START_FAILED':
      if (details.failure_kind === 'package_not_found') {
        return t('settings.mcpErrorPackageNotFound', {
          command: details.command || 'command',
          defaultValue: fallback,
        });
      }
      if (details.failure_kind === 'package_download_failed') {
        return t('settings.mcpErrorPackageDownloadFailed', {
          command: details.command || 'command',
          defaultValue: fallback,
        });
      }
      if (details.failure_kind === 'dependency_missing') {
        return t('settings.mcpErrorProcessDependencyMissing', {
          command: details.command || 'command',
          defaultValue: fallback,
        });
      }
      if (details.failure_kind === 'configuration_required') {
        return t('settings.mcpErrorProcessConfigurationRequired', {
          command: details.command || 'command',
          defaultValue: fallback,
        });
      }
      if (details.failure_kind === 'process_exited') {
        return t('settings.mcpErrorProcessExited', {
          command: details.command || 'command',
          defaultValue: fallback,
        });
      }
      return t('settings.mcpErrorCommandStartFailed', {
        command: details.command || 'command',
        defaultValue: fallback,
      });
    case 'MCP_TIMEOUT':
      // Only the owner knows which bounded setup phase expired. A missing
      // measurement must not become an invented 30/120-second deadline.
      if (typeof details.timeout_seconds !== 'number' || !Number.isFinite(details.timeout_seconds) || details.timeout_seconds < 0) {
        return fallback;
      }
      if (details.endpoint_scope === 'local') {
        return t('settings.mcpErrorLocalServiceTimeout', {
          host: details.host || 'localhost',
          port: details.port || '',
          seconds: details.timeout_seconds,
          defaultValue: fallback,
        });
      }
      return t('settings.mcpErrorTimeout', {
        seconds: details.timeout_seconds,
        defaultValue: fallback,
      });
    case 'MCP_CONNECTION_FAILED':
      if (details.endpoint_scope === 'local') {
        return t('settings.mcpErrorLocalServiceUnavailable', {
          host: details.host || 'localhost',
          port: details.port || '',
          defaultValue: fallback,
        });
      }
      return t('settings.mcpErrorConnectionFailed', { defaultValue: fallback });
    case 'MCP_HTTP_ERROR':
      if (typeof details.status !== 'number') {
        return fallback;
      }
      return t('settings.mcpErrorHttp', {
        status: details.status,
        defaultValue: fallback,
      });
    case 'MCP_RPC_ERROR':
      return t('settings.mcpErrorRpc', {
        method: details.method || 'request',
        defaultValue: fallback,
      });
    case 'MCP_PROTOCOL_ERROR':
      if (details.owner_code === 'MCP_PROTOCOL_VERSION_MISMATCH') {
        return t('settings.mcpErrorProtocolVersion', {
          defaultValue: fallback,
        });
      }
      return t('settings.mcpErrorProtocol', { defaultValue: fallback });
    default:
      return fallback;
  }
};

const formatThrownMcpErrorMessage = (t: TFunction, error: unknown): string => {
  if (isBackendHttpError(error)) {
    return formatMcpErrorMessage(t, {
      error: error.backendMessage,
      code: error.code,
      details: error.details,
    });
  }
  return error instanceof Error ? error.message : t('settings.mcpError');
};

/**
 * MCP连接测试管理Hook
 * 处理MCP服务器的连接测试和状态更新
 */
export const useMcpConnection = (
  onAuthRequired?: (server: IMcpServer) => void,
  onAuthResolved?: (server: IMcpServer) => void
) => {
  const { t } = useTranslation();
  const [testingServers, setTestingServers] = useState<Record<string, boolean>>({});
  const active = useRef(true);
  const pendingTests = useRef(new Map<string, number>());
  useEffect(() => {
    active.current = true;
    return () => { active.current = false; };
  }, []);

  type TestOptions = {
    notify?: boolean;
  };

  // 连接测试函数
  const handleTestMcpConnection = useCallback(
    async (server: IMcpServer, options?: TestOptions) => {
      const notify = options?.notify ?? true;
      const displayName = resolveMcpDisplayName(server);
      const configurationFields = getMcpConfigurationFields(server.transport);
      if (configurationFields.length > 0) {
        if (notify) {
          Message.warning({
            content: t('settings.mcpConfigurationRequired', {
              name: displayName,
              fields: configurationFields.join(', '),
              defaultValue: `${displayName}: Complete these configuration fields before testing: ${configurationFields.join(', ')}`,
            }),
            duration: 5000,
          });
        }
        return;
      }

      pendingTests.current.set(server.mcp_server_id, (pendingTests.current.get(server.mcp_server_id) ?? 0) + 1);
      if (active.current) setTestingServers((prev) => ({ ...prev, [server.mcp_server_id]: true }));

      let closePreparation: (() => void) | undefined;
      let probeEntered = false;
      const preparationId = `mcp-runtime-${uuidv7()}`;
      try {
        if (!active.current) return;
        probeEntered = true;
        const result = await withRuntimePreparation(
          () => mcpService.testMcpConnection.invoke(buildMcpConnectionTestRequest(server)), {
            isCurrent: () => active.current,
            onProgress: (snapshot: RuntimeToolSnapshot) => {
              if (notify && active.current) closePreparation = Message.loading({ id: preparationId,
                content: `${displayName}: ${formatRuntimePreparation(snapshot, t)}`, duration: 0 });
            },
          });
        if (!active.current) return;
        const needsAuth = result.needsAuth ?? result.needs_auth;
        const authMethod = result.authMethod ?? result.auth_method;

        // 检查是否需要认证
        if (needsAuth) {
          const canUseOAuth = authMethod === 'oauth' && supportsMcpOAuthLogin(server.transport);
          if (notify) {
            Message.warning({
              content: canUseOAuth
                ? `${displayName}: ${t('settings.mcpAuthRequired') || 'Authentication required'}`
                : t('settings.mcpHeaderAuthRequired', {
                    name: displayName,
                    defaultValue: `${displayName}: The server requires credentials. Add the required HTTP authorization header in its MCP JSON.`,
                  }),
              duration: 3000,
            });
          }

          // Only a Bearer/OAuth challenge can enter the OAuth flow. Basic or
          // unspecified challenges must be configured explicitly as headers.
          if (canUseOAuth && onAuthRequired) {
            onAuthRequired(server);
          }
          return;
        }

        if (supportsMcpOAuthLogin(server.transport) && onAuthResolved) {
          onAuthResolved(server);
        }

        if (result.success) {
          if (notify) {
            Message.success({
              content: `${displayName}: ${t('settings.mcpTestConnectionSuccess')}`,
              duration: 3000,
            });
          }

        } else {
          const errorMsg = truncateErrorMessage(formatMcpErrorMessage(t, result));
          if (notify) {
            Message.error({
              content: t('settings.mcpTestConnectionFailedWithHint', {
                name: displayName,
                error: errorMsg,
                defaultValue: `${displayName}: ${errorMsg}`,
              }),
              duration: 8000,
            });
          }
        }
      } catch (error) {
        if (isBackendHttpError(error) && getMcpErrorDetails(error.details).phase === 'runtime_resolution') {
          probeEntered = false;
        }
        if (!active.current) return;
        const errorMsg = truncateErrorMessage(formatThrownMcpErrorMessage(t, error));
        if (notify) {
          Message.error({
            content: t('settings.mcpTestConnectionFailedWithHint', {
              name: displayName,
              error: errorMsg,
              defaultValue: `${displayName}: ${errorMsg}`,
            }),
            duration: 8000,
          });
        }
      } finally {
        closePreparation?.();
        const remaining = (pendingTests.current.get(server.mcp_server_id) ?? 1) - 1;
        if (remaining > 0) pendingTests.current.set(server.mcp_server_id, remaining);
        else pendingTests.current.delete(server.mcp_server_id);
        if (active.current) {
          setTestingServers((prev) => ({ ...prev, [server.mcp_server_id]: remaining > 0 }));
        }
        // The backend persists success, failure, and authentication outcomes
        // before the response settles. Reload that row, including after errors
        // such as a stale probe, instead of inventing a local catalog result.
        if (probeEntered) notifyMcpCatalogChanged();
      }
    },
    [t, onAuthRequired, onAuthResolved]
  );

  const handleTestMcpConnections = useCallback(
    async (servers: IMcpServer[], options?: TestOptions & { concurrency?: number }) => {
      const concurrency = Math.max(1, options?.concurrency ?? 4);
      let nextIndex = 0;

      const worker = async () => {
        while (true) {
          const currentIndex = nextIndex;
          nextIndex += 1;
          const server = servers[currentIndex];
          if (!server) {
            return;
          }
          await handleTestMcpConnection(server, options);
        }
      };

      await Promise.all(Array.from({ length: Math.min(concurrency, servers.length) }, () => worker()));
    },
    [handleTestMcpConnection]
  );

  return {
    testingServers,
    handleTestMcpConnection,
    handleTestMcpConnections,
  };
};
