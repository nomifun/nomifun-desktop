/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { AgentMetadata } from '@/renderer/utils/model/agentTypes';
import { useAgents } from '@/renderer/hooks/agent/useAgents';
import { Button, Typography } from '@arco-design/web-react';
import { IconRefresh } from '@arco-design/web-react/icon';
import React, { useCallback, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import AgentCard from './AgentCard';
import { getAgentKey } from '@/renderer/pages/guid/hooks/agentSelectionUtils';

/**
 * 卡片网格按「内容容器实际宽度」自动定列，而非视口断点 —— 模型管理内容面板
 * 被一次 rail + 二级 ContentSider 占去宽度，视口宽 ≠ 面板可用宽，用 md:/lg:/xl:
 * 视口断点会在窄面板下给出过多列数把卡片挤到裁剪。auto-fill 让列数随容器缩放。
 * Card grids auto-fit columns to the actual container width (not viewport
 * breakpoints): the model-hub pane is narrower than the viewport, so viewport
 * md:/lg:/xl: over-columns and clips cards on a narrow pane.
 */
const CARD_GRID_COLS = 'repeat(auto-fill, minmax(min(168px, 100%), 1fr))';

const LocalAgents: React.FC = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();

  const { agents: detectedAgents, refreshCustomAgents } = useAgents();
  const [refreshingDetection, setRefreshingDetection] = useState(false);

  const handleRefreshDetection = useCallback(async () => {
    setRefreshingDetection(true);
    try {
      await refreshCustomAgents();
    } catch (err) {
      console.error('refresh local agent detection failed:', err);
    } finally {
      setRefreshingDetection(false);
    }
  }, [refreshCustomAgents]);

  // Nomi first among detected agents
  const nomiAgent = detectedAgents?.find((a) => a.agent_type === 'nomi' || a.backend === 'nomi');
  const otherDetected = detectedAgents?.filter((a) => a.agent_type !== 'nomi' && a.backend !== 'nomi') ?? [];

  const goToChatWithAgent = useCallback(
    (agent: AgentMetadata) => {
      navigate('/guid', { state: { selectedAgentKey: getAgentKey(agent) } });
    },
    [navigate]
  );

  return (
    <div className='flex flex-col gap-8px py-16px'>
      <div className='flex flex-wrap items-center gap-x-6px gap-y-2px px-16px text-12px text-t-secondary'>
        <span>{t('settings.agentManagement.localAgentsDescription')} </span>
        <Button
          type='text'
          size='mini'
          icon={<IconRefresh />}
          loading={refreshingDetection}
          data-testid='btn-refresh-local-agents'
          className='!h-auto !p-0 !align-baseline !text-12px !font-normal !text-primary-6 hover:!text-primary-7 hover:!underline underline-offset-2'
          onClick={() => void handleRefreshDetection()}
        >
          {t('settings.agentManagement.refreshDetection')}
        </Button>
      </div>

      {/* Installed Agents section */}
      <div className='px-16px mt-8px'>
        <Typography.Text className='text-12px font-medium text-t-secondary mb-4px block'>
          {t('settings.agentManagement.installed')}
        </Typography.Text>
      </div>
      <div className='grid gap-10px px-16px' style={{ gridTemplateColumns: CARD_GRID_COLS }}>
        {nomiAgent && (
          <AgentCard
            agent={nomiAgent}
            onGoToChat={() => goToChatWithAgent(nomiAgent)}
          />
        )}
        {otherDetected.map((agent) => (
          <AgentCard
            key={agent.backend || agent.agent_type}
            agent={agent}
            onGoToChat={() => goToChatWithAgent(agent)}
          />
        ))}
      </div>
      {(!detectedAgents || detectedAgents.length === 0) && (
        <Typography.Text type='secondary' className='block px-16px py-16px text-center text-12px'>
          {t('settings.agentManagement.localAgentsEmpty')}
        </Typography.Text>
      )}
    </div>
  );
};

export default LocalAgents;
