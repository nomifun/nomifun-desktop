/**
 * McpMarketSettings — MCP market tab for the MCP hub page.
 *
 * SECURITY: market configs are remote, untrusted input. Every server resolved
 * from a market entry is imported DISABLED, the exact transport (command +
 * args + env keys, or URL + header keys) is shown to the user for review
 * before anything is persisted, and no connection test is ever triggered here
 * — testing an stdio server would execute its command on this machine.
 */
import { ipcBridge } from '@/common';
import type { ISkillMarketItem } from '@/common/adapter/ipcBridge';
import type { IMcpServer, IMcpServerTransport } from '@/common/config/storage';
import { Alert, Message, Modal, Tag } from '@arco-design/web-react';
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import MarketSettingsPanel from '@/renderer/pages/settings/MarketSettingsPanel';
import { MCP_MARKET_SOURCES } from '@/renderer/pages/settings/skill/skillMarket';
import { useMcpServerCRUD } from '@/renderer/hooks/mcp';
import { getMcpConfigurationFields } from '@/renderer/hooks/mcp/mcpAuthConfig';
import {
  toImportableMcpServersFromConfig,
  type ImportableMcpServer,
} from '@/renderer/pages/settings/ToolsSettings/mcpImportUtils';

type McpMarketSettingsProps = {
  saveMcpServers: (serversOrUpdater: IMcpServer[] | ((prev: IMcpServer[]) => IMcpServer[])) => Promise<void>;
  mcpServers: IMcpServer[];
  addedStateLoading?: boolean;
};

const MCP_MARKET_ORIGIN_KEY = '_nomifun_market';

export const isLocalMcpEndpoint = (transport: IMcpServerTransport): boolean => {
  if (transport.type === 'stdio') return false;
  try {
    const hostname = new URL(transport.url).hostname.toLowerCase();
    return (
      hostname === 'localhost' ||
      hostname.endsWith('.localhost') ||
      hostname === '::1' ||
      hostname === '[::1]' ||
      /^127(?:\.\d{1,3}){3}$/.test(hostname)
    );
  } catch {
    return false;
  }
};

export const attachMcpMarketOrigin = (originalJson: string, marketItemId: string): string => {
  let original: Record<string, unknown> = {};
  try {
    const parsed = JSON.parse(originalJson) as unknown;
    if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
      original = parsed as Record<string, unknown>;
    }
  } catch {
    // Importable market configs normally contain valid JSON. Keep provenance
    // even if an upstream formatter produced malformed original_json.
  }
  return JSON.stringify(
    {
      ...original,
      [MCP_MARKET_ORIGIN_KEY]: { version: 1, item_id: marketItemId },
    },
    null,
    2
  );
};

export const getMcpMarketOrigin = (server: Pick<IMcpServer, 'original_json'>): string | null => {
  try {
    const parsed = JSON.parse(server.original_json) as Record<string, unknown>;
    const origin = parsed[MCP_MARKET_ORIGIN_KEY];
    if (!origin || typeof origin !== 'object' || Array.isArray(origin)) return null;
    const itemId = (origin as Record<string, unknown>).item_id;
    return typeof itemId === 'string' && itemId.trim() ? itemId : null;
  } catch {
    return null;
  }
};

const normalizeMcpMarketName = (value: string): string =>
  value
    .trim()
    .toLocaleLowerCase()
    .replace(/[\s_]+/g, '-');

export const isMcpMarketItemInstalled = (
  item: Pick<ISkillMarketItem, 'id' | 'name'>,
  servers: readonly IMcpServer[]
): boolean => {
  return getMcpMarketItemServers(item, servers).length > 0;
};

export const getMcpMarketItemServers = (
  item: Pick<ISkillMarketItem, 'id' | 'name'>,
  servers: readonly IMcpServer[]
): IMcpServer[] => {
  const exact = servers.filter((server) => getMcpMarketOrigin(server) === item.id);
  if (exact.length > 0) return exact;

  const idSlug = item.id.split(':').slice(1).join(':').split('/').filter(Boolean).at(-1) ?? '';
  const legacyNames = new Set([item.name, idSlug].map(normalizeMcpMarketName).filter(Boolean));
  return servers.filter((server) => legacyNames.has(normalizeMcpMarketName(server.name)));
};

