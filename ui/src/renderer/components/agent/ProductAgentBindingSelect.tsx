import { agentPlatform, companion } from '@/common/adapter/ipcBridge';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import type { TProviderWithModel } from '@/common/config/storage';
import type { OfficialPresetKey, ProductAgentOptions, ProductAgentSelection } from '@/common/types/agentPlatform';
import { parseCompanionId } from '@/common/types/ids';
import { TEMPLATE_I18N_PATH } from '@/renderer/pages/agentSettings/model';
import { AgentIdentityBadge, AgentLogoIcon } from './AgentBadge';
import { Button, Dropdown, Menu, Message, Select, Tooltip } from '@arco-design/web-react';
import { Down } from '@icon-park/react';
import React, { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import useSWR from 'swr';

type Props = {
  targetKind: 'companion' | 'robot' | 'customer' | 'creative_studio_canvas';
  targetId: string;
  defaultTemplateKey: OfficialPresetKey;
  model?: Pick<TProviderWithModel, 'id' | 'use_model'>;
  conversationId?: string | null;
  disabled?: boolean;
  compact?: boolean;
  onChanged?: () => void;
  onSavingChange?: (saving: boolean) => void;
};

export const productSelectionValue = (selection: ProductAgentSelection): string =>
  selection.kind === 'template' ? `template:${selection.template_key}` : `preset:${selection.preset_id}`;

/** Only localized categories reach a toast; backend envelopes stay in diagnostics. */
export function productAgentErrorReason(error: unknown): 'modelChanged' | 'busy' | 'saveFailed' {
  if (isBackendHttpError(error)) {
    if (error.code.startsWith('MODEL_')) return 'modelChanged';
    if (error.code.endsWith('_BUSY')) return 'busy';
  }
  return 'saveFailed';
}

const ProductAgentBindingSelect: React.FC<Props> = ({ targetKind, targetId, model, conversationId, disabled = false, compact = false, onChanged, onSavingChange }) => {
  const { t } = useTranslation();
  const selectedModel = model?.id && model.use_model ? { provider_id: model.id, model: model.use_model } : undefined;
  const key = ['product-agent-options', targetKind, targetId, selectedModel?.provider_id ?? '', selectedModel?.model ?? ''];
  const requestKey = JSON.stringify(key);
  const activeRequest = useRef(requestKey);
  activeRequest.current = requestKey;
  const mounted = useRef(true);
  const savingRef = useRef(false);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string>();
  const { data, error, isLoading, mutate } = useSWR<ProductAgentOptions>(key,
    () => agentPlatform.productBindingOptions.invoke({ target_kind: targetKind, target_id: targetId, model: selectedModel }),
    { shouldRetryOnError: false });
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => { setSaveError(undefined); }, [requestKey]);
  const current = () => mounted.current && activeRequest.current === requestKey;

  const nameFor = (option: ProductAgentOptions['options'][number]): string => {
    const key = option.selection.kind === 'template' ? option.selection.template_key : option.display_name;
    if (Object.prototype.hasOwnProperty.call(TEMPLATE_I18N_PATH, key)) {
      return t(`agentSettings.template.${TEMPLATE_I18N_PATH[key as OfficialPresetKey]}.name`);
    }
    return option.display_name || t('agentSettings.productBinding.unavailableAgent');
  };

  const change = async (value: string) => {
    const option = data?.options.find((option) => productSelectionValue(option.selection) === value);
    if (disabled || savingRef.current || !option?.available || !data || value === productSelectionValue(data.selection)) return;
    savingRef.current = true;
    setSaving(true);
    onSavingChange?.(true);
    setSaveError(undefined);
    try {
      const activeId = selectedModel ? conversationId ?? (targetKind === 'companion'
        ? (await companion.getCompanionSession.invoke({ companion_id: parseCompanionId(targetId) })).conversation_id
        : null) : null;
      if (!current()) return;
      const result = await agentPlatform.selectProductBinding.invoke({
        target_kind: targetKind, target_id: targetId,
        request: { selection: option.selection, ...(selectedModel ? { model: selectedModel } : {}), ...(activeId ? { conversation_id: activeId } : {}) },
      });
      if (!current()) return;
      await mutate({ ...data, selection: result.selection, needs_model: result.needs_model }, { revalidate: true });
      if (!current()) return;
      Message.success(t('agentSettings.productBinding.saved'));
      onChanged?.();
    } catch (reason) {
      if (!current()) return;
      const message = t(`agentSettings.productBinding.${productAgentErrorReason(reason)}`);
      setSaveError(message);
      Message.error(message);
      void mutate();
    } finally {
      savingRef.current = false;
      if (mounted.current) {
        setSaving(false);
        onSavingChange?.(false);
      }
    }
  };

  if (error) return <div role='alert' className='inline-flex min-w-0 items-center gap-8px'>
    <AgentIdentityBadge
      backend='nomi'
      name={t('agent.identity.unavailable', { defaultValue: 'Unavailable' })}
      compact
    />
    <span>{t('agentSettings.productBinding.loadFailed')}</span>
    <Button size='mini' onClick={() => void mutate()}>{t('agentSettings.actions.retry')}</Button>
  </div>;
  if (isLoading || !data) {
    return (
      <AgentIdentityBadge
        backend='nomi'
        loading
        compact
      />
    );
  }
  const selected = data.options.find((option) => productSelectionValue(option.selection) === productSelectionValue(data.selection));
  const selectedName = selected
    ? nameFor(selected)
    : t('agentSettings.productBinding.unavailableAgent');
  if (compact) return (
    <Tooltip content={t('agentSettings.productBinding.companionHint')}>
      <span className='inline-flex min-w-0'>
      <Dropdown trigger='click' disabled={disabled || saving} droplist={
        <Menu>
          {data.options.map((option) => <Menu.Item
            key={productSelectionValue(option.selection)}
            disabled={!option.available}
            onClick={() => void change(productSelectionValue(option.selection))}
          >
            <span className='flex flex-col'>
              <span>{nameFor(option)}</span>
              {!option.available && <small>{t(`agentSettings.productBinding.reasons.${option.reason ?? 'capability'}`)}</small>}
            </span>
          </Menu.Item>)}
        </Menu>
      }>
        <Button size='small' shape='round' loading={saving} disabled={disabled || saving}
          className='sendbox-model-btn header-model-btn nomi-sendbox-agent-btn min-w-0'
          aria-label={`${t('agentSettings.productBinding.label')}: ${selectedName}`}
          data-agent-identity
          data-agent-name={selectedName}>
          <span className='flex items-center gap-6px min-w-0'>
            <AgentLogoIcon backend='nomi' agent_name={selectedName} />
            <span className='sendbox-responsive-label truncate'>{selectedName}</span>
            <Down theme='outline' size={12} className='sendbox-responsive-chevron' />
          </span>
        </Button>
      </Dropdown>
      </span>
    </Tooltip>
  );
  return <div className='flex min-w-0 flex-col gap-6px' style={{ maxWidth: 300 }}>
    <AgentIdentityBadge backend='nomi' name={selectedName} />
    <Select value={productSelectionValue(data.selection)} loading={saving} disabled={disabled || saving}
      onChange={(next: string) => void change(next)} style={{ width: 230, maxWidth: '100%' }}
      aria-label={t('agentSettings.productBinding.label')}>
      {data.options.map((option) => {
        const reason = option.reason ? t(`agentSettings.productBinding.reasons.${option.reason}`) : '';
        return <Select.Option key={productSelectionValue(option.selection)} value={productSelectionValue(option.selection)} disabled={!option.available}>
          <span title={reason} className='flex flex-col'>
            <span>{nameFor(option)}</span>
            {!option.available && <small className='whitespace-normal'>{reason}</small>}
          </span>
        </Select.Option>;
      })}
    </Select>
    {data.needs_model && <span className='text-12px text-t-secondary'>{t('agentSettings.productBinding.chooseModelLater')}</span>}
    {selected && !selected.available && <span role='status' className='text-12px text-t-secondary'>
      {t('agentSettings.productBinding.currentUnavailable', { reason: t(`agentSettings.productBinding.reasons.${selected.reason ?? 'capability'}`) })}
    </span>}
    {saveError && <span role='alert' className='text-12px'>{saveError}</span>}
  </div>;
};

export default ProductAgentBindingSelect;
