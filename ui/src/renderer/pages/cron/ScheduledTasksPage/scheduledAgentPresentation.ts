import type { ICronJob } from '@/common/adapter/ipcBridge';
import type { TFunction } from 'i18next';
import { TEMPLATE_I18N_PATH } from '@/renderer/pages/agentSettings/model';
import { getAgentLogo } from '@/renderer/utils/model/agentLogo';

export function scheduledAgentName(job: ICronJob, t: TFunction): string {
  const selection = job.metadata.agent_selection;
  const label = job.metadata.agent_label?.trim();
  if (selection?.kind === 'template' && (!label || label === selection.template_key)) return t(`agentSettings.template.${TEMPLATE_I18N_PATH[selection.template_key]}.name`);
  return label || t('cron.page.form.unknownAgentLabel');
}

export const scheduledAgentLogo = getAgentLogo('nomi');

export function scheduledWorkspace(job: ICronJob): string | undefined {
  return job.metadata.agent_binding?.typed_resource_bindings.find(binding => binding.resource_kind === 'workspace')?.typed_parameters?.workspace_root;
}
