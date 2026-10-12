/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useRef, useState } from 'react';
import { Button, Message } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import type { TChatConversation } from '@/common/config/storage';
import NomiSelect from '@/renderer/components/base/NomiSelect';
import ProductAgentBindingSelect, { productAgentErrorReason } from '@/renderer/components/agent/ProductAgentBindingSelect';
import { useModelsForTask } from '@/renderer/hooks/agent/useModelsForTask';
import { useModelSelectorProviderLabel } from '@/renderer/hooks/agent/useModelSelectorProviderLabel';
import { modelDisplayLabel } from '@/common/utils/modelPresentation';
import { getConversationRuntimeAuthority } from '@/renderer/pages/conversation/utils/conversationRuntime';
import { saveCompanionChatModel, type CompanionChatModel } from '../companion/saveCompanionChatModel';
import type { CompanionHandle } from './types';
import styles from './CompanionAgentModelQuickSettings.module.css';

type Props = {
  companion: CompanionHandle;
  conversation?: TChatConversation | null;
  disabled?: boolean;
  className?: string;
  onChanged?: () => void;
  onSavingChange?: (saving: boolean) => void;
};

const valueFor = (model: CompanionChatModel) => JSON.stringify([model.provider_id, model.model]);

/** Quick editing stays inside the docked partner sidebar and the existing Session. */
const CompanionAgentModelQuickSettings: React.FC<Props> = ({ companion, conversation, disabled = false, className, onChanged, onSavingChange }) => {
  const { t } = useTranslation();
  const { profile, patchCompanion } = companion;
  const { groups, isLoading, error: catalogError, refresh: refreshCatalog } = useModelsForTask('chat');
  const providerLabel = useModelSelectorProviderLabel();
  const [modelSaving, setModelSaving] = useState(false);
  const [agentSaving, setAgentSaving] = useState(false);
  const [saveError, setSaveError] = useState('');
  const [retryModel, setRetryModel] = useState<CompanionChatModel | null>(null);
  const [committedModel, setCommittedModel] = useState<CompanionChatModel | null>(null);
  const busyRef = useRef(false);
  const committedRef = useRef<CompanionChatModel | null>(null);
  const mounted = useRef(true);
  const requestKey = `${profile?.companion_id ?? ''}/${conversation?.id ?? ''}`;
  const activeRequest = useRef(requestKey);
  activeRequest.current = requestKey;
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => {
    const model = conversation ? { provider_id: conversation.model.id, model: conversation.model.use_model } : null;
    committedRef.current = model;
    setCommittedModel(model);
  }, [conversation?.id, conversation?.model.id, conversation?.model.use_model]);
  useEffect(() => { setSaveError(''); setRetryModel(null); }, [requestKey]);
  const current = () => mounted.current && activeRequest.current === requestKey;
  const busy = modelSaving || agentSaving;
  const sessionBlocked = Boolean(conversation && (
    getConversationRuntimeAuthority(conversation) !== 'idle' || conversation.runtime?.active_turn_id ||
    conversation.extra.execution_phase === 'paused' || conversation.execution_attempt_id
  ));
  const controlsDisabled = disabled || busy || sessionBlocked || Boolean(profile?.model && !conversation);
  const model = committedModel ?? (conversation ? { provider_id: conversation.model.id, model: conversation.model.use_model } : profile?.model);
  const eligibleGroups = groups.filter(({ provider }) => !provider.platform.toLowerCase().includes('gemini-with-google-auth'));
  const modelAvailable = Boolean(model && eligibleGroups.some((group) => group.provider.id === model.provider_id && group.models.includes(model.model)));

  const saveModel = async (next: CompanionChatModel) => {
    if (!profile || busyRef.current || disabled || sessionBlocked || Boolean(profile.model && !conversation)) return;
    if (!eligibleGroups.some((group) => group.provider.id === next.provider_id && group.models.includes(next.model))) return;
    busyRef.current = true;
    setModelSaving(true);
    onSavingChange?.(true);
    setSaveError('');
    try {
      const result = await saveCompanionChatModel({
        conversation,
        sessionModel: committedRef.current ?? (conversation ? { provider_id: conversation.model.id, model: conversation.model.use_model } : undefined),
        profileModel: profile.model,
        next,
        patchCompanion,
        onSessionChanged: (selected) => {
          onChanged?.();
          if (!current()) return;
          committedRef.current = selected;
          setCommittedModel(selected);
        },
      });
      if (!current()) return;
      if (result.status === 'default-save-failed') {
        const message = t('nomi.cohabit.modelDefaultSaveFailed', { defaultValue: '当前会话模型已切换，默认模型保存失败，请重试。' });
        setRetryModel(next);
        setSaveError(message);
        Message.error(message);
      } else {
        setRetryModel(null);
        Message.success(t('agent.model.switchSuccess'));
        onChanged?.();
      }
    } catch (reason) {
      if (!current()) return;
      const message = t(`agentSettings.productBinding.${productAgentErrorReason(reason)}`);
      setSaveError(message);
      Message.error(message);
    } finally {
      busyRef.current = false;
      onSavingChange?.(false);
      if (mounted.current) setModelSaving(false);
    }
  };

  if (!profile) return null;
  return <div className={[styles.fields, className].filter(Boolean).join(' ')}>
    <div className={styles.row}>
      <span className={styles.label}>{t('nomi.cohabit.quickAgent', { defaultValue: 'Agent' })}</span>
      <div className={styles.control}>
        <ProductAgentBindingSelect targetKind='companion' targetId={profile.companion_id} defaultTemplateKey='companion.default'
          model={model ? { id: model.provider_id, use_model: model.model } : undefined} conversationId={conversation?.id}
          layout='stacked' showManagementActions={false} disabled={controlsDisabled}
          onChanged={onChanged} onSavingChange={(saving) => { busyRef.current = saving; if (mounted.current) setAgentSaving(saving); onSavingChange?.(saving); }} />
      </div>
    </div>
    <div className={styles.row}>
      <span className={styles.label}>{t('nomi.cohabit.quickModel', { defaultValue: '模型' })}</span>
      <div className={styles.control}>
        <NomiSelect size='mini' className='w-full' value={model ? valueFor(model) : undefined} loading={modelSaving || isLoading}
          aria-label={t('nomi.cohabit.quickModel', { defaultValue: '模型' })}
          placeholder={t('nomi.cohabit.quickModelPlaceholder', { defaultValue: '选择模型' })}
          disabled={controlsDisabled || isLoading || Boolean(catalogError)} showSearch dropdownMenuClassName={styles.popup}
          onChange={(value: string) => {
            const selected = eligibleGroups.flatMap((group) => group.models.map((name) => ({ provider_id: group.provider.id, model: name })))
              .find((candidate) => valueFor(candidate) === value);
            if (selected) void saveModel(selected);
          }}>
          {model && !modelAvailable && <NomiSelect.Option value={valueFor(model)} disabled>
            {isLoading || catalogError ? modelDisplayLabel(model.model) : t('settings.taskModel.unavailableOption', { model: model.model })}
          </NomiSelect.Option>}
          {eligibleGroups.map((group) => <NomiSelect.OptGroup key={group.provider.id} label={providerLabel(group.provider)}>
            {group.models.map((name) => <NomiSelect.Option key={valueFor({ provider_id: group.provider.id, model: name })} value={valueFor({ provider_id: group.provider.id, model: name })}>
              {modelDisplayLabel(name, group.provider.models.find((row) => row.model === name)?.display_name)}
            </NomiSelect.Option>)}
          </NomiSelect.OptGroup>)}
        </NomiSelect>
      </div>
    </div>
    {sessionBlocked && <p role='status' className={styles.hint}>{t('agentSettings.productBinding.busy')}</p>}
    {catalogError && <div role='alert' className={`${styles.hint} ${styles.error}`}>
      {t('agentSettings.productBinding.loadFailed')} <Button size='mini' type='text' onClick={refreshCatalog}>{t('agentSettings.actions.retry')}</Button>
    </div>}
    {!isLoading && !catalogError && eligibleGroups.length === 0 && <p className={styles.hint}>{t('nomi.chat.modelNoTextModel')}</p>}
    {saveError && <p role='alert' className={`${styles.hint} ${styles.error}`}>{saveError}</p>}
    {retryModel && <Button size='mini' type='text' disabled={controlsDisabled || Boolean(catalogError)} loading={modelSaving}
      onClick={() => void saveModel(retryModel)}>{t('agentSettings.actions.retry')}</Button>}
  </div>;
};

export default CompanionAgentModelQuickSettings;
