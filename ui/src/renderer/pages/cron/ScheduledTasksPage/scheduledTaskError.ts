import type { AgentStreamErrorInfo } from '@/common/chat/chatLib';

/** Legacy summaries are diagnostic text; only a typed host configuration code establishes a cause. */
export function scheduledTaskError(summary: string, configurationCode?: string): AgentStreamErrorInfo {
  return {
    message: summary,
    ...(['CRON_CONFIGURATION_RESELECTION_REQUIRED', 'CRON_MODEL_RESELECTION_REQUIRED'].includes(configurationCode ?? '')
      ? { code: configurationCode, retryable: false, ownership: 'nomifun' as const } : {}),
  };
}
