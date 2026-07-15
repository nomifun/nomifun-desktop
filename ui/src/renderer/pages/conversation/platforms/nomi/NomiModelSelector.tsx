/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { NomiModelSelection } from './useNomiModelSelection';
import { compositeKey } from '@/common/utils/compositeKey';
import { usePreviewContext } from '@/renderer/pages/conversation/Preview';
import { useLayoutContext } from '@/renderer/hooks/context/LayoutContext';
import { getModelDisplayLabel } from '@/renderer/utils/model/agentLogo';
import { iconColors } from '@/renderer/styles/colors';
import { Button, Dropdown, Menu, Tooltip } from '@arco-design/web-react';
import { Brain, Down } from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import classNames from 'classnames';

const NomiModelSelector: React.FC<{
  selection?: NomiModelSelection;
  disabled?: boolean;
  compact?: boolean;
  className?: string;
}> = ({ selection, disabled = false, compact: compactProp, className }) => {
  const { t } = useTranslation();
  const { isOpen: isPreviewOpen } = usePreviewContext();
  const layout = useLayoutContext();
  const compact = compactProp ?? (isPreviewOpen || layout?.isMobile);
  const isMobileHeaderCompact = Boolean(layout?.isMobile);
  const defaultModelLabel = t('common.defaultModel');

  const current_model = selection?.current_model;

  const renderLogo = () => <Brain theme='outline' size='14' fill={iconColors.secondary} className='shrink-0' />;

  if (disabled || !selection) {
    return (
      <Tooltip content={t('conversation.welcome.modelSwitchNotSupported')} position='top'>
        <Button
          className={classNames(
            'sendbox-model-btn header-model-btn min-w-0',
            compact ? '!max-w-[120px]' : '!max-w-[280px]',
            isMobileHeaderCompact && '!max-w-[160px]',
            className
          )}
          shape='round'
          size='small'
          style={{ cursor: 'default' }}
        >
          <span className='flex items-center gap-6px min-w-0'>
            {renderLogo()}
            <span className='block truncate min-w-0'>{t('conversation.welcome.useCliModel')}</span>
          </span>
        </Button>
      </Tooltip>
    );
  }

  const { providers, getAvailableModels, handleSelectModel } = selection;

  const label = getModelDisplayLabel({
    selected_value: current_model?.use_model,
    selectedLabel: current_model?.use_model || '',
    defaultModelLabel,
    fallbackLabel: t('conversation.welcome.selectModel'),
  });

  return (
    <Dropdown
      trigger='click'
      // Mobile: portal the popup to <body> so it escapes the titlebar slot.
      // Desktop: leave default container so click events reach Menu.Item normally.
      {...(isMobileHeaderCompact ? { getPopupContainer: () => document.body } : {})}
      droplist={
        <Menu>
          {providers.map((provider) => {
            const models = getAvailableModels(provider);
            if (!models.length) return null;

            return (
              <Menu.ItemGroup title={provider.name} key={provider.id}>
                {models.map((modelName) => (
                  <Menu.Item
                    key={compositeKey(provider.id, modelName)}
                    data-testid={`nomi-model-option-${modelName}`}
                    className={current_model?.id === provider.id && current_model?.use_model === modelName ? '!bg-2' : ''}
                    onClick={() => void handleSelectModel(provider, modelName)}
                  >
                    <div className='flex items-center gap-8px w-full'>
                      <span>{modelName}</span>
                    </div>
                  </Menu.Item>
                ))}
              </Menu.ItemGroup>
            );
          })}
        </Menu>
      }
    >
      <Button
        data-testid='nomi-model-selector'
        className={classNames(
          'sendbox-model-btn header-model-btn min-w-0',
          compact ? '!max-w-[120px]' : '!max-w-[280px]',
          isMobileHeaderCompact && '!max-w-[160px]',
          className
        )}
        shape='round'
        size='small'
      >
        <span className='flex items-center gap-6px min-w-0'>
          {renderLogo()}
          <span className='block truncate min-w-0'>{label}</span>
          <Down theme='outline' size={12} fill={iconColors.secondary} className='shrink-0' />
        </span>
      </Button>
    </Dropdown>
  );
};

export default NomiModelSelector;
