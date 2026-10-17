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
        className='flex flex-wrap w-full [&_.arco-radio-button]:flex-1'
        onChange={(next) => onChange(next as ProviderCompatibilityMode)}
      >
        <Radio value='auto' data-provider-compatibility-option='auto'>
          {t('settings.providerCompatibilityMode.auto', {
            defaultValue: '自动识别',
          })}
        </Radio>
        <Radio value='openai-responses' data-provider-compatibility-option='openai-responses'>
          {t('settings.providerCompatibilityMode.responses', {
            defaultValue: 'OpenAI Responses',
          })}
        </Radio>
        <Radio value='openai' data-provider-compatibility-option='openai'>
          {t('settings.providerCompatibilityMode.openai', {
            defaultValue: 'Chat Completions',
          })}
        </Radio>
        <Radio value='anthropic' data-provider-compatibility-option='anthropic'>
          {t('settings.providerCompatibilityMode.anthropic', {
            defaultValue: 'Claude 兼容',
          })}
        </Radio>
      </Radio.Group>
      <div className='text-11px leading-4 text-t-secondary'>
        {value === 'auto'
          ? t('settings.providerCompatibilityMode.autoHint', {
              defaultValue: '探测可用协议；无法确认时使用 Chat Completions。仅支持 Responses 的网关请选择 OpenAI Responses。',
            })
          : value === 'openai-responses'
            ? t('settings.providerCompatibilityMode.responsesHint', {
                defaultValue: '使用 /responses，适合新版 OpenAI 模型及支持 Responses 的网关；网关需明确支持此接口。',
              })
          : value === 'openai'
            ? t('settings.providerCompatibilityMode.openaiHint', {
                defaultValue:
                  '使用 /chat/completions，适合提供该接口的 OpenAI 兼容供应商；与 Responses 是不同接口。',
              })
            : t('settings.providerCompatibilityMode.anthropicHint', {
                defaultValue:
                  '使用 anthropic.messages 和 x-api-key 鉴权；最大输出需按模型文档配置。',
              })}
      </div>
    </div>
  );
};

export default ProviderCompatibilityModePicker;
