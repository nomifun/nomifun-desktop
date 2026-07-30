import { ipcBridge } from '@/common';
import type { ISkillMarketItem } from '@/common/adapter/ipcBridge';
import type { IMcpServer } from '@/common/config/storage';
import { Message } from '@arco-design/web-react';
import React, { useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import MarketSettingsPanel from '@/renderer/pages/settings/MarketSettingsPanel';
import { MCP_MARKET_SOURCES } from '@/renderer/pages/settings/skill/skillMarket';
import { useMcpConnection, useMcpServerCRUD } from '@/renderer/hooks/mcp';
import { toImportableMcpServersFromConfig } from '@/renderer/pages/settings/ToolsSettings/mcpImportUtils';

type McpMarketSettingsProps = {
  setMcpServers: React.Dispatch<React.SetStateAction<IMcpServer[]>>;
  saveMcpServers: (serversOrUpdater: IMcpServer[] | ((prev: IMcpServer[]) => IMcpServer[])) => Promise<void>;
};

const McpMarketSettings: React.FC<McpMarketSettingsProps> = ({ setMcpServers, saveMcpServers }) => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { handleBatchImportMcpServers } = useMcpServerCRUD(saveMcpServers);
  const { handleTestMcpConnections } = useMcpConnection(setMcpServers);

  const handleAdd = useCallback(
    async (item: ISkillMarketItem) => {
      try {
        const resolved = await ipcBridge.fs.resolveSkillMarketMcpConfig.invoke({
          source: item.source,
          id: item.id,
          url: item.url,
        });
        const servers = toImportableMcpServersFromConfig(resolved.config_json, true);
        if (servers.length === 0) {
          Message.error(t('settings.mcpMarket.configMissing', { defaultValue: 'No importable MCP config found.' }));
          return;
        }

        const needsConfigNames = new Set(
          servers.filter((server) => server.market_needs_configuration).map((server) => server.name)
        );
        const imported = await handleBatchImportMcpServers(servers);
        if (imported && imported.length > 0) {
          const testableServers = imported.filter((server) => !needsConfigNames.has(server.name) && server.enabled);
          if (testableServers.length > 0) {
            await handleTestMcpConnections(testableServers, { concurrency: 4, notify: false });
          }

          const firstConfigServer = imported.find((server) => needsConfigNames.has(server.name));
          if (firstConfigServer) {
            Message.warning(
              t('settings.mcpMarket.addNeedsConfig', {
                name: firstConfigServer.name,
                defaultValue: `${firstConfigServer.name} added. Fill required API config before enabling it.`,
              })
            );
            navigate(`/mcp?editMcp=${encodeURIComponent(firstConfigServer.name)}`);
            return;
          }

          Message.success(
            t('settings.mcpMarket.addSuccess', {
              count: imported.length,
              defaultValue: `Added ${imported.length} MCP server(s).`,
            })
          );
        }
      } catch (error) {
        console.error('Failed to add MCP from market:', error);
        Message.error(t('settings.mcpMarket.addFailed', { defaultValue: 'Failed to add MCP server.' }));
      }
    },
    [handleBatchImportMcpServers, handleTestMcpConnections, navigate, t]
  );

  return (
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
    />
  );
};

export default McpMarketSettings;
