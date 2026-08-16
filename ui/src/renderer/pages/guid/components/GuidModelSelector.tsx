/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IProvider, TProviderWithModel } from '@/common/config/storage';
import { compositeKey } from '@/common/utils/compositeKey';
import { iconColors } from '@/renderer/styles/colors';
import { getModelDisplayLabel } from '@/renderer/utils/model/agentLogo';
import { Button, Dropdown, Menu } from '@arco-design/web-react';
import { Brain, Down, Plus } from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { useProvidersQuery } from '@/renderer/hooks/agent/useModelProviderList';
import { useModelsForTask } from '@/renderer/hooks/agent/useModelsForTask';
import { useModelSelectorProviderLabel } from '@/renderer/hooks/agent/useModelSelectorProviderLabel';
import { exactChatHealthDotColor } from './guidModelHealth';

type GuidModelSelectorProps = {
  /** True when the selected engine picks from configured model providers. */
  isProviderModelMode: boolean;
  modelList: IProvider[];
  current_model: TProviderWithModel | undefined;
  setCurrentModel: (model: TProviderWithModel) => Promise<void>;
};

const GuidModelSelector: React.FC<GuidModelSelectorProps> = ({
  isProviderModelMode,
  modelList,
  current_model,
  setCurrentModel,
}) => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const defaultModelLabel = t('common.defaultModel');
  const providerLabel = useModelSelectorProviderLabel();

  // Provider rows carry task-scoped health for exact capability lookups.
  const { data: modelConfig } = useProvidersQuery();

  // Unified Chat catalog from exact nested capabilities. modelList only
  // constrains which providers the caller permits.
  const { groups: chatGroups } = useModelsForTask('chat');

  // 过滤掉被禁用的 provider，且仅保留调用方允许的供应商
  const enabledGroups = React.useMemo(() => {
    const allowedIds = new Set(modelList.filter((p) => p.enabled !== false).map((p) => p.id));
    return chatGroups.filter((group) => allowedIds.has(group.provider.id));
  }, [chatGroups, modelList]);

  const providerSelectedLabel = React.useMemo(() => {
    if (!current_model?.use_model) return '';
    return current_model.use_model;
  }, [current_model?.use_model]);

  const providerButtonLabel = React.useMemo(() => {
    return getModelDisplayLabel({
      selected_value: current_model?.use_model,
      selectedLabel: providerSelectedLabel,
      defaultModelLabel,
      fallbackLabel: defaultModelLabel,
    });
  }, [current_model?.use_model, defaultModelLabel, providerSelectedLabel]);

  if (isProviderModelMode) {
    const hasModels = enabledGroups.length > 0;

    // Per-model health dot color.
    const healthDotColor = (providerId: IProvider['id'], modelName: string): string | null => {
      return exactChatHealthDotColor(modelConfig, providerId, modelName);
    };

    // The droplist is the bare <Menu> (no wrapper box, no forced min-width), so
    // Arco's native popup styling keeps it as smooth as the agent dropdown.
    return (
      <Dropdown
        trigger='click'
        droplist={
          <Menu selectedKeys={current_model ? [current_model.id + current_model.use_model] : []}>
            {!hasModels
              ? [
                  <Menu.Item
                    key='no-models'
                    className='px-12px py-12px text-t-secondary text-14px text-center flex justify-center items-center'
                    disabled
                  >
                    {t('settings.noAvailableModels')}
                  </Menu.Item>,
                  <Menu.Item key='add-model' className='text-12px text-t-secondary' onClick={() => navigate('/models?section=models')}>
                    <Plus theme='outline' size='12' />
                    {t('settings.addModel')}
                  </Menu.Item>,
                ]
              : [
                  ...enabledGroups.map(({ provider, models }) => {
                    return (
                      <Menu.ItemGroup title={providerLabel(provider)} key={provider.id}>
                        {models.map((modelName) => {
                          const dot = healthDotColor(provider.id, modelName);
                          return (
                            <Menu.Item
                              key={compositeKey(provider.id, modelName)}
                              className={
                                current_model?.id === provider.id && current_model?.use_model === modelName
                                  ? '!bg-2'
                                  : ''
                              }
                              onClick={() => {
                                setCurrentModel({ ...provider, use_model: modelName }).catch((error) => {
                                  console.error('Failed to set current model:', error);
                                });
                              }}
                            >
                              <div className='flex items-center gap-8px w-full'>
                                {dot && <div className={`w-6px h-6px rounded-full shrink-0 ${dot}`} />}
                                <span>{modelName}</span>
                              </div>
                            </Menu.Item>
                          );
                        })}
                      </Menu.ItemGroup>
                    );
                  }),
                  <Menu.Item key='add-model' className='text-12px text-t-secondary' onClick={() => navigate('/models?section=models')}>
                    <Plus theme='outline' size='12' />
                    {t('settings.addModel')}
                  </Menu.Item>,
                ]}
          </Menu>
        }
      >
        <Button
          className={'sendbox-model-btn guid-config-btn'}
          shape='round'
          size='small'
          data-testid='guid-model-selector'
          aria-label={providerButtonLabel}
        >
          <span className='flex items-center gap-6px min-w-0'>
            <Brain theme='outline' size='14' fill={iconColors.secondary} className='shrink-0' />
            <span className='sendbox-responsive-label truncate'>{providerButtonLabel}</span>
            <Down
              theme='outline'
              size='12'
              fill={iconColors.secondary}
              className='sendbox-responsive-chevron shrink-0'
            />
          </span>
        </Button>
      </Dropdown>
    );
  }

  // Fallback: no model switching
  return (
    <Button
      className={'sendbox-model-btn guid-config-btn'}
      shape='round'
      size='small'
      style={{ cursor: 'default' }}
      aria-label={defaultModelLabel}
    >
      <span className='flex items-center gap-6px min-w-0'>
        <Brain theme='outline' size='14' fill={iconColors.secondary} className='shrink-0' />
        <span className='sendbox-responsive-label'>{defaultModelLabel}</span>
      </span>
    </Button>
  );
};

export default GuidModelSelector;
