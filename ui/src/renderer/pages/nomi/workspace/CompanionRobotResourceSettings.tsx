/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useRef, useState } from 'react';
import { Button, Message, Spin } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import useSWR from 'swr';
import { ipcBridge } from '@/common';
import type { TChatConversation } from '@/common/config/storage';
import type { CompanionId } from '@/common/types/ids';
import NomiSelect from '@/renderer/components/base/NomiSelect';
import { productAgentErrorReason, productSelectionValue } from '@/renderer/components/agent/ProductAgentBindingSelect';
import { resolveAgentResourceSelections } from '@/renderer/hooks/agent/agentResourceSelection';
import { isConversationBindingBlocked } from '@/renderer/pages/conversation/utils/conversationRuntime';
import styles from './CompanionRobotResourceSettings.module.css';

type Props = {
  companionId: CompanionId;
  conversation?: TChatConversation | null;
  disabled?: boolean;
  onChanged?: () => void;
  onSavingChange?: (saving: boolean) => void;
};

/** Pairing identifies a companion; only an explicit Session transition selects control resources. */
const CompanionRobotResourceSettings: React.FC<Props> = ({ companionId, conversation, disabled = false, onChanged, onSavingChange }) => {
  const { t } = useTranslation();
  const [draft, setDraft] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState('');
  const [retryDefault, setRetryDefault] = useState(false);
  const savingRef = useRef(false);
  const mounted = useRef(true);
  const requestKey = `${companionId}/${conversation?.id ?? ''}`;
  const activeRequest = useRef(requestKey);
  activeRequest.current = requestKey;
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => { setDraft(null); setSaveError(''); setRetryDefault(false); }, [requestKey]);
  const current = () => mounted.current && activeRequest.current === requestKey;
  const load = async () => {
    const [observation, options, robots] = await Promise.all([
      ipcBridge.agentPlatform.sessions.get.invoke({ agent_session_id: conversation!.id }),
      ipcBridge.agentPlatform.productBindingOptions.invoke({ target_kind: 'companion', target_id: companionId,
        model: { provider_id: conversation!.model.id, model: conversation!.model.use_model } }),
      ipcBridge.robot.list.invoke(),
    ]);
    return { binding: observation.session.agent_binding, options, robots: robots.filter((robot) => robot.companion_id === companionId) };
  };
  const { data, error, isLoading, mutate } = useSWR(conversation
    ? ['companion-robot-resources', requestKey, conversation.agent_snapshot?.canonical_binding?.binding_version ?? 0,
      conversation.model.id, conversation.model.use_model] : null, load, { shouldRetryOnError: false });
  useEffect(() => ipcBridge.robot.onStatus.on(() => { void mutate().catch(() => {}); }), [mutate]);
  const selectedBindings = data?.binding.typed_resource_bindings.filter((resource) => resource.resource_kind === 'robot') ?? [];
  const selectedIds = selectedBindings.map((resource) => resource.resource_id);
  const committed = selectedIds.length === 1 ? selectedIds[0] : '';
  const selected = draft ?? committed;
  const blocked = Boolean(conversation && isConversationBindingBlocked(conversation));
  const controlsDisabled = disabled || saving || blocked || isLoading || Boolean(error) || !data?.options.selection;
  const available = !selected || Boolean(data?.robots.some((robot) => robot.robot_id === selected));
  const currentCapabilityRevision = data?.robots.find((robot) => robot.robot_id === selected)?.capability_revision ?? 0;
  const boundCapabilityRevision = Number(selectedBindings[0]?.typed_parameters?.capability_revision ?? 0);
  const capabilitiesChanged = Boolean(selected && selected === committed && currentCapabilityRevision !== boundCapabilityRevision);
  const changed = capabilitiesChanged || (draft !== null && (selected !== committed || selectedIds.length > 1));

  const apply = async () => {
    if (!conversation || !data?.options.selection || controlsDisabled || savingRef.current || !available || (!changed && !retryDefault)) return;
    savingRef.current = true;
    setSaving(true);
    setSaveError('');
    onSavingChange?.(true);
    try {
      // Re-read the existing canonical binding so selecting one robot cannot drop
      // Knowledge, MCP, channel, workspace, or other current Session resources.
      const latest = await load();
      if (!current()) return;
      const projectedVersion = conversation.agent_snapshot?.canonical_binding?.binding_version;
      const latestCapabilityRevision = latest.robots.find((robot) => robot.robot_id === selected)?.capability_revision ?? 0;
      if (!latest.options.selection || productSelectionValue(latest.options.selection) !== productSelectionValue(data.options.selection)
        || latest.binding.binding_version !== data.binding.binding_version
        || latestCapabilityRevision !== currentCapabilityRevision
        || (projectedVersion !== undefined && projectedVersion !== latest.binding.binding_version)) {
        const message = t('nomi.cohabit.robotResource.changed');
        setSaveError(message);
        Message.error(message);
        onChanged?.();
        await mutate();
        return;
      }
      if (selected && !latest.robots.some((robot) => robot.robot_id === selected)) throw new Error('Robot is no longer paired');
      const resourceSelections = latest.binding.typed_resource_bindings
        .filter((resource) => resource.resource_kind !== 'robot')
        .map(({ resource_kind, resource_id }) => ({ resource_kind,
          // The Session owner carries the frozen project-memory namespace across
          // the transition; the selection resolver accepts its catalog identity.
          resource_id: resource_kind === 'project_memory'
            ? resolveAgentResourceSelections(['project_memory'], {}).selections[0].resource_id : resource_id }));
      if (selected) resourceSelections.push({ resource_kind: 'robot', resource_id: selected });
      await ipcBridge.agentPlatform.selectProductBinding.invoke({ target_kind: 'companion', target_id: companionId,
        request: { selection: latest.options.selection, conversation_id: conversation.id,
          model: { provider_id: conversation.model.id, model: conversation.model.use_model }, resource_selections: resourceSelections } });
      onChanged?.();
      if (!current()) return;
      setDraft(null);
      setRetryDefault(false);
      await mutate();
      if (current()) Message.success(t('nomi.cohabit.robotResource.saved'));
    } catch (reason) {
      const category = productAgentErrorReason(reason);
      if (category === 'partialSave') onChanged?.();
      if (!current()) return;
      const message = t(`agentSettings.productBinding.${category}`);
      setSaveError(message);
      setRetryDefault(category === 'partialSave');
      Message.error(message);
      void mutate().catch(() => {});
    } finally {
      savingRef.current = false;
      onSavingChange?.(false);
      if (mounted.current) setSaving(false);
    }
  };

  if (!conversation) return null;
  return <div className={styles.settings}>
    <label className={styles.label} htmlFor={`companion-robot-${companionId}`}>{t('nomi.cohabit.robotResource.label')}</label>
    {isLoading && !data ? <Spin size={14} /> : <NomiSelect id={`companion-robot-${companionId}`} size='mini' className='w-full'
      aria-label={t('nomi.cohabit.robotResource.label')} value={selected} disabled={controlsDisabled}
      onChange={(value: string) => { setDraft(value); setSaveError(''); }}>
      <NomiSelect.Option value=''>{t('nomi.cohabit.robotResource.none')}</NomiSelect.Option>
      {!available && <NomiSelect.Option value={selected} disabled>{t('nomi.cohabit.robotResource.unavailable')}</NomiSelect.Option>}
      {data?.robots.map((robot) => <NomiSelect.Option key={robot.robot_id} value={robot.robot_id}>{robot.name || robot.board || robot.robot_id}</NomiSelect.Option>)}
    </NomiSelect>}
    <p className={styles.hint}>{t('nomi.cohabit.robotResource.hint')}</p>
    {capabilitiesChanged && <p role='status' className={styles.hint}>{t('nomi.cohabit.robotResource.capabilitiesChanged')}</p>}
    {blocked && <p role='status' className={styles.hint}>{t('agentSettings.productBinding.busy')}</p>}
    {data && !data.options.selection && <p role='status' className={styles.hint}>{t('agentSettings.productBinding.selectionNeededHint')}</p>}
    {error && <div role='alert' className={styles.error}>{t('agentSettings.productBinding.loadFailed')}
      <Button size='mini' type='text' disabled={saving} onClick={() => void mutate().catch(() => {})}>{t('agentSettings.actions.retry')}</Button>
    </div>}
    {saveError && <p role='alert' className={styles.error}>{saveError}</p>}
    <Button size='mini' type='text' loading={saving} disabled={controlsDisabled || (!changed && !retryDefault) || !available} onClick={() => void apply()}>
      {t('nomi.cohabit.robotResource.apply')}
    </Button>
  </div>;
};

export default CompanionRobotResourceSettings;
