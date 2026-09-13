import { ipcBridge } from '@/common';
import type { RuntimeEngineDescriptor, RuntimeEngineSelection } from '@/common/types/agentPlatform';
import { Alert, Select } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import useSWR from 'swr';

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
      value: JSON.stringify(selection),
      label: `${engine.display_name} · ${profile} · ${engine.build_id}`,
      selection,
    };
  }));
}

/** Discovery is dynamic; HTTP/model output cannot register executable code. */
export default function GuidRuntimeEngineSelector({ value, onChange, disabled }: {
  value?: RuntimeEngineSelection;
  onChange: (value: RuntimeEngineSelection | undefined) => void;
  disabled?: boolean;
}) {
  const { t } = useTranslation();
  const { data, error, isLoading } = useSWR('runtime-engines', () =>
    ipcBridge.agentPlatform.runtimeEngines.list.invoke()
  );
  const options = runtimeEngineOptions(data ?? []);
  const key = value ? JSON.stringify(value) : '';
  const unavailable = Boolean(value && data && !options.some((item) => item.value === key));
  return (
    <div className='flex flex-col gap-2'>
      <Select
        aria-label={t('guid.runtimeEngine.label')}
        value={key}
        disabled={disabled}
        loading={isLoading}
        style={{ minWidth: 200, maxWidth: 420 }}
        options={[
          { value: '', label: t('guid.runtimeEngine.default') },
          ...options,
          ...(unavailable ? [{ value: key, label: t('guid.runtimeEngine.unavailable'), disabled: true }] : []),
        ]}
        onChange={(next: string) => {
          if (next === '') onChange(undefined);
          else {
            const option = options.find((item) => item.value === next);
            if (option) onChange(option.selection);
          }
        }}
      />
      {(error || unavailable) && <Alert type='warning' content={t('guid.runtimeEngine.unavailable')} />}
      {value?.selector.family_id === 'nomifun.coding' && (
        <Alert type='info' content={t('guid.runtimeEngine.codingLimit')} />
      )}
    </div>
  );
}
