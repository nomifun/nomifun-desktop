import type React from 'react';
import { useState, useCallback } from 'react';
import { Message } from '@arco-design/web-react';
import type { TFunction } from 'i18next';
import { useTranslation } from 'react-i18next';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import { mcpService } from '@/common/adapter/ipcBridge';
import { buildMcpConnectionTestRequest } from '@/common/adapter/mcpRequest';
import type { IMcpServer } from '@/common/config/storage';
import { getMcpConfigurationFields, supportsMcpOAuthLogin } from './mcpAuthConfig';

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
      if (details.phase === 'package_bootstrap') {
        return t('settings.mcpErrorPackageBootstrapTimeout', {
          command: details.command || 'command',
          seconds: details.timeout_seconds ?? 120,
          defaultValue: fallback,
        });
      }
      if (details.endpoint_scope === 'local') {
        return t('settings.mcpErrorLocalServiceTimeout', {
          host: details.host || 'localhost',
          port: details.port || '',
          seconds: details.timeout_seconds ?? 30,
          defaultValue: fallback,
        });
      }
      return t('settings.mcpErrorTimeout', {
        seconds: details.timeout_seconds ?? 30,
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
  setMcpServers: React.Dispatch<React.SetStateAction<IMcpServer[]>>,
  onAuthRequired?: (server: IMcpServer) => void,
  onAuthResolved?: (server: IMcpServer) => void
) => {
  const { t } = useTranslation();
  const [testingServers, setTestingServers] = useState<Record<string, boolean>>({});

  type TestOptions = {
    notify?: boolean;
  };

  // 连接测试函数
  const handleTestMcpConnection = useCallback(
    async (server: IMcpServer, options?: TestOptions) => {
      const notify = options?.notify ?? true;
      const configurationFields = getMcpConfigurationFields(server.transport);
      if (configurationFields.length > 0) {
        if (notify) {
          Message.warning({
            content: t('settings.mcpConfigurationRequired', {
              name: server.name,
              fields: configurationFields.join(', '),
              defaultValue: `${server.name}: Complete these configuration fields before testing: ${configurationFields.join(', ')}`,
            }),
            duration: 5000,
          });
        }
        return;
      }

      setTestingServers((prev) => ({ ...prev, [server.mcp_server_id]: true }));

      // 更新服务器状态 - 使用统一的保存函数，避免竞态条件
      const updateServerStatus = async (
        last_test_status: IMcpServer['last_test_status'],
        additionalData?: Partial<IMcpServer>
      ) => {
        setMcpServers((prevServers) =>
          prevServers.map((s) =>
            s.mcp_server_id === server.mcp_server_id
              ? {
                  ...s,
                  last_test_status,
                  updated_at: Date.now(),
                  ...additionalData,
                }
              : s
          )
        );
      };

      await updateServerStatus('testing');

      try {
        const result = await mcpService.testMcpConnection.invoke(buildMcpConnectionTestRequest(server));
        const needsAuth = result.needsAuth ?? result.needs_auth;
        const authMethod = result.authMethod ?? result.auth_method;

        // 检查是否需要认证
        if (needsAuth) {
          await updateServerStatus('disconnected');
          const canUseOAuth = authMethod === 'oauth' && supportsMcpOAuthLogin(server.transport);
          if (notify) {
            Message.warning({
              content: canUseOAuth
                ? `${server.name}: ${t('settings.mcpAuthRequired') || 'Authentication required'}`
                : t('settings.mcpHeaderAuthRequired', {
                    name: server.name,
                    defaultValue: `${server.name}: The server requires credentials. Add the required HTTP authorization header in its MCP JSON.`,
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
          // Record the latest successful availability test in local UI state.
          await updateServerStatus('connected', {
            tools: result.tools?.map((tool) => ({
              name: tool.name,
              description: tool.description,
              ...(tool.input_schema ? { input_schema: tool.input_schema } : {}),
              ...(tool._meta ? { _meta: tool._meta } : {}),
            })),
            last_connected: Date.now(),
          });
          if (notify) {
            Message.success({
              content: `${server.name}: ${t('settings.mcpTestConnectionSuccess')}`,
              duration: 3000,
            });
          }

          // 连接测试成功，不执行额外操作
        } else {
          // Record the latest failed availability test in local UI state.
          await updateServerStatus('error');
          const errorMsg = truncateErrorMessage(formatMcpErrorMessage(t, result));
          if (notify) {
            Message.error({
              content: t('settings.mcpTestConnectionFailedWithHint', {
                name: server.name,
                error: errorMsg,
                defaultValue: `${server.name}: ${errorMsg}`,
              }),
              duration: 8000,
            });
          }
        }
      } catch (error) {
        // Record the latest failed availability test in local UI state.
        await updateServerStatus('error');
        const errorMsg = truncateErrorMessage(formatThrownMcpErrorMessage(t, error));
        if (notify) {
          Message.error({
            content: t('settings.mcpTestConnectionFailedWithHint', {
              name: server.name,
              error: errorMsg,
              defaultValue: `${server.name}: ${errorMsg}`,
            }),
            duration: 8000,
          });
        }
      } finally {
        setTestingServers((prev) => ({
          ...prev,
          [server.mcp_server_id]: false,
        }));
      }
    },
    [setMcpServers, t, onAuthRequired, onAuthResolved]
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
