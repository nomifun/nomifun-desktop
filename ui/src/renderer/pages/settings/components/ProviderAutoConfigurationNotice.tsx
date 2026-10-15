/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Spin, Tag } from '@arco-design/web-react';
import React, { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import type { ProviderCompatibilityMode } from './providerAutoConfiguration';
import type { ProviderAutoConfigurationBatch } from './useProviderAutoConfiguration';

const ProviderAutoConfigurationNotice: React.FC<{
  enabled: boolean;
  loading: boolean;
  batch?: ProviderAutoConfigurationBatch;
  mode?: ProviderCompatibilityMode;
}> = ({ enabled, loading, batch, mode = 'auto' }) => {
  const { t } = useTranslation();
  const protocols = useMemo(
    () => [...new Set(batch?.detections.map((detection) => detection.protocol) ?? [])],
    [batch?.detections]
  );
  if (!enabled || (!loading && protocols.length === 0)) return null;

  const confidence = batch?.detections.some(
    (detection) => detection.confidence === 'fallback'
  )
    ? 'fallback'
    : batch?.detections.some(
          (detection) => detection.confidence === 'endpoint_confirmed'
        )
      ? 'endpoint_confirmed'
      : 'verified';
  const adjustedUrl = batch?.detections.some((detection) =>
    Boolean(detection.suggestedBaseUrl)
  );

  return (
    <div
      className='flex min-h-34px items-center gap-8px border-0 border-l-2px border-l-solid border-primary-5 bg-fill-1 px-10px py-7px text-11px text-t-secondary'
      data-provider-auto-configuration={loading ? 'probing' : confidence}
      role='status'
      aria-live='polite'
    >
      {loading ? (
        <Spin size={14} />
      ) : (
        <Tag
          size='small'
          color={confidence === 'fallback' ? 'orange' : 'green'}
          bordered={false}
        >
          {mode === 'openai-responses'
            ? t('settings.providerCompatibilityMode.responses', {
                defaultValue: 'OpenAI Responses',
              })
            : mode === 'openai'
            ? t('settings.providerCompatibilityMode.openai', {
                defaultValue: 'Chat Completions',
              })
            : mode === 'anthropic'
              ? t('settings.providerCompatibilityMode.anthropic', {
                  defaultValue: 'Claude 兼容',
                })
              : t('settings.providerAutoConfiguration.applied', {
                  defaultValue: '已自动配置',
                })}
        </Tag>
      )}
      <span className='min-w-0 flex-1 leading-16px'>
        {loading
          ? t('settings.providerAutoConfiguration.detecting', {
              defaultValue: '正在检查接口地址及鉴权格式，不会生成模型回答…',
            })
          : confidence === 'verified'
            ? t('settings.providerAutoConfiguration.verified', {
                protocols: protocols.join(', '),
                defaultValue: '已探测到接口 {{protocols}}；尚未验证当前模型，请点击「检测并选择可用协议」确认。',
              })
            : confidence === 'endpoint_confirmed'
              ? t('settings.providerAutoConfiguration.endpointConfirmed', {
                  protocols: protocols.join(', '),
                  defaultValue:
                    '已确认接口 {{protocols}}；当前密钥未通过验证，尚未验证当前模型。',
                })
              : t('settings.providerAutoConfiguration.fallback', {
                  protocols: protocols.join(', '),
                  defaultValue:
                    '接口在线确认未完成，暂用通用协议 {{protocols}}；可检测当前模型后再确认。',
                })}
        {!loading && adjustedUrl
          ? ` ${t('settings.providerAutoConfiguration.urlAdjusted', {
              defaultValue: 'API 地址已自动修正。',
            })}`
          : ''}
      </span>
    </div>
  );
};

export default ProviderAutoConfigurationNotice;
