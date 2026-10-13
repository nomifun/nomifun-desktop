import type { ChatRouteRecord } from '@/common/types/agentPlatform';
import { agentPlatform } from '@/common/adapter/ipcBridge';
import { compositeKey } from '@/common/utils/compositeKey';
import { modelDisplayLabel } from '@/common/utils/modelPresentation';
import { useModelsForTask } from '@/renderer/hooks/agent/useModelsForTask';
import { useModelSelectorProviderLabel } from '@/renderer/hooks/agent/useModelSelectorProviderLabel';
import { Select } from '@arco-design/web-react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import styles from './AgentSettingsPage.module.css';

const NO_DEFAULT_MODEL = 'no-default-model';

export default function AgentDefaultModelPicker({ presetId, record, disabled, onChange, onPendingChange }: {
  presetId: string;
  record?: ChatRouteRecord;
  disabled: boolean;
  onChange: (record: ChatRouteRecord | undefined) => void;
  onPendingChange: (pending: boolean) => void;
}) {
  const { t } = useTranslation();
  const { groups, isLoading, error: catalogError } = useModelsForTask('chat');
  const providerLabel = useModelSelectorProviderLabel();
  const [pending, setPending] = useState(false);
  const [failed, setFailed] = useState(false);
  const requestVersion = useRef(0);
  const callbacks = useRef({ onChange, onPendingChange });
  callbacks.current = { onChange, onPendingChange };
  const choices = useMemo(() => groups.flatMap(({ provider, models }) => models.map(model => ({
    key: compositeKey(String(provider.id), model),
    provider_id: String(provider.id),
    model,
    label: `${providerLabel(provider)} / ${modelDisplayLabel(model, provider.models.find(item => item.model === model)?.display_name)}`,
  }))), [groups, providerLabel]);
  const selectedKey = record ? compositeKey(record.primary.provider_id, record.primary.model) : undefined;
  const savedChoiceMissing = Boolean(record && !choices.some(choice => choice.key === selectedKey));

  useEffect(() => {
    setPending(false);
    setFailed(false);
    callbacks.current.onPendingChange(false);
    return () => {
      requestVersion.current += 1;
      callbacks.current.onPendingChange(false);
    };
  }, [presetId, record]);

  const selectModel = async (key: string | undefined) => {
    const version = ++requestVersion.current;
    setFailed(false);
    if (!key || key === NO_DEFAULT_MODEL) {
      setPending(false);
      callbacks.current.onPendingChange(false);
      callbacks.current.onChange(undefined);
      return;
    }
    const choice = choices.find(item => item.key === key);
    if (!choice) return;
    setPending(true);
    callbacks.current.onPendingChange(true);
    try {
      const resolved = await agentPlatform.resolveChatModelRoute.invoke({
        preset_id: presetId,
        model: { provider_id: choice.provider_id, model: choice.model },
      });
      if (version === requestVersion.current) callbacks.current.onChange(resolved);
    } catch {
      if (version === requestVersion.current) setFailed(true);
    } finally {
      if (version === requestVersion.current) {
        setPending(false);
        callbacks.current.onPendingChange(false);
      }
    }
  };

  return <div className={styles.modelPicker} data-testid='agent-default-model-picker'>
    <Select
      aria-label={t('agentSettings.fields.defaultModel')}
      value={selectedKey}
      disabled={disabled}
      loading={pending || (isLoading && !catalogError)}
      allowClear
      showSearch
      placeholder={t('agentSettings.fields.defaultModelPlaceholder')}
      onChange={(key: string | undefined) => void selectModel(key)}
    >
      <Select.Option value={NO_DEFAULT_MODEL}>{t('agentSettings.fields.defaultModelPlaceholder')}</Select.Option>
      {savedChoiceMissing && <Select.Option value={selectedKey!} disabled>
        {modelDisplayLabel(record!.primary.model)}
      </Select.Option>}
      {choices.map(choice => <Select.Option key={choice.key} value={choice.key}>{choice.label}</Select.Option>)}
    </Select>
    <span className={styles.fieldHint}>{t('agentSettings.fields.defaultModelHint')}</span>
    {(failed || catalogError) && <span className={styles.fieldHint} role='alert'>{t('agentSettings.fields.defaultModelFailed')}</span>}
  </div>;
}
