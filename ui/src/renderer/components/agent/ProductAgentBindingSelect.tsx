import { agentPlatform } from '@/common/adapter/ipcBridge';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import type { TProviderWithModel } from '@/common/config/storage';
import type { OfficialPresetKey, ProductAgentOptions, ProductAgentSelection } from '@/common/types/agentPlatform';
import { TEMPLATE_I18N_PATH } from '@/renderer/pages/agentSettings/model';
import { AgentLogoIcon } from './AgentBadge';
import NomiSelect from '@/renderer/components/base/NomiSelect';
import { Button, Dropdown, Menu, Message, Spin, Tooltip } from '@arco-design/web-react';
import { Down } from '@icon-park/react';
import React, { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Link } from 'react-router-dom';
import useSWR from 'swr';
import styles from './ProductAgentBindingSelect.module.css';

type Props = {
  targetKind: 'companion' | 'customer' | 'creative_studio_canvas';
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
export function productAgentErrorReason(error: unknown): 'modelChanged' | 'busy' | 'partialSave' | 'saveFailed' {
  if (isBackendHttpError(error)) {
    if (error.code === 'COMPANION_AGENT_DEFAULT_SAVE_FAILED') return 'partialSave';
    if (error.code.startsWith('MODEL_') || error.code === 'AGENT_SESSION_MODEL_INCOMPATIBLE') return 'modelChanged';
    if (error.code.endsWith('_BUSY') || error.code === 'AGENT_SESSION_TURN_ACTIVE') return 'busy';
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
    const key = option.selection.kind === 'template' ? option.selection.template_key : undefined;
    if (key && Object.prototype.hasOwnProperty.call(TEMPLATE_I18N_PATH, key)) {
      return t(`agentSettings.template.${TEMPLATE_I18N_PATH[key as OfficialPresetKey]}.name`);
    }
    return option.display_name || t('agentSettings.productBinding.unavailableAgent');
  };

  const change = async (value: string, applyLatest = false) => {
    const option = data?.options.find((option) => productSelectionValue(option.selection) === value);
    if (disabled || savingRef.current || !option?.available || !data || (!applyLatest && value === productSelectionValue(data.selection))) return;
    savingRef.current = true;
    setSaving(true);
    onSavingChange?.(true);
    setSaveError(undefined);
    try {
      const activeId = selectedModel ? conversationId ?? null : null;
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
      const category = productAgentErrorReason(reason);
      const message = t(`agentSettings.productBinding.${category}`);
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

  if (error) return <div role='alert'>
    {t('agentSettings.productBinding.loadFailed')}
    <Button size='mini' onClick={() => void mutate()}>{t('agentSettings.actions.retry')}</Button>
  </div>;
  if (isLoading || !data) return <Spin size={14} />;
  const selected = data.options.find((option) => productSelectionValue(option.selection) === productSelectionValue(data.selection));
  const selectedName = selected
    ? nameFor(selected)
    : t('agentSettings.productBinding.unavailableAgent');
  const canApplyLatest = targetKind === 'companion';
  const editPath = data.selection.kind === 'template'
    ? `/agent?template=${encodeURIComponent(data.selection.template_key)}`
    : `/agent?preset=${encodeURIComponent(data.selection.preset_id)}`;
  const canEdit = canApplyLatest && selected && selected.reason !== 'removed';
  const options = targetKind === 'companion'
    ? data.options.filter((option) => option.reason !== 'companion' && option.reason !== 'removed')
    : data.options;
  const sourceTag = (selection: ProductAgentSelection) => <span className={styles.sourceTag}
    data-agent-origin={selection.kind === 'template' ? 'official' : 'custom'}>
    {t(selection.kind === 'template' ? 'agentSettings.productBinding.officialTag' : 'agentSettings.productBinding.customTag')}
  </span>;
  const optionLabel = (option: ProductAgentOptions['options'][number]) => <span className={styles.optionLabel}>
    <span className={styles.name} title={nameFor(option)}>{nameFor(option)}</span>
    {sourceTag(option.selection)}
  </span>;
  const currentUnavailable = selected && !selected.available
    ? t('agentSettings.productBinding.currentUnavailable', { reason: t(`agentSettings.productBinding.reasons.${selected.reason ?? 'capability'}`) })
    : undefined;
  if (compact) return (
    <Tooltip content={currentUnavailable ?? t('agentSettings.productBinding.companionHint')}>
      <span className='inline-flex min-w-0'>
      <Dropdown trigger='click' disabled={disabled || saving} droplist={
        <Menu className={styles.menu}>
          {options.map((option) => <Menu.Item
            key={productSelectionValue(option.selection)}
            disabled={!option.available}
            aria-disabled={!option.available}
            onClick={() => void change(productSelectionValue(option.selection))}
          >
            <span className={styles.optionContent}>
              {optionLabel(option)}
              {!option.available && <small className={styles.reason}>{t(`agentSettings.productBinding.reasons.${option.reason ?? 'capability'}`)}</small>}
            </span>
          </Menu.Item>)}
          {canEdit && <Menu.Item key='edit-agent'>
            <Link to={editPath}>{t('agentSettings.productBinding.editAgent')}</Link>
          </Menu.Item>}
          {canApplyLatest && <Menu.Item key='apply-latest' disabled={!selected?.available} aria-disabled={!selected?.available}
            onClick={() => void change(productSelectionValue(data.selection), true)}>
            {t('agentSettings.productBinding.applyLatest')}
          </Menu.Item>}
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
  return <div className='flex min-w-0 max-w-full flex-col items-end gap-4px'>
    <div className='flex min-w-0 max-w-full flex-col items-end gap-2px'>
      <NomiSelect size='mini' contentFit contentMaxWidth='min(220px, 100%)'
        value={productSelectionValue(data.selection)} loading={saving} disabled={disabled || saving}
        onChange={(next: string) => void change(next)}
        dropdownMenuClassName={styles.popup}
        renderFormat={() => <span className={styles.selectedLabel}>
          <span className={styles.name} title={selectedName}>{selectedName}</span>{sourceTag(data.selection)}
        </span>}
        aria-label={t('agentSettings.productBinding.label')}>
        {options.map((option) => {
          const reason = option.reason ? t(`agentSettings.productBinding.reasons.${option.reason}`) : '';
          return <NomiSelect.Option key={productSelectionValue(option.selection)} value={productSelectionValue(option.selection)} disabled={!option.available}>
            <span title={reason} className={styles.optionContent}>
              {optionLabel(option)}
              {!option.available && <small className={styles.reason}>{reason}</small>}
            </span>
          </NomiSelect.Option>;
        })}
      </NomiSelect>
      {canApplyLatest && <div className='flex min-w-0 items-center gap-6px'>
        {canEdit && <Link to={editPath} className={styles.editLink}>{t('agentSettings.productBinding.editAgent')}</Link>}
        <Tooltip content={t('agentSettings.productBinding.applyLatestHint')}>
          <Button size='mini' type='text' className={styles.applyButton} loading={saving} disabled={disabled || saving || !selected?.available}
            onClick={() => void change(productSelectionValue(data.selection), true)}>
            {t('agentSettings.productBinding.applyLatest')}
          </Button>
        </Tooltip>
      </div>}
    </div>
    {data.needs_model && <span className={styles.hint}>{t('agentSettings.productBinding.chooseModelLater')}</span>}
    {currentUnavailable && <span role='status' className={styles.hint}>{currentUnavailable}</span>}
    {saveError && <span role='alert' className={styles.hint}>{saveError}</span>}
  </div>;
};

export default ProductAgentBindingSelect;
