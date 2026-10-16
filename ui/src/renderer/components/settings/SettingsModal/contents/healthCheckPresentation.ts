import type { ModelTask } from '@/common/protocolBindings/ModelTask';
import type { HealthStatus } from '@/common/protocolBindings/HealthStatus';
import type { ProviderModelCapabilityResponse } from '@/common/types/provider/providerModel';

type Translate = (key: string, options?: { defaultValue?: string }) => string;

/** A checked but unverified task must not disappear behind another green task. */
export const capabilityForHealthSummary = (capabilities: readonly ProviderModelCapabilityResponse[]) =>
  capabilities.find(capability => capability.health?.status === 'unhealthy')
  ?? capabilities.find(capability => capability.health?.status === 'unknown')
  ?? capabilities.find(capability => capability.health);

export const unverifiedHealthMessage = (t: Translate, task: ModelTask): string =>
  task === 'realtime_conversation'
    ? t('settings.healthRealtimeUnverified', { defaultValue: '会话已建立，尚未验证音频对话。' })
    : t('settings.healthAcceptedUnverified', { defaultValue: '请求已受理，尚未验证最终产物。' });

export const healthStatusLabel = (t: Translate, status: HealthStatus, checked: boolean): string =>
  status === 'healthy' ? t('common.success')
    : status === 'unhealthy' ? t('common.failed')
    : checked ? t('settings.healthUnverified', { defaultValue: '尚未完整验证' })
    : t('settings.healthNotChecked', { defaultValue: '未检测' });
