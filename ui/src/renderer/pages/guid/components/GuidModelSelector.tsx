/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IProvider, TProviderWithModel } from '@/common/config/storage';
import { compositeKey } from '@/common/utils/compositeKey';
import { modelHealthOf } from '@/common/utils/providerModels';
import { iconColors } from '@/renderer/styles/colors';
import { getModelDisplayLabel } from '@/renderer/utils/model/agentLogo';
import type { AcpModelInfo } from '../types';
import { Button, Dropdown, Menu } from '@arco-design/web-react';
import { Brain, Down, Plus } from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { useProvidersQuery } from '@/renderer/hooks/agent/useModelProviderList';
import { useModelsForTask } from '@/renderer/hooks/agent/useModelsForTask';
import { useModelSelectorProviderLabel } from '@/renderer/hooks/agent/useModelSelectorProviderLabel';

type GuidModelSelectorProps = {
  // Gemini model state
  isGeminiMode: boolean;
  modelList: IProvider[];
  current_model: TProviderWithModel | undefined;
  setCurrentModel: (model: TProviderWithModel) => Promise<void>;

  // ACP model state
  currentAcpCachedModelInfo: AcpModelInfo | null;
  selectedAcpModel: string | null;
  setSelectedAcpModel: React.Dispatch<React.SetStateAction<string | null>>;
};

const GuidModelSelector: React.FC<GuidModelSelectorProps> = ({
  isGeminiMode,
  modelList,
  current_model,
  setCurrentModel,
  currentAcpCachedModelInfo,
  selectedAcpModel,
  setSelectedAcpModel,
}) => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const defaultModelLabel = t('common.defaultModel');
  const providerLabel = useModelSelectorProviderLabel();

  // 获取模型配置数据（包含健康状态）
  const { data: modelConfig } = useProvidersQuery();

  // 统一 chat catalog（后端 resolve，无名称启发式）。modelList 仅约束「允许哪些
  // 供应商」（如 nomi 模式排除 Google Auth）；模型清单一律来自 catalog 分组。
  const { groups: chatGroups } = useModelsForTask('chat');

  // 过滤掉被禁用的 provider，且仅保留调用方允许的供应商
  const enabledGroups = React.useMemo(() => {
    const allowedIds = new Set(modelList.filter((p) => p.enabled !== false).map((p) => p.id));
    return chatGroups.filter((group) => allowedIds.has(group.provider.id));
  }, [chatGroups, modelList]);

  const geminiSelectedLabel = React.useMemo(() => {
    if (!current_model?.use_model) return '';
    return current_model.use_model;
  }, [current_model?.use_model]);

  const geminiButtonLabel = React.useMemo(() => {
    return getModelDisplayLabel({
      selected_value: current_model?.use_model,
      selectedLabel: geminiSelectedLabel,
      defaultModelLabel,
      fallbackLabel: defaultModelLabel,
    });
  }, [current_model?.use_model, defaultModelLabel, geminiSelectedLabel]);

  const acpSelectedLabel = React.useMemo(() => {
    return (
      currentAcpCachedModelInfo?.available_models?.find((m) => m.id === selectedAcpModel)?.label ||
      currentAcpCachedModelInfo?.current_model_label ||
      currentAcpCachedModelInfo?.current_model_id ||
      ''
    );
  }, [
    currentAcpCachedModelInfo?.available_models,
    currentAcpCachedModelInfo?.current_model_id,
    currentAcpCachedModelInfo?.current_model_label,
    selectedAcpModel,
  ]);

  const acpButtonLabel = React.useMemo(() => {
    return getModelDisplayLabel({
      selected_value: selectedAcpModel || currentAcpCachedModelInfo?.current_model_id,
      selectedLabel: acpSelectedLabel,
      defaultModelLabel,
      fallbackLabel: defaultModelLabel,
    });
  }, [acpSelectedLabel, currentAcpCachedModelInfo?.current_model_id, defaultModelLabel, selectedAcpModel]);

  if (isGeminiMode) {
    const hasModels = enabledGroups.length > 0;

    // Per-model health dot color.
    const healthDotColor = (providerId: string, modelName: string): string | null => {
      const matchedProvider = modelConfig?.find((p) => p.id === providerId);
      const healthStatus = modelHealthOf(matchedProvider, modelName)?.status || 'unknown';
      if (healthStatus === 'unknown') return null;
      return healthStatus === 'healthy' ? 'bg-green-500' : healthStatus === 'unhealthy' ? 'bg-red-500' : 'bg-gray-400';
    };

    // Mirror the ACP selector exactly: the droplist is the bare <Menu> (no wrapper
    // box, no forced min-width), so Arco's native popup styling keeps it as smooth
    // as the ACP agent dropdown.
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
          aria-label={geminiButtonLabel}
        >
          <span className='flex items-center gap-6px min-w-0'>
            <Brain theme='outline' size='14' fill={iconColors.secondary} className='shrink-0' />
            <span className='sendbox-responsive-label truncate'>{geminiButtonLabel}</span>
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

  // ACP cached model selector
  if (currentAcpCachedModelInfo && currentAcpCachedModelInfo.available_models?.length > 0) {
    if (currentAcpCachedModelInfo.available_models.length > 0) {
      return (
        <Dropdown
          trigger='click'
          droplist={
            <Menu selectedKeys={selectedAcpModel ? [selectedAcpModel] : []}>
              {currentAcpCachedModelInfo.available_models.map((model) => {
                // 获取模型健康状态
                const providerConfig = modelConfig?.find((p) => p.platform?.includes(''));
                const healthStatus = modelHealthOf(providerConfig, model.id)?.status || 'unknown';
                const healthColor =
                  healthStatus === 'healthy'
                    ? 'bg-green-500'
                    : healthStatus === 'unhealthy'
                      ? 'bg-red-500'
                      : 'bg-gray-400';

                return (
                  <Menu.Item
                    key={model.id}
                    className={model.id === selectedAcpModel ? '!bg-2' : ''}
                    onClick={() => setSelectedAcpModel(model.id)}
                  >
                    <div className='flex items-center gap-8px w-full'>
                      {healthStatus !== 'unknown' && (
                        <div className={`w-6px h-6px rounded-full shrink-0 ${healthColor}`} />
                      )}
                      <span>{model.label}</span>
                    </div>
                  </Menu.Item>
                );
              })}
            </Menu>
          }
        >
          <Button
            className={'sendbox-model-btn guid-config-btn'}
            shape='round'
            size='small'
            aria-label={acpButtonLabel}
          >
            <span className='flex items-center gap-6px min-w-0'>
              <Brain theme='outline' size='14' fill={iconColors.secondary} className='shrink-0' />
              <span className='sendbox-responsive-label'>{acpButtonLabel}</span>
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

    return (
      <Button
        className={'sendbox-model-btn guid-config-btn'}
        shape='round'
        size='small'
        style={{ cursor: 'default' }}
        aria-label={acpButtonLabel}
      >
        <span className='flex items-center gap-6px min-w-0'>
          <Brain theme='outline' size='14' fill={iconColors.secondary} className='shrink-0' />
          <span className='sendbox-responsive-label'>{acpButtonLabel}</span>
        </span>
      </Button>
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
