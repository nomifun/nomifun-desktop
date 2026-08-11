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
import React, { useCallback, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import MarketSettingsPanel from '@/renderer/pages/settings/MarketSettingsPanel';
import { MCP_MARKET_SOURCES } from '@/renderer/pages/settings/skill/skillMarket';
import { useMcpServerCRUD } from '@/renderer/hooks/mcp';
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
  value.trim().toLocaleLowerCase().replace(/[\s_]+/g, '-');

export const isMcpMarketItemInstalled = (
  item: Pick<ISkillMarketItem, 'id' | 'name'>,
  servers: readonly IMcpServer[]
): boolean => {
  if (servers.some((server) => getMcpMarketOrigin(server) === item.id)) return true;

  const idSlug = item.id.split(':').slice(1).join(':').split('/').filter(Boolean).at(-1) ?? '';
  const legacyNames = new Set([item.name, idSlug].map(normalizeMcpMarketName).filter(Boolean));
  return servers.some((server) => legacyNames.has(normalizeMcpMarketName(server.name)));
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
            {t('settings.mcpMarket.confirmCommand', { defaultValue: 'Command' })}:
          </span>
          <code className='font-mono text-t-primary break-all'>{transport.command}</code>
        </div>
        {(transport.args?.length ?? 0) > 0 && (
          <div className='flex gap-6px'>
            <span className='flex-shrink-0 text-t-tertiary'>
              {t('settings.mcpMarket.confirmArgs', { defaultValue: 'Arguments' })}:
            </span>
            <code className='font-mono text-t-primary break-all'>{(transport.args ?? []).join(' ')}</code>
          </div>
        )}
        {envKeys.length > 0 && (
          <div className='flex gap-6px'>
            <span className='flex-shrink-0 text-t-tertiary'>
              {t('settings.mcpMarket.confirmEnvKeys', { defaultValue: 'Env variables' })}:
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
            {t('settings.mcpMarket.confirmHeaderKeys', { defaultValue: 'Header keys' })}:
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
  const { handleBatchImportMcpServers } = useMcpServerCRUD(saveMcpServers);

  const [pendingServers, setPendingServers] = useState<ImportableMcpServer[] | null>(null);
  const [importing, setImporting] = useState(false);

  const handleAdd = useCallback(
    async (item: ISkillMarketItem) => {
      try {
        const resolved = await ipcBridge.fs.resolveSkillMarketMcpConfig.invoke({
          source: item.source,
          id: item.id,
          url: item.url,
        });
        // Force every market server to import disabled — the user must review
        // the transport (especially stdio commands) before enabling anything.
        const servers = toImportableMcpServersFromConfig(resolved.config_json, false).map((server) => ({
          ...server,
          enabled: false,
          original_json: attachMcpMarketOrigin(server.original_json, item.id),
        }));
        if (servers.length === 0) {
          Message.error(t('settings.mcpMarket.configMissing', { defaultValue: 'No importable MCP config found.' }));
          return;
        }

        // Import proceeds only after the user confirms the reviewed transports.
        setPendingServers(servers);
      } catch (error) {
        console.error('Failed to resolve MCP market config:', error);
        Message.error(t('settings.mcpMarket.addFailed', { defaultValue: 'Failed to add MCP server.' }));
      }
    },
    [t]
  );

  const handleConfirmImport = useCallback(async () => {
    if (!pendingServers || importing) return;
    setImporting(true);
    try {
      // Servers stay disabled; deliberately NO connection test — testing an
      // stdio server would spawn its command on this machine.
      const imported = await handleBatchImportMcpServers(pendingServers);
      if (imported && imported.length > 0) {
        setPendingServers(null);
        Message.warning(
          t('settings.mcpMarket.importedDisabled', {
            count: imported.length,
            defaultValue:
              'Imported {{count}} MCP server(s) in a disabled state. Review the command and config before enabling or testing.',
          })
        );
        navigate('/mcp');
      }
    } catch (error) {
      console.error('Failed to import MCP market servers:', error);
      Message.error(t('settings.mcpMarket.addFailed', { defaultValue: 'Failed to add MCP server.' }));
    } finally {
      setImporting(false);
    }
  }, [handleBatchImportMcpServers, importing, navigate, pendingServers, t]);

  const hasStdioServer = (pendingServers ?? []).some((server) => server.transport.type === 'stdio');
  const isAdded = useCallback(
    (item: ISkillMarketItem) => isMcpMarketItemInstalled(item, mcpServers),
    [mcpServers]
  );

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
        searchPlaceholder={t('settings.mcpMarket.searchPlaceholder', { defaultValue: 'Search MCP servers...' })}
        emptyText={t('settings.mcpMarket.empty', { defaultValue: 'Refresh to load MCP market entries.' })}
        onAdd={handleAdd}
        isAdded={isAdded}
        addedStateLoading={addedStateLoading}
        testIdPrefix='mcp-market'
      />

      <Modal
        title={t('settings.mcpMarket.confirmTitle', { defaultValue: 'Review MCP server before import' })}
        visible={pendingServers !== null}
        onCancel={() => setPendingServers(null)}
        onOk={() => void handleConfirmImport()}
        okText={t('settings.mcpMarket.confirmOk', { defaultValue: 'Import disabled' })}
        cancelText={t('common.cancel', { defaultValue: 'Cancel' })}
        okButtonProps={{ loading: importing }}
        maskClosable={false}
      >
        <div className='space-y-12px'>
          <div className='text-13px text-t-secondary'>
            {t('settings.mcpMarket.confirmIntro', {
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
                  'Stdio servers run a local command on your machine once enabled or tested. Only enable commands you trust.',
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
              <TransportDetails transport={server.transport} />
            </div>
          ))}
        </div>
      </Modal>
    </>
  );
};

export default McpMarketSettings;
