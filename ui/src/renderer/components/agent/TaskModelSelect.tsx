/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { Button, Dropdown, Menu } from '@arco-design/web-react';
import { Brain, Down, Plus } from '@icon-park/react';
import type { ModelTask, ModelTrait } from '@/common/config/storage';
import type { ProviderId } from '@/common/types/ids';
import { compositeKey } from '@/common/utils/compositeKey';
import { modelHealthOf } from '@/common/utils/providerModels';
import { iconColors } from '@/renderer/styles/colors';
import { useModelsForTask } from '@/renderer/hooks/agent/useModelsForTask';
import { useModelSelectorProviderLabel } from '@/renderer/hooks/agent/useModelSelectorProviderLabel';

/** A concrete provider+model pick emitted by the unified selector. */
export interface TaskModelSelection {
  providerId: ProviderId;
  model: string;
}

export interface TaskModelSelectProps {
  /** Which task the listed models must support (catalog-resolved, no name heuristics). */
  task: ModelTask;
  /** Additional trait refinement within the task (e.g. `['vision_input']`). */
  requiredTraits?: ModelTrait[];
  /** Current selection; `null`/`undefined` renders the placeholder. */
  value?: TaskModelSelection | null;
  onSelect: (selection: TaskModelSelection) => void;
  placeholder?: string;
  disabled?: boolean;
  size?: 'mini' | 'small' | 'default' | 'large';
}

/**
 * Unified task-scoped provider+model dropdown (Menu.ItemGroup per provider,
 * per-model health dot, "(不可用)" disabled row for a stale current value, and
 * an empty-catalog state linking to the model management page). Behavior is
 * aligned with the Knowledge/Companion selectors; the model list comes from
 * `useModelsForTask` — the authoritative catalog resolution.
 */
const TaskModelSelect: React.FC<TaskModelSelectProps> = ({
  task,
  requiredTraits,
  value,
  onSelect,
  placeholder,
  disabled,
  size = 'default',
}) => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { groups, isLoading } = useModelsForTask(task, requiredTraits);
  const providerLabel = useModelSelectorProviderLabel();

  const valueAvailable =
    !value ||
    groups.some((group) => group.provider.id === value.providerId && group.models.includes(value.model));
  // A stale value stays visible (as a disabled "(不可用)" row + warning trigger)
  // until the user explicitly re-picks — never silently dropped.
  const valueUnavailable = Boolean(value && !isLoading && !valueAvailable);

  const buttonLabel = value
    ? valueUnavailable
      ? t('nomi.chat.modelUnavailableOption', { model: value.model })
      : value.model
    : (placeholder ?? t('conversation.welcome.selectModel'));

  const droplist = (
    <Menu selectedKeys={value ? [compositeKey(value.providerId, value.model)] : []}>
      {valueUnavailable && value && (
        <Menu.Item key={compositeKey(value.providerId, value.model)} disabled>
          {t('nomi.chat.modelUnavailableOption', { model: value.model })}
        </Menu.Item>
      )}
      {groups.length === 0
        ? [
            <Menu.Item
              key='no-models'
              className='px-12px py-12px text-t-secondary text-14px text-center flex justify-center items-center'
              disabled
            >
              {t('settings.noAvailableModels')}
            </Menu.Item>,
            <Menu.Item
              key='add-model'
              className='text-12px text-t-secondary'
              onClick={() => navigate('/models?section=models')}
            >
              <Plus theme='outline' size='12' />
              {t('settings.addModel')}
            </Menu.Item>,
          ]
        : groups.map(({ provider, models }) => (
            <Menu.ItemGroup title={providerLabel(provider)} key={provider.id}>
              {models.map((modelName) => {
                const healthStatus = modelHealthOf(provider, modelName)?.status || 'unknown';
                const healthColor =
                  healthStatus === 'healthy'
                    ? 'bg-green-500'
                    : healthStatus === 'unhealthy'
                      ? 'bg-red-500'
                      : 'bg-gray-400';
                return (
                  <Menu.Item
                    key={compositeKey(provider.id, modelName)}
                    className={
                      value?.providerId === provider.id && value?.model === modelName ? '!bg-2' : ''
                    }
                    onClick={() => onSelect({ providerId: provider.id, model: modelName })}
                  >
                    <div className='flex items-center gap-8px w-full'>
                      {healthStatus !== 'unknown' && (
                        <div className={`w-6px h-6px rounded-full shrink-0 ${healthColor}`} />
                      )}
                      <span>{modelName}</span>
                    </div>
                  </Menu.Item>
                );
              })}
            </Menu.ItemGroup>
          ))}
    </Menu>
  );

  return (
    <Dropdown trigger='click' droplist={droplist} disabled={disabled}>
      <Button
        size={size}
        disabled={disabled}
        status={valueUnavailable ? 'warning' : undefined}
        data-testid='task-model-select'
        title={valueUnavailable ? t('common.modelUnavailableHint') : undefined}
      >
        <span className='flex items-center gap-6px min-w-0 max-w-240px'>
          <Brain theme='outline' size='14' fill={iconColors.secondary} className='shrink-0' />
          <span className='truncate'>{buttonLabel}</span>
          <Down theme='outline' size='12' fill={iconColors.secondary} className='shrink-0' />
        </span>
      </Button>
    </Dropdown>
  );
};

export default TaskModelSelect;
