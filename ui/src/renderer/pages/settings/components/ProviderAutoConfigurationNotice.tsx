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
      className='flex min-h-34px items-center gap-8px border-0 border-l-2 border-solid border-primary-5 bg-fill-1 px-10px py-7px text-11px text-t-secondary'
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
          {mode === 'openai'
            ? t('settings.providerCompatibilityMode.openai', {
                defaultValue: 'OpenAI 兼容',
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
              defaultValue: '正在探测协议、鉴权方式和可用 API 地址…',
            })
          : confidence === 'verified'
            ? t('settings.providerAutoConfiguration.verified', {
                protocols: protocols.join(', '),
                defaultValue: '已验证并采用 {{protocols}}；仍可在高级配置中修改。',
              })
            : confidence === 'endpoint_confirmed'
              ? t('settings.providerAutoConfiguration.endpointConfirmed', {
                  protocols: protocols.join(', '),
                  defaultValue:
                    '已确认接口并采用 {{protocols}}；当前密钥未通过验证，可稍后替换。',
                })
              : t('settings.providerAutoConfiguration.fallback', {
                  protocols: protocols.join(', '),
                  defaultValue:
                    '暂未完成在线验证，已安装通用方案 {{protocols}}；可继续配置并在稍后调整。',
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
