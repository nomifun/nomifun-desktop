/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Tabs } from '@arco-design/web-react';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import React, { useState, useEffect } from 'react';
import { useSearchParams } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import LocalAgents from '@/renderer/pages/settings/AgentSettings/LocalAgents';
import RemoteAgents from '@/renderer/pages/settings/AgentSettings/RemoteAgents';
import NomiScrollArea from '@/renderer/components/base/NomiScrollArea';
import { useSettingsViewMode } from '../settingsViewContext';
import AgentRuntimeSettingsContent from './AgentRuntimeSettingsContent';

const AgentModalContent: React.FC = () => {
  const { t } = useTranslation();
  const [, agentMessageContext] = useArcoMessage({ maxCount: 10 });
  const viewMode = useSettingsViewMode();
  const isPageMode = viewMode === 'page';
  const [searchParams, setSearchParams] = useSearchParams();
  const [activeTab, setActiveTab] = useState<string>('local');

  useEffect(() => {
    const tabParam = searchParams.get('tab');
    if (tabParam === 'remote' || tabParam === 'local' || tabParam === 'runtime') {
      setActiveTab(tabParam);
    }
  }, [searchParams]);

  const handleTabChange = (key: string) => {
    setActiveTab(key);
    const next = new URLSearchParams(searchParams);
    next.set('tab', key);
    setSearchParams(next, { replace: true });
  };

  return (
    <div className='flex flex-col h-full w-full'>
      {agentMessageContext}

      <Tabs
        activeTab={activeTab}
        onChange={handleTabChange}
        type='line'
        className='flex flex-col flex-1 min-h-0 [&>.arco-tabs-content]:pt-0'
      >
        <Tabs.TabPane key='local' title={t('settings.agentManagement.localAgents')}>
          <NomiScrollArea className='flex-1 min-h-0 pb-16px scrollbar-hide' disableOverflow={isPageMode}>
            <LocalAgents />
          </NomiScrollArea>
        </Tabs.TabPane>
        <Tabs.TabPane key='remote' title={t('settings.agentManagement.remoteAgents')}>
          <NomiScrollArea className='flex-1 min-h-0 pb-16px scrollbar-hide' disableOverflow={isPageMode}>
            <RemoteAgents />
          </NomiScrollArea>
        </Tabs.TabPane>
        <Tabs.TabPane key='runtime' title={t('settings.executionEngineHub.runtimeTab')}>
          <AgentRuntimeSettingsContent />
        </Tabs.TabPane>
      </Tabs>
    </div>
  );
};

export default AgentModalContent;
