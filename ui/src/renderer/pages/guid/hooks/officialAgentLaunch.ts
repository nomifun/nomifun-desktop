import { agentPlatform } from '@/common/adapter/ipcBridge';
import type { TProviderWithModel } from '@/common/config/storage';
import type { OfficialPresetTemplate } from '@/common/types/agentPlatform';
import type { ExecutableAgentPreset } from '../types';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import { getConversationCreateErrorMessage } from '@/renderer/pages/conversation/utils/conversationCreateError';
import { conversationConfigurationErrorCode } from '@/renderer/pages/conversation/utils/conversationConfigurationError';
import type { TFunction } from 'i18next';

export function officialAgentLaunchError(error: unknown, t: TFunction): string {
  const codes: string[] = [];
  const reasons: string[] = [];
  if (isBackendHttpError(error)) {
    if (error.code) codes.push(error.code);
    const configurationCode = conversationConfigurationErrorCode({ code: error.code, error: error.backendMessage });
    if (configurationCode) codes.push(configurationCode);
    // The HTTP bridge has already redacted these fields. Keep the concrete
    // reason without rendering arbitrary response bodies or diagnostic data.
    reasons.push([error.code ? `[${error.code}]` : '', error.backendMessage].filter(Boolean).join(' '));
    const details = error.details as { diagnostics?: unknown[] } | undefined;
    if (Array.isArray(details?.diagnostics)) {
      for (const entry of details.diagnostics) {
        if (!entry || typeof entry !== 'object') continue;
        const diagnostic = entry as { code?: unknown; subject?: unknown; message?: unknown };
        if (typeof diagnostic.code === 'string') codes.push(diagnostic.code);
        if (reasons.length <= 5) {
          reasons.push([diagnostic.code, diagnostic.subject, diagnostic.message]
            .filter((value): value is string => typeof value === 'string' && Boolean(value.trim()))
            .join(': '));
        }
      }
    }
  } else {
    reasons.push(getConversationCreateErrorMessage(error, t));
  }
  const hint = codes.includes('AGENT_SESSION_CONTRACT_EVOLUTION_REJECTED')
    ? t('guid.agentEntries.contractChanged')
    : codes.some((code) => /MODEL_|CHAT_ROUTE|PROVIDER_/.test(code))
      ? t('guid.agentEntries.modelNeeded')
      : codes.some((code) => /CAPABILITY_|CODING_CODEX_|ROLE_COVERAGE/.test(code))
        ? t('guid.agentEntries.capabilitiesNeeded')
        : t('guid.agentEntries.launchFailed');
  return [...new Set([hint, ...reasons].filter(Boolean))].join('\n');
}

/** Prepare an internal session configuration without adding a personal Agent. */
export async function prepareOfficialAgent(
  template: OfficialPresetTemplate,
  model?: Pick<TProviderWithModel, 'id' | 'use_model'>,
  createFromTemplate = agentPlatform.createFromTemplate.invoke,
): Promise<ExecutableAgentPreset> {
  // Internal configurations store identity; localized names belong to UI copy.
  const editor = await createFromTemplate({
    template_id: template.template_key,
    request: {
      display_name: template.template_key,
      model_route_refs: {},
      chat_route_records: {},
      reuse_existing: true,
      ...(model && template.template_key !== 'creative-studio.default'
        ? { model: { provider_id: model.id, model: model.use_model } }
        : {}),
    },
  });
  if (!editor.preset.current_stable_revision) {
    throw new Error('AGENT_PRESET_REQUIRED');
  }
  return editor.preset as ExecutableAgentPreset;
}
