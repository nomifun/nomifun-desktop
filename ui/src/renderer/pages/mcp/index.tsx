/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { Tabs } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import { useSearchParams } from 'react-router-dom';
import HubPageShell from '@/renderer/components/layout/HubPageShell';
import { ToolsModalContentWithState } from '@/renderer/components/settings/SettingsModal/contents/ToolsModalContent';
import { useMcpServers } from '@/renderer/hooks/mcp';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import McpMarketSettings from './McpMarketSettings';
import PluginSettingsPanel from './PluginSettingsPanel';

type McpTab = 'servers' | 'market' | 'plugins' | 'plugin-market';

const isMcpTab = (value: string | null): value is McpTab =>
  value === 'servers' || value === 'market' || value === 'plugins' || value === 'plugin-market';

const McpPage: React.FC = () => {
  const { t } = useTranslation();
  const [searchParams, setSearchParams] = useSearchParams();
  const [mcpMessage, mcpMessageContext] = useArcoMessage({ maxCount: 10 });
  const {
    mcpServers,
    extensionMcpServers,
    isMcpServersLoading,
    mcpServersLoadFailed,
    saveMcpServers,
    setMcpServers,
  } = useMcpServers();
  const tabParam = searchParams.get('tab');
  const activeTab: McpTab = isMcpTab(tabParam) ? tabParam : 'servers';

  const handleTabChange = (key: string) => {
    if (!isMcpTab(key)) return;
    const next = new URLSearchParams(searchParams);
    if (key === 'servers') next.delete('tab');
    else next.set('tab', key);
    setSearchParams(next, { replace: true });
  };

  return (
    <HubPageShell
      title={t('settings.mcpHub.title', { defaultValue: 'MCP' })}
      subtitle={t('settings.mcpHub.subtitle', {
        defaultValue: 'Register MCP servers, browse MCP markets, and manage plugins.',
      })}
      maxWidthClass='md:max-w-1200px'
    >
      <Tabs
        activeTab={activeTab}
        onChange={handleTabChange}
        type='line'
        lazyload
        className='flex flex-col flex-1 min-h-0 [&>.arco-tabs-content]:pt-0'
      >
        <Tabs.TabPane key='servers' title={t('settings.mcpPage.installedMcpTab', { defaultValue: 'Installed MCP' })}>
          <ToolsModalContentWithState
            mcpMessage={mcpMessage}
            mcpMessageContext={mcpMessageContext}
            mcpServers={mcpServers}
            extensionMcpServers={extensionMcpServers}
            saveMcpServers={saveMcpServers}
            setMcpServers={setMcpServers}
          />
        </Tabs.TabPane>
        <Tabs.TabPane key='market' title={t('settings.mcpPage.mcpMarketTab', { defaultValue: 'MCP Market' })}>
          <McpMarketSettings
            saveMcpServers={saveMcpServers}
            mcpServers={mcpServers}
            addedStateLoading={isMcpServersLoading || mcpServersLoadFailed}
          />
        </Tabs.TabPane>
        <Tabs.TabPane key='plugins' title={t('settings.mcpPage.installedPluginsTab', { defaultValue: 'Installed Plugins' })}>
          <PluginSettingsPanel section='installed' />
        </Tabs.TabPane>
        <Tabs.TabPane key='plugin-market' title={t('settings.mcpPage.pluginMarketTab', { defaultValue: 'Plugin Market' })}>
          <PluginSettingsPanel section='market' />
        </Tabs.TabPane>
      </Tabs>
    </HubPageShell>
  );
};

export default McpPage;