/** Read-only transport summary so the user can review exactly what would run or be contacted. */
const TransportDetails: React.FC<{ transport: IMcpServerTransport }> = ({ transport }) => {
  const { t } = useTranslation();

  if (transport.type === 'stdio') {
    const envKeys = Object.keys(transport.env ?? {});
    return (
      <div className='mt-8px space-y-4px text-12px leading-18px'>
        <div className='flex gap-6px'>
          <span className='flex-shrink-0 text-t-tertiary'>
            {t('settings.mcpMarket.confirmCommand', {
              defaultValue: 'Command',
            })}
            :
          </span>
          <code className='font-mono text-t-primary break-all'>{transport.command}</code>
        </div>
        {(transport.args?.length ?? 0) > 0 && (
          <div className='flex gap-6px'>
            <span className='flex-shrink-0 text-t-tertiary'>
              {t('settings.mcpMarket.confirmArgs', {
                defaultValue: 'Arguments',
              })}
              :
            </span>
            <code className='font-mono text-t-primary break-all'>{(transport.args ?? []).join(' ')}</code>
          </div>
        )}
        {envKeys.length > 0 && (
          <div className='flex gap-6px'>
            <span className='flex-shrink-0 text-t-tertiary'>
              {t('settings.mcpMarket.confirmEnvKeys', {
                defaultValue: 'Env variables',
              })}
              :
            </span>
            <code className='font-mono text-t-primary break-all'>{envKeys.join(', ')}</code>
          </div>
        )}
      </div>
    );
  }

  const headerKeys = Object.keys(transport.headers ?? {});
  return (
    <div className='mt-8px space-y-4px text-12px leading-18px'>
      <div className='flex gap-6px'>
        <span className='flex-shrink-0 text-t-tertiary'>
          {t('settings.mcpMarket.confirmUrl', { defaultValue: 'URL' })}:
        </span>
        <code className='font-mono text-t-primary break-all'>{transport.url}</code>
      </div>
      {headerKeys.length > 0 && (
        <div className='flex gap-6px'>
          <span className='flex-shrink-0 text-t-tertiary'>
            {t('settings.mcpMarket.confirmHeaderKeys', {
              defaultValue: 'Header keys',
            })}
            :
          </span>
          <code className='font-mono text-t-primary break-all'>{headerKeys.join(', ')}</code>
        </div>
      )}
    </div>
  );
};

