/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useLayoutContext } from '@/renderer/hooks/context/LayoutContext';
import type { ExecutableAgentPreset } from '../types';
import { Plus, Robot } from '@icon-park/react';
import { Tooltip } from '@arco-design/web-react';
import React from 'react';
import { useNavigate } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import styles from '../index.module.css';

type AgentPillBarProps = {
  presets: ExecutableAgentPreset[];
  selectedPresetId: string;
  onSelectPreset: (presetId: string) => void;
  suppressSelectionAnimation?: boolean;
};

const AgentPillBar: React.FC<AgentPillBarProps> = ({
  presets,
  selectedPresetId,
  onSelectPreset,
  suppressSelectionAnimation = false,
}) => {
  const layout = useLayoutContext();
  const isMobile = layout?.isMobile ?? false;
  const navigate = useNavigate();
  const { t } = useTranslation();

  return (
    <div className='w-full flex justify-center'>
      <div
        className='flex items-center justify-center'
        style={{
          marginBottom: 20,
          padding: '6px',
          borderRadius: '30px',
          backgroundColor: 'var(--color-guid-agent-bar, var(--aou-2))',
          transition: 'background-color 0.35s ease',
          width: isMobile ? 'calc(100% + 28px)' : 'fit-content',
          maxWidth: isMobile ? 'none' : '100%',
          marginLeft: isMobile ? -14 : 0,
          marginRight: isMobile ? -14 : 0,
          overflow: 'visible',
          gap: isMobile ? 6 : 4,
          flexWrap: 'wrap',
          color: 'var(--text-primary)',
        }}
      >
        {presets.map((preset) => {
          const presetId = preset.preset_id;
          const isSelected = selectedPresetId === presetId;

          return (
            <button
              type='button'
              key={presetId}
              data-testid={`agent-pill-${presetId}`}
              data-agent-pill='true'
              data-agent-preset-id={presetId}
              data-agent-selected={isSelected ? 'true' : 'false'}
              aria-pressed={isSelected}
              title={preset.display_name}
              className={`relative flex max-w-180px items-center overflow-hidden whitespace-nowrap border-0 px-10px py-7px rd-20px cursor-pointer ${isSelected ? `opacity-100 ${styles.agentItemSelected}` : 'opacity-70 hover:opacity-100'}`}
              style={{
                color: 'var(--text-primary)',
                background: isSelected ? undefined : 'transparent',
                transition: 'opacity 0.2s ease, background-color 0.2s ease',
                ...(isMobile || suppressSelectionAnimation
                  ? { animation: 'none' }
                  : undefined),
              }}
              onClick={() => onSelectPreset(presetId)}
            >
              <Robot
                theme='outline'
                size={18}
                fill='currentColor'
                style={{ flexShrink: 0 }}
              />
              <span
                className={`ml-5px min-w-0 truncate text-14px ${isSelected ? 'font-semibold' : 'font-medium'}`}
              >
                {preset.display_name}
              </span>
            </button>
          );
        })}
        <Tooltip
          content={t('agentSettings.navigation.railTitle', {
            defaultValue: 'Agent Workbench',
          })}
        >
          <button
            type='button'
            aria-label={t('agentSettings.navigation.railTitle', {
              defaultValue: 'Agent Workbench',
            })}
            data-testid='agent-workbench-add'
            className='flex items-center justify-center cursor-pointer p-4px opacity-60 hover:opacity-100 self-center'
            style={{
              transition: 'opacity 0.2s ease',
              flexShrink: 0,
              marginTop: 4,
              border: 0,
              color: 'inherit',
              background: 'transparent',
            }}
            onClick={() => navigate('/agent')}
          >
            <Plus
              theme='outline'
              size={20}
              fill='currentColor'
              style={{ flexShrink: 0 }}
            />
          </button>
        </Tooltip>
      </div>
    </div>
  );
};

export default AgentPillBar;
