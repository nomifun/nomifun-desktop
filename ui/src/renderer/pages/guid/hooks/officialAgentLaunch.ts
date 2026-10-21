import { agentPlatform } from '@/common/adapter/ipcBridge';
import type { TProviderWithModel } from '@/common/config/storage';
import type { OfficialPresetTemplate } from '@/common/types/agentPlatform';
import type { ExecutableAgentPreset } from '../types';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import { getAgentPreparationDiagnostics, getConversationCreateErrorMessage } from '@/renderer/pages/conversation/utils/conversationCreateError';
import { conversationConfigurationErrorCode } from '@/renderer/pages/conversation/utils/conversationConfigurationError';
import type { TFunction } from 'i18next';

export function officialAgentLaunchError(error: unknown, t: TFunction): string {
  if (isBackendHttpError(error) && conversationConfigurationErrorCode({ code: error.code, error: error.backendMessage })) {
    return getConversationCreateErrorMessage(error, t);
  }
  const codes: string[] = [];
  const reasons: string[] = [];
  if (isBackendHttpError(error)) {
    const diagnostics = getAgentPreparationDiagnostics(error);
    codes.push(...diagnostics.codes);
    reasons.push(...diagnostics.reasons);
    const configurationCode = conversationConfigurationErrorCode({ code: error.code, error: error.backendMessage });
    if (configurationCode) codes.push(configurationCode);
  } else {
    reasons.push(getConversationCreateErrorMessage(error, t));
  }
  const hint = codes.some(code => code === 'AGENT_SESSION_CONTRACT_EVOLUTION_REJECTED' || code === 'CAPABILITY_ACTION_NOT_MATERIALIZED')
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