const McpMarketSettings: React.FC<McpMarketSettingsProps> = ({
  saveMcpServers,
  mcpServers,
  addedStateLoading = false,
}) => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { handleBatchImportMcpServers, handleEditMcpServer } = useMcpServerCRUD(saveMcpServers);

  const [pendingServers, setPendingServers] = useState<ImportableMcpServer[] | null>(null);
  const [pendingMarketItem, setPendingMarketItem] = useState<ISkillMarketItem | null>(null);
  const [importing, setImporting] = useState(false);
  const mounted = useRef(false);
  const revision = useRef(0);
  const importInFlight = useRef(false);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      revision.current += 1;
    };
  }, []);

  const handleCancel = () => {
    // Dismissing review invalidates UI completion, not an already sent import.
    revision.current += 1;
    setPendingServers(null);
    setPendingMarketItem(null);
  };

  const handleAdd = useCallback(
    async (item: ISkillMarketItem) => {
      if (!mounted.current || importInFlight.current) return;
      const request = ++revision.current;
      setPendingServers(null);
      setPendingMarketItem(null);
      try {
        const resolved = await ipcBridge.fs.resolveSkillMarketMcpConfig.invoke({
          source: item.source,
          id: item.id,
          url: item.url,
        });
        if (!mounted.current || request !== revision.current) return;
        // Force every market server to import disabled — the user must review
        // the transport (especially stdio commands) before enabling anything.
        const servers = toImportableMcpServersFromConfig(resolved.config_json, false).map((server) => ({
          ...server,
          enabled: false,
          original_json: attachMcpMarketOrigin(server.original_json, item.id),
        }));
        if (servers.length === 0) {
          Message.error(
            t('settings.mcpMarket.configMissing', {
              defaultValue: 'No importable MCP config found.',
            })
          );
          return;
        }

        // Import proceeds only after the user confirms the reviewed transports.
        setPendingServers(servers);
        setPendingMarketItem(item);
      } catch (error) {
        if (!mounted.current || request !== revision.current) return;
        console.error('Failed to resolve MCP market config:', error);
        Message.error(
          t('settings.mcpMarket.addFailed', {
            defaultValue: 'Failed to add MCP server.',
          })
        );
      }
    },
    [t]
  );

  const handleConfirmImport = useCallback(async () => {
    if (!mounted.current || !pendingServers || !pendingMarketItem || importInFlight.current) return;
    importInFlight.current = true;
    const request = ++revision.current;
    setImporting(true);
    try {
      // Servers stay disabled; deliberately NO connection test — testing an
      // stdio server would spawn its command on this machine.
      const installed = getMcpMarketItemServers(pendingMarketItem, mcpServers);
      const hasExactProvenance = installed.some((server) => getMcpMarketOrigin(server) === pendingMarketItem.id);
      const unmatchedInstalled = [...installed];
      const updated: IMcpServer[] = [];
      const additions: ImportableMcpServer[] = [];

      for (const server of pendingServers) {
        let matchingIndex = unmatchedInstalled.findIndex(
          (current) => normalizeMcpMarketName(current.name) === normalizeMcpMarketName(server.name)
        );
        // Market authors occasionally rename the single server inside an
        // entry. Exact provenance still identifies the installed resource;
        // legacy name-only matches are paired only when the unmatched sets
        // are equal-sized. Preserve the local name because the backend
        // deliberately forbids rename-on-edit, and a repair must replace
        // config rather than create a duplicate.
        if (
          matchingIndex < 0 &&
          unmatchedInstalled.length > 0 &&
          (hasExactProvenance ||
            unmatchedInstalled.length === pendingServers.length - updated.length - additions.length)
        ) {
          matchingIndex = 0;
        }
        const matching = matchingIndex >= 0 ? unmatchedInstalled.splice(matchingIndex, 1)[0] : undefined;
        if (matching) {
          const result = await handleEditMcpServer(matching, {
            ...server,
            name: matching.name,
          });
          if (!result) return;
          updated.push(result);
        } else {
          additions.push(server);
        }
      }

      const imported = additions.length > 0 ? await handleBatchImportMcpServers(additions) : [];
      if (imported.length !== additions.length) return;
      if (!mounted.current || request !== revision.current) return;
      const changedCount = updated.length + imported.length;
      if (changedCount > 0) {
        const repaired = installed.length > 0;
        setPendingServers(null);
        setPendingMarketItem(null);
        Message.warning(
          repaired
            ? t('settings.mcpMarket.repairedDisabled', {
                count: changedCount,
                defaultValue:
                  'Updated {{count}} MCP server(s) in a disabled state. Complete required fields and review the new transport before testing.',
              })
            : t('settings.mcpMarket.importedDisabled', {
                count: changedCount,
                defaultValue:
                  'Imported {{count}} MCP server(s) in a disabled state. Review the command and config before enabling or testing.',
              })
        );
        navigate('/mcp');
      }
    } catch (error) {
      if (!mounted.current || request !== revision.current) return;
      console.error('Failed to import MCP market servers:', error);
      Message.error(
        t('settings.mcpMarket.addFailed', {
          defaultValue: 'Failed to add MCP server.',
        })
      );
    } finally {
      importInFlight.current = false;
      if (mounted.current) setImporting(false);
    }
  }, [handleBatchImportMcpServers, handleEditMcpServer, mcpServers, navigate, pendingMarketItem, pendingServers, t]);

  const hasStdioServer = (pendingServers ?? []).some((server) => server.transport.type === 'stdio');
  const hasLocalNetworkServer = (pendingServers ?? []).some((server) => isLocalMcpEndpoint(server.transport));
  const isAdded = useCallback((item: ISkillMarketItem) => isMcpMarketItemInstalled(item, mcpServers), [mcpServers]);
  const canRepair = useCallback(
    (item: ISkillMarketItem) =>
      getMcpMarketItemServers(item, mcpServers).some(
        (server) => server.last_test_status === 'error' || getMcpConfigurationFields(server.transport).length > 0
      ),
    [mcpServers]
  );
  const isRepair = pendingMarketItem ? getMcpMarketItemServers(pendingMarketItem, mcpServers).length > 0 : false;

  return (
    <>
      <MarketSettingsPanel
        title={t('settings.mcpMarket.title', { defaultValue: 'MCP Market' })}
        description={t('settings.mcpMarket.description', {
          defaultValue: 'Browse SkillHub MCP and MCP World popular servers, then import their MCP JSON directly.',
        })}
        sources={MCP_MARKET_SOURCES}
        cacheKey='nomifun.mcpMarket.rankings.v1'
        autoSyncKey='nomifun.mcpMarket.autoSynced.v1'
        defaultSource='mcpworld'
        searchPlaceholder={t('settings.mcpMarket.searchPlaceholder', {
          defaultValue: 'Search MCP servers...',
        })}
        emptyText={t('settings.mcpMarket.empty', {
          defaultValue: 'Refresh to load MCP market entries.',
        })}
        onAdd={handleAdd}
        isAdded={isAdded}
        canRunAddedAction={canRepair}
        addedActionLabel={t('settings.mcpMarket.repair', {
          defaultValue: 'Repair config',
        })}
        addedStateLoading={addedStateLoading || importing}
        testIdPrefix='mcp-market'
      />

      <Modal
        title={
          isRepair
            ? t('settings.mcpMarket.repairTitle', {
                defaultValue: 'Review replacement MCP configuration',
              })
            : t('settings.mcpMarket.confirmTitle', {
                defaultValue: 'Review MCP server before import',
              })
        }
        visible={pendingServers !== null}
        onCancel={handleCancel}
        onOk={() => void handleConfirmImport()}
        okText={
          isRepair
            ? t('settings.mcpMarket.repairOk', {
                defaultValue: 'Replace and disable',
              })
            : t('settings.mcpMarket.confirmOk', {
                defaultValue: 'Import disabled',
              })
        }
        cancelText={t('common.cancel', { defaultValue: 'Cancel' })}
        okButtonProps={{ loading: importing }}
        maskClosable={false}
      >
        <div className='space-y-12px'>
          <div className='text-13px text-t-secondary'>
            {isRepair
              ? t('settings.mcpMarket.repairIntro', {
                  defaultValue:
                    'This replaces the failed market configuration and disables the server. Review the new transport and restore credentials before testing.',
                })
              : t('settings.mcpMarket.confirmIntro', {
                  defaultValue:
                    'This configuration comes from an external market. Servers are imported disabled; review the details below before confirming.',
                })}
          </div>
          {hasStdioServer && (
            <Alert
              type='warning'
              showIcon
              content={t('settings.mcpMarket.confirmStdioWarning', {
                defaultValue:
                  'The market imports this launch configuration but does not install its command or runtime. Testing or enabling it runs the command on this machine; verify the prerequisite and only run commands you trust.',
              })}
            />
          )}
          {hasLocalNetworkServer && (
            <Alert
              type='warning'
              showIcon
              content={t('settings.mcpMarket.confirmLocalServiceWarning', {
                defaultValue:
                  'This localhost URL is a connection descriptor only. Nomi does not start or install the referenced program or Docker service; start it separately before testing.',
              })}
            />
          )}
          {(pendingServers ?? []).map((server) => (
            <div
              key={server.name}
              className='rounded-12px border border-solid border-[var(--color-border-2)] bg-[var(--color-fill-1)] p-12px'
            >
              <div className='flex items-center gap-8px min-w-0'>
                <span className='truncate text-14px font-medium text-t-primary'>{server.name}</span>
                <Tag size='small' bordered={false} className='!flex-shrink-0 !text-11px'>
                  {server.transport.type}
                </Tag>
              </div>
              {server.description && (
                <div className='mt-4px text-12px leading-18px text-t-secondary'>{server.description}</div>
              )}
              {server.market_needs_configuration && (
                <Alert
                  className='mt-8px'
                  type='warning'
                  showIcon
                  content={t('settings.mcpMarket.configurationRequired', {
                    fields: (server.market_configuration_fields ?? []).join(', '),
                    defaultValue: `This is a configuration template. Complete these fields after import: ${(server.market_configuration_fields ?? []).join(', ')}`,
                  })}
                />
              )}
              <TransportDetails transport={server.transport} />
            </div>
          ))}
        </div>
      </Modal>
    </>
  );
};

export default McpMarketSettings;
