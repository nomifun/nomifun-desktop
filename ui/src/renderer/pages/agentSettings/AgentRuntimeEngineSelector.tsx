import { ipcBridge } from '@/common';
import type { RuntimeEngineDescriptor, RuntimeEngineSelection } from '@/common/types/agentPlatform';
import { Alert, Select } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import useSWR from 'swr';

export function runtimeEngineKey(value?: RuntimeEngineSelection): string {
  if (!value) return '';
  const selector = value.selector;
  return JSON.stringify(selector.selection === 'exact'
    ? [selector.selection, selector.family_id, selector.build_id, selector.build_digest, value.profile]
    : [selector.selection, selector.family_id, selector.channel, value.profile]);
}

export function runtimeEngineOptions(catalog: RuntimeEngineDescriptor[]) {
  return catalog.flatMap((engine) => engine.supported_profiles.map((profile) => {
    const selection: RuntimeEngineSelection = {
      selector: {
        selection: 'exact',
        family_id: engine.family_id,
        build_id: engine.build_id,
        build_digest: engine.build_digest,
      },
      profile,
    };
    return {
      value: runtimeEngineKey(selection),
      label: `${engine.display_name} · ${profile} · ${engine.build_id}`,
      selection,
    };
  }));
}

/** Discovery is dynamic; HTTP/model output cannot register executable code. */
export default function AgentRuntimeEngineSelector({ value, onChange, disabled }: {
  value?: RuntimeEngineSelection;
  onChange: (value: RuntimeEngineSelection | undefined) => void;
  disabled?: boolean;
}) {
  const { t } = useTranslation();
  const { data, error, isLoading } = useSWR('runtime-engines', () =>
    ipcBridge.agentPlatform.runtimeEngines.list.invoke()
  );
  const options = runtimeEngineOptions(data ?? []);
  const key = runtimeEngineKey(value);
  // Channel preferences can be authored by an embedding host. Discovery lists
  // builds, not channel aliases; Preview/Save validates the alias server-side.
  const channel = value?.selector.selection === 'channel' ? value.selector : undefined;
  const channelEngine = channel && data?.find((engine) =>
    engine.family_id === channel.family_id && engine.supported_profiles.includes(value!.profile));
  if (channelEngine && channel && value) {
    options.push({ value: key, label: `${channelEngine.display_name} · ${value.profile} · ${channel.channel}`, selection: value });
  }
  const unavailable = Boolean(value && data && !options.some((item) => item.value === key));
  return (
    <div className='flex flex-col gap-2'>
      <Select
        aria-label={t('agentSettings.runtimeEngine.label')}
        value={key}
        disabled={disabled}
        loading={isLoading}
        style={{ minWidth: 200, maxWidth: 420 }}
        options={[
          { value: '', label: t('agentSettings.runtimeEngine.default') },
          ...options,
          ...(unavailable ? [{ value: key, label: t('agentSettings.runtimeEngine.unavailable'), disabled: true }] : []),
        ]}
        onChange={(next: string) => {
          if (next === '') onChange(undefined);
          else {
            const option = options.find((item) => item.value === next);
            if (option) onChange(option.selection);
          }
        }}
      />
      <span>{t('agentSettings.runtimeEngine.hint')}</span>
      {(error || unavailable) && <Alert type='warning' content={t('agentSettings.runtimeEngine.unavailable')} />}
      {value?.selector.family_id === 'nomifun.coding' && (
        <Alert type='info' content={t('agentSettings.runtimeEngine.codingLimit')} />
      )}
    </div>
  );
}
