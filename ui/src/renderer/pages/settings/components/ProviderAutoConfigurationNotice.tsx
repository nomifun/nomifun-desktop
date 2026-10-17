/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Button, Spin, Tag } from '@arco-design/web-react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import type { ProviderAutoConfigurationDetection, ProviderCompatibilityMode } from './providerAutoConfiguration';
import type { ModelDefinitionDraft } from './providerModelAdvanced';
import type { ProviderAutoConfigurationBatch } from './useProviderAutoConfiguration';

const ProviderAutoConfigurationNotice: React.FC<{
  enabled: boolean;
  loading: boolean;
  stale?: boolean;
  batch?: ProviderAutoConfigurationBatch;
  mode?: ProviderCompatibilityMode;
  definition?: ModelDefinitionDraft;
  onSelect?: (detection: ProviderAutoConfigurationDetection) => void;
  onRecheck?: () => void;
  canRecheck?: boolean;
}> = ({ enabled, loading, stale = false, batch, mode = 'auto', definition, onSelect, onRecheck, canRecheck = true }) => {
  const { t } = useTranslation();
  const candidates = batch?.candidates ?? batch?.detections ?? [];
  const selectedDetections = candidates.filter(candidate => definition?.capabilities.some(capability =>
    capability.task === candidate.task && capability.protocol === candidate.protocol
  ));
  const detections = selectedDetections.length > 0 ? selectedDetections : batch?.detections ?? [];
  const protocols = [...new Set(detections.map(detection => detection.protocol))];
  const hasMultipleTasks = new Set(candidates.map(candidate => candidate.task)).size > 1;
  const hasResults = protocols.length > 0 || candidates.length > 0;
  const staleTasks = batch?.staleTasks;
  const hasStaleResults = stale || Boolean(staleTasks?.length);
  if (!enabled || (!loading && !hasResults && !onRecheck)) return null;

  const confidence = detections.some(
    (detection) => detection.confidence === 'fallback'
  )
    ? 'fallback'
    : detections.some(
          (detection) => detection.confidence === 'credentials_rejected'
        )
      ? 'credentials_rejected'
      : 'reachable';
  const adjustedUrl = detections.some((detection) =>
    Boolean(detection.suggestedBaseUrl)
  );

  return (
    <div
      className='space-y-8px border-0 border-l-2px border-l-solid border-primary-5 bg-fill-1 px-10px py-7px text-11px text-t-secondary'
      data-provider-auto-configuration={loading ? 'probing' : hasStaleResults ? 'stale' : hasResults ? confidence : 'unconfirmed'}
    >
      <div className='flex min-h-20px items-center gap-8px' role='status' aria-live='polite'>
        {loading ? (
          <Spin size={14} />
        ) : !hasStaleResults && hasResults ? (
          <Tag
            size='small'
            color={confidence === 'fallback' ? 'orange' : 'arcoblue'}
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
        ) : null}
        <span className='min-w-0 flex-1 leading-16px'>
          {loading
            ? t('settings.providerAutoConfiguration.detecting', {
                defaultValue: '正在检查接口地址及鉴权格式，不会生成模型回答…',
              })
            : hasStaleResults
              ? t('settings.providerAutoConfiguration.changed', {
                  defaultValue: '配置已修改，尚未重新检测',
                })
              : !hasResults
                ? t(canRecheck ? 'settings.healthNotChecked' : 'settings.modelProtocolDiscovery.needsConfiguration')
                : confidence === 'reachable'
                  ? t('settings.providerAutoConfiguration.reachable', {
                      protocols: protocols.join(', '),
                      defaultValue: '已识别接口 {{protocols}}；确认模型 ID 后会用真实请求自动验证，可用性以该结果为准。',
                    })
                  : confidence === 'credentials_rejected'
                    ? t('settings.providerAutoConfiguration.keyRejected', {
                        protocols: protocols.join(', '),
                        defaultValue:
                          '已识别接口 {{protocols}}，但密钥被拒绝（401）；请检查 API Key。',
                      })
                    : t('settings.providerAutoConfiguration.fallback', {
                        protocols: protocols.join(', '),
                        defaultValue:
                          '接口在线确认未完成，暂用通用协议 {{protocols}}；确认模型 ID 后会用真实请求自动验证。',
                      })}
          {!loading && !hasStaleResults && adjustedUrl
            ? ` ${t('settings.providerAutoConfiguration.urlAdjusted', {
                defaultValue: 'API 地址已自动修正。',
              })}`
            : ''}
        </span>
        {onRecheck && <Button size='mini' type='secondary' disabled={loading || !canRecheck}
          onClick={onRecheck} data-provider-protocol-recheck>
          {t('settings.modelProtocolDiscovery.recheck', { defaultValue: '重新检测' })}
        </Button>}
      </div>
      {candidates.length > 0 && <div className='space-y-6px' role='group' aria-label={t('settings.providerAutoConfiguration.results', { defaultValue: '接口检测结果' })}>
        {candidates.map(candidate => {
          const candidateStale = staleTasks ? staleTasks.includes(candidate.task) : stale;
          const selected = definition?.capabilities.some(capability =>
            capability.task === candidate.task && capability.protocol === candidate.protocol
          ) ?? false;
          const status = candidateStale
            ? t('settings.providerAutoConfiguration.unconfirmed', { defaultValue: '未确认' })
            : candidate.confidence === 'reachable'
              ? t('settings.providerAutoConfiguration.identified', { defaultValue: '已识别' })
              : candidate.confidence === 'credentials_rejected'
                ? t('settings.providerAutoConfiguration.credentialsRejected', { defaultValue: '密钥被拒绝（401）' })
                : t('settings.providerAutoConfiguration.unconfirmed', { defaultValue: '未确认' });
          return <div key={`${candidate.task}:${candidate.protocol}`} className='flex flex-wrap items-center gap-6px' data-provider-protocol-result={candidate.protocol} data-provider-protocol-task={candidate.task}>
            <Button size='mini' type={selected ? 'primary' : 'secondary'} aria-pressed={selected}
              disabled={loading || candidateStale || !onSelect} onClick={() => onSelect?.(candidate)}>
              {hasMultipleTasks ? `${t(`settings.modelTask.${candidate.task}`, { defaultValue: candidate.task })} · ` : ''}{candidate.protocol}
            </Button>
            <Tag size='small' bordered={false} color={!candidateStale && candidate.confidence === 'reachable' ? 'gray' : 'orange'}>{status}</Tag>
            {candidate.suggestedBaseUrl && <span className='min-w-0 break-all text-t-secondary'>{candidate.suggestedBaseUrl}</span>}
          </div>;
        })}
      </div>}
    </div>
  );
};

export default ProviderAutoConfigurationNotice;
