/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Radio } from '@arco-design/web-react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import type { ProviderCompatibilityMode } from './providerAutoConfiguration';

const ProviderCompatibilityModePicker: React.FC<{
  value: ProviderCompatibilityMode;
  onChange: (mode: ProviderCompatibilityMode) => void;
}> = ({ value, onChange }) => {
  const { t } = useTranslation();
  return (
    <div className='space-y-6px' data-provider-compatibility-mode>
      <div className='text-13px font-500 text-t-secondary'>
        {t('settings.providerCompatibilityMode.title', {
          defaultValue: '接口模式',
        })}
      </div>
      <Radio.Group
        type='button'
        value={value}
        className='flex w-full [&_.arco-radio-button]:flex-1'
        onChange={(next) => onChange(next as ProviderCompatibilityMode)}
      >
        <Radio value='auto' data-provider-compatibility-option='auto'>
          {t('settings.providerCompatibilityMode.auto', {
            defaultValue: '自动识别',
          })}
        </Radio>
        <Radio value='openai' data-provider-compatibility-option='openai'>
          {t('settings.providerCompatibilityMode.openai', {
            defaultValue: 'OpenAI 兼容',
          })}
        </Radio>
        <Radio value='anthropic' data-provider-compatibility-option='anthropic'>
          {t('settings.providerCompatibilityMode.anthropic', {
            defaultValue: 'Claude 兼容',
          })}
        </Radio>
      </Radio.Group>
      <div className='text-11px leading-4 text-t-tertiary'>
        {value === 'auto'
          ? t('settings.providerCompatibilityMode.autoHint', {
              defaultValue: '系统会探测可用协议；无法确认时默认使用 OpenAI 兼容模式。',
            })
          : value === 'openai'
            ? t('settings.providerCompatibilityMode.openaiHint', {
                defaultValue:
                  '预置 openai.chat_text、Bearer 鉴权和 /v1 根路径，适用于 OpenAI 兼容网关。',
              })
            : t('settings.providerCompatibilityMode.anthropicHint', {
                defaultValue:
                  '预置 anthropic.messages、x-api-key 鉴权和 Claude 所需的最大输出配置。',
              })}
      </div>
    </div>
  );
};

export default ProviderCompatibilityModePicker;

