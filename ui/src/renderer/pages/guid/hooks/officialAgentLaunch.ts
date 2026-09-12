import { agentPlatform } from '@/common/adapter/ipcBridge';
import type { TProviderWithModel } from '@/common/config/storage';
import type { OfficialPresetTemplate } from '@/common/types/agentPlatform';
import type { ExecutableAgentPreset } from '../types';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import type { TFunction } from 'i18next';

export function officialAgentLaunchError(error: unknown, t: TFunction): string {
  const codes: string[] = [];
  if (isBackendHttpError(error)) {
    if (error.code) codes.push(error.code);
    const details = error.details as { diagnostics?: Array<{ code?: string }> } | undefined;
    if (Array.isArray(details?.diagnostics)) {
      codes.push(...details.diagnostics.flatMap((entry) => typeof entry.code === 'string' ? [entry.code] : []));
    }
  }
  if (codes.some((code) => /MODEL_|CHAT_ROUTE|PROVIDER_/.test(code))) return t('guid.agentEntries.modelNeeded');
  if (codes.some((code) => /CAPABILITY_|CODING_CODEX_|ROLE_COVERAGE/.test(code))) return t('guid.agentEntries.capabilitiesNeeded');
  return t('guid.agentEntries.launchFailed');
}

/** Prepare an internal session configuration without adding a personal Agent. */
export async function prepareOfficialAgent(
  template: OfficialPresetTemplate,
  displayName: string,
  model?: Pick<TProviderWithModel, 'id' | 'use_model'>,
  createFromTemplate = agentPlatform.createFromTemplate.invoke,
): Promise<ExecutableAgentPreset> {
  const editor = await createFromTemplate({
    template_id: template.template_key,
    request: {
      display_name: displayName,
      model_route_refs: {},
      chat_route_records: {},
      reuse_existing: true,
      ...(model
        ? { model: { provider_id: model.id, model: model.use_model } }
        : {}),
    },
  });
  if (!editor.preset.current_stable_revision) {
    throw new Error('AGENT_PRESET_REQUIRED');
  }
  return editor.preset as ExecutableAgentPreset;
}
