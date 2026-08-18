/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useState, useMemo, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { Form, Input, Select, Message, TimePicker, Radio, Switch } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import NomiModal from '@renderer/components/base/NomiModal';
import type { ICreateCronJobParams, ICronAgentConfig, ICronJob } from '@/common/adapter/ipcBridge';
import { useConversationAgents } from '@renderer/pages/conversation/hooks/useConversationAgents';
import { presetSupportsTarget } from '@/common/types/agent/presetTypes';
import { resolvePresetCatalogName } from '@renderer/utils/model/presetPresentation';
import dayjs from 'dayjs';
import { getFullAutoMode } from '@renderer/utils/model/agentModes';
import type { TProviderWithModel } from '@/common/config/storage';
import type { ConversationId, ProviderId } from '@/common/types/ids';
import { useModelsForTask } from '@renderer/hooks/agent/useModelsForTask';
import GuidModelSelector from '@renderer/pages/guid/components/GuidModelSelector';
import { WorkspaceFolderSelect } from '@renderer/components/workspace';
import type { AgentMetadata } from '@renderer/utils/model/agentTypes';
import { createCronSchedule, getCurrentCronTimeZone } from '@renderer/pages/cron/cronUtils';
import { useAllCronJobs } from '@renderer/pages/cron/useCronJobs';
import { getConversationCreateErrorMessage } from '@renderer/pages/conversation/utils/conversationCreateError';
import CronExpressionBuilder, { validateCronExpression } from './CronExpressionBuilder';
import { useConversationListSync } from '@renderer/pages/conversation/SessionList/hooks/useConversationListSync';
import { getBackendKeyFromConversation } from '@renderer/pages/conversation/SessionList/utils/exportHelpers';
import { renderConversationOption } from '@renderer/pages/conversation/components/renderConversationOption';
import { shortSessionId } from '@renderer/utils/ui/shortId';
import {
  buildCronConversationRequestFields,
  resolveCronConversationTarget,
  type ConversationExecutionMode,
} from './cronConversationTarget';
import {
  CronAgentOptionIdentity,
  CronPresetOptionIdentity,
  CronUnavailableAgentIdentity,
} from './CronAgentOptionIdentity';
import {
  findCronSelectedAgent,
  getCronAgentOptionValue,
  getCronAgentSelectionFromJob,
  getCronPresetOptionValue,
  hasCronAgentConfigurationChanged,
  parseCronAgentSelection,
  resolveCronAgentDisplayName,
} from './cronAgentSelection';

const FormItem = Form.Item;
const TextArea = Input.TextArea;
const Option = Select.Option;
const OptGroup = Select.OptGroup;

interface CreateTaskDialogProps {
  visible: boolean;
  onClose: () => void;
  /** When provided, the dialog operates in edit mode */
  editJob?: ICronJob;
  /** Preset the specified conversation target on open (create mode only). */
  initialSpecifiedConversationId?: ConversationId;
  /** Prevent changing the preset target fields while still allowing task details to be edited. */
  lockInitialTarget?: boolean;
}

type FrequencyType = 'manual' | 'hourly' | 'daily' | 'weekdays' | 'weekly' | 'custom';
// UI-level execution mode. 'specified' is a frontend affordance that maps to the
// backend `existing` mode bound to a user-picked conversation_id.
const WEEKDAYS = [
  { value: 'MON', label: 'monday' },
  { value: 'TUE', label: 'tuesday' },
  { value: 'WED', label: 'wednesday' },
  { value: 'THU', label: 'thursday' },
  { value: 'FRI', label: 'friday' },
  { value: 'SAT', label: 'saturday' },
  { value: 'SUN', label: 'sunday' },
];

/**
 * Infer frequency type and time/weekday from a 5- or 6-field cron expression
 * for edit mode. Returns 'custom' for non-preset (incl. sub-minute) schedules.
 */
function parseCronExpr(expr: string): { frequency: FrequencyType; time: string; weekday: string } {
  if (!expr) return { frequency: 'manual', time: '09:00', weekday: 'MON' };

  let parts = expr.trim().split(/\s+/);
  if (parts.length === 5) parts = ['0', ...parts];
  if (parts.length < 6) return { frequency: 'daily', time: '09:00', weekday: 'MON' };

  const [seconds, min, hour, dayRaw, month, dowRaw] = parts;
  if (seconds !== '0') return { frequency: 'custom', time: '09:00', weekday: 'MON' };
  const day = dayRaw === '?' ? '*' : dayRaw;
  const dow = dowRaw === '?' ? '*' : dowRaw;

  if (hour === '*' && min === '0' && day === '*' && month === '*' && dow === '*') {
    return { frequency: 'hourly', time: '09:00', weekday: 'MON' };
  }
  if (dow === 'MON-FRI' && day === '*' && month === '*') {
    const hh = String(hour).padStart(2, '0');
    const mm = String(min).padStart(2, '0');
    return { frequency: 'weekdays', time: `${hh}:${mm}`, weekday: 'MON' };
  }
  if (dow !== '*' && day === '*' && month === '*') {
    const dayUpper = dow.toUpperCase();
    const matched = WEEKDAYS.find((d) => d.value === dayUpper);
    if (matched) {
      const hh = String(hour).padStart(2, '0');
      const mm = String(min).padStart(2, '0');
      return { frequency: 'weekly', time: `${hh}:${mm}`, weekday: dayUpper };
    }
    return { frequency: 'daily', time: '09:00', weekday: 'MON' };
  }
  if (day === '*' && month === '*' && dow === '*') {
    const hourNum = Number(hour);
    const minNum = Number(min);
    if (!isNaN(hourNum) && !isNaN(minNum) && hourNum >= 0 && hourNum <= 23 && minNum >= 0 && minNum <= 59) {
      const hh = String(hourNum).padStart(2, '0');
      const mm = String(minNum).padStart(2, '0');
      return { frequency: 'daily', time: `${hh}:${mm}`, weekday: 'MON' };
    }
  }

  return { frequency: 'custom', time: '09:00', weekday: 'MON' };
}

function getDescriptionInitialValue(job: ICronJob): string {
  return job.description?.trim() ?? '';
}

const CreateTaskDialog: React.FC<CreateTaskDialogProps> = ({
  visible,
  onClose,
  editJob,
  initialSpecifiedConversationId,
  lockInitialTarget = false,
}) => {
  const { t, i18n } = useTranslation();
  const [form] = Form.useForm();
  const [submitting, setSubmitting] = useState(false);
  const { cliAgents, presets: presetPresets, isLoading: identitiesLoading } = useConversationAgents();
  // Provider/model groups with an exact enabled Chat capability.
  const { groups: chatGroups } = useModelsForTask('chat');
  const [frequency, setFrequency] = useState<FrequencyType>('manual');
  const [time, setTime] = useState('09:00');
  const [weekday, setWeekday] = useState('MON');
  const [customCronExpr, setCustomCronExpr] = useState<string>('');

  const isEditMode = !!editJob;
  const [execution_mode, setExecutionMode] = useState<ConversationExecutionMode>('new_conversation');
  const [specifiedConversationId, setSpecifiedConversationId] = useState<ConversationId | undefined>(undefined);
  // When reusing an existing conversation, optionally clear the agent context
  // before each run so accumulated history does not pile up across ticks.
  const [clearContextEachRun, setClearContextEachRun] = useState(false);

  // Existing conversations (for the "指定会话 / reuse a session" execution mode).
  const { conversations } = useConversationListSync();

  // All cron jobs — drives the "already-bound conversations are hidden"
  // filtering on the specified-conversation picker below.
  const { jobs: allCronJobs } = useAllCronJobs();

  // ── Bound-conversation filtering ─────────────────────────────────────────
  // A conversation already bound by ANY cron job (paused or not) is hidden
  // from the picker. The task being edited is excluded from the bound set, and
  // the currently-selected value is always kept visible.
  const boundConversationIds = useMemo(() => {
    const set = new Set<ConversationId>();
    for (const job of allCronJobs) {
      if (editJob && job.cron_job_id === editJob.cron_job_id) continue;
      // Only 'existing' execution reuses metadata.conversation_id as its bound
      // target. (new_conversation jobs merely anchor there for UI grouping and
      // spawn a fresh conversation each run — not a reuse bind, so don't hide it.)
      if (job.execution_mode === 'existing' && job.metadata.conversation_id) {
        set.add(job.metadata.conversation_id);
      }
    }
    return set;
  }, [allCronJobs, editJob]);

  const visibleConversations = useMemo(
    () => conversations.filter((c) => !boundConversationIds.has(c.id) || c.id === specifiedConversationId),
    [conversations, boundConversationIds, specifiedConversationId]
  );

  // Distinguish "nothing to pick" from "everything is already bound elsewhere".
  const conversationEmptyText =
    conversations.length > 0 && visibleConversations.length === 0
      ? t('cron.page.form.allConversationsBound', { defaultValue: '所有会话已被其它定时任务绑定' })
      : t('cron.page.form.noConversations', { defaultValue: '暂无可用会话' });

  // Agent settings
  const [model, setModelId] = useState<string | undefined>(undefined);
  const [providerId, setProviderId] = useState<ProviderId | undefined>(undefined);
  const [config_options, setConfigOptions] = useState<Record<string, string> | undefined>(undefined);
  const [workspace, setWorkspace] = useState<string | undefined>(undefined);
  const [selectedAgent, setSelectedAgent] = useState<string | undefined>(undefined);

  const removedPresetId = useMemo(() => {
    const presetId = editJob?.metadata.agent_config?.preset_id;
    if (
      !presetId ||
      identitiesLoading ||
      presetPresets.some((preset) => preset.preset_id === presetId)
    ) {
      return undefined;
    }
    return presetId;
  }, [editJob?.metadata.agent_config?.preset_id, identitiesLoading, presetPresets]);

  const removedAgentId = useMemo(() => {
    const config = editJob?.metadata.agent_config;
    const agentId = config?.preset_id ? undefined : config?.custom_agent_id;
    if (!agentId || identitiesLoading || cliAgents.some((agent) => agent.agent_id === agentId)) return undefined;
    return agentId;
  }, [identitiesLoading, cliAgents, editJob?.metadata.agent_config]);

  // Populate form when entering edit mode
  useEffect(() => {
    if (!visible) return;
    if (editJob) {
      const cronExpr = editJob.schedule.kind === 'cron' ? editJob.schedule.expr : '';
      const parsed = parseCronExpr(cronExpr);
      setFrequency(parsed.frequency);
      setTime(parsed.time);
      setWeekday(parsed.weekday);
      setCustomCronExpr(parsed.frequency === 'custom' ? cronExpr : '');

      setExecutionMode(editJob.execution_mode);
      setSpecifiedConversationId(undefined);
      const agentKey = getCronAgentSelectionFromJob(editJob, cliAgents);
      setSelectedAgent(agentKey);
      form.setFieldsValue({
        name: editJob.name,
        description: getDescriptionInitialValue(editJob),
        prompt: editJob.message,
        agent: agentKey,
      });
      setModelId(editJob.metadata.agent_config?.model);
      setProviderId(editJob.metadata.agent_config?.provider_id);
      setConfigOptions(editJob.metadata.agent_config?.config_options);
      setWorkspace(editJob.metadata.agent_config?.workspace);
      setClearContextEachRun(editJob.metadata.agent_config?.clear_context_each_run ?? false);
    } else {
      form.resetFields();
      setFrequency('manual');
      setTime('09:00');
      setWeekday('MON');
      setCustomCronExpr('');
      setExecutionMode(initialSpecifiedConversationId ? 'specified' : 'new_conversation');
      setSpecifiedConversationId(initialSpecifiedConversationId);
      setModelId(undefined);
      setProviderId(undefined);
      setConfigOptions(undefined);
      setWorkspace(undefined);
      setSelectedAgent(undefined);
      setClearContextEachRun(false);
    }
  }, [visible, editJob, form, initialSpecifiedConversationId]);

  // Legacy rows do not carry custom_agent_id, so their unique backend fallback
  // can only be restored after AgentRegistry metadata has arrived.
  useEffect(() => {
    if (!visible || !editJob) return;
    const agentKey = getCronAgentSelectionFromJob(editJob, cliAgents);
    if (!agentKey || agentKey === selectedAgent) return;
    if (selectedAgent && parseCronAgentSelection(selectedAgent)?.kind !== 'legacy') return;
    setSelectedAgent(agentKey);
    form.setFieldValue('agent', agentKey);
  }, [visible, editJob, cliAgents, selectedAgent, form]);

  const selectedRuntimeAgent = useMemo<AgentMetadata | undefined>(() => {
    if (!selectedAgent) return undefined;
    const selection = parseCronAgentSelection(selectedAgent);
    if (selection?.kind === 'preset') {
      const preset = presetPresets.find((item) => item.preset_id === selection.id);
      const preferredAgentId = preset?.preferred_agent_id || preset?.agent_preferences[0]?.agent_id;
      return cliAgents.find((agent) => agent.agent_id === preferredAgentId);
    }
    return findCronSelectedAgent(selectedAgent, cliAgents);
  }, [selectedAgent, presetPresets, cliAgents]);

  const resolvedBackend = selectedRuntimeAgent?.backend || selectedRuntimeAgent?.agent_type;
  const isPresetSelection = parseCronAgentSelection(selectedAgent)?.kind === 'preset';

  const isProviderModelMode = resolvedBackend === 'nomi';

  const nomiGroups = useMemo(
    () => chatGroups.filter((g) => !g.provider.platform?.toLowerCase().includes('gemini-with-google-auth')),
    [chatGroups]
  );
  const hasNomiProvider = nomiGroups.length > 0;

  const filteredGroups = useMemo(
    () => (resolvedBackend === 'nomi' ? nomiGroups : chatGroups),
    [resolvedBackend, chatGroups, nomiGroups]
  );
  const filteredProviders = useMemo(() => filteredGroups.map((g) => g.provider), [filteredGroups]);

  const geminiCurrentModel = useMemo<TProviderWithModel | undefined>(() => {
    if (resolvedBackend !== 'nomi' || !model) return undefined;
    if (providerId) {
      const byId = filteredGroups.find((g) => g.provider.id === providerId);
      if (byId && byId.models.includes(model)) {
        return { ...byId.provider, use_model: model } as TProviderWithModel;
      }
    }
    for (const g of filteredGroups) {
      if (g.models.includes(model)) {
        return { ...g.provider, use_model: model } as TProviderWithModel;
      }
    }
    return undefined;
  }, [resolvedBackend, model, providerId, filteredGroups]);

  const handleGeminiModelSelect = useCallback(async (selection: TProviderWithModel) => {
    setProviderId(selection.id);
    setModelId(selection.use_model);
  }, []);

  useEffect(() => {
    if (isPresetSelection || resolvedBackend !== 'nomi' || model) return;
    const firstGroup = nomiGroups[0];
    if (firstGroup && firstGroup.models.length > 0) {
      setProviderId(firstGroup.provider.id);
      setModelId(firstGroup.models[0]);
    }
  }, [isPresetSelection, resolvedBackend, model, nomiGroups]);

  // 指定会话：复用一个已存在的会话。该会话的执行 Agent 与项目（workspace）在创建时
  // 已固化，因此这里不再展示 / 不再要求配置这两项（仅新建模式下可选此模式）。
  const isSpecifiedMode = execution_mode === 'specified';
  const showTimePicker = frequency === 'daily' || frequency === 'weekdays' || frequency === 'weekly';
  const showWeekdayPicker = frequency === 'weekly';

  // Build a 6-field (seconds-first) cron expression from frequency settings.
  const scheduleInfo = useMemo(() => {
    const [hour, minute] = time.split(':').map(Number);
    switch (frequency) {
      case 'manual':
        return { expr: '', description: t('cron.page.scheduleDesc.manual') };
      case 'hourly':
        return { expr: '0 0 * * * ?', description: t('cron.page.scheduleDesc.hourly') };
      case 'daily':
        return { expr: `0 ${minute} ${hour} * * ?`, description: t('cron.page.scheduleDesc.dailyAt', { time }) };
      case 'weekdays':
        return {
          expr: `0 ${minute} ${hour} ? * MON-FRI`,
          description: t('cron.page.scheduleDesc.weekdaysAt', { time }),
        };
      case 'weekly': {
        const dayLabel = WEEKDAYS.find((d) => d.value === weekday)?.label ?? weekday;
        return {
          expr: `0 ${minute} ${hour} ? * ${weekday}`,
          description: t('cron.page.scheduleDesc.weeklyAt', { day: t(`cron.page.weekday.${dayLabel}`), time }),
        };
      }
      case 'custom':
        return { expr: customCronExpr, description: editJob?.schedule.description || customCronExpr };
      default:
        return { expr: '', description: '' };
    }
  }, [frequency, time, weekday, t, customCronExpr, editJob]);

  const conversationModeOptions = useMemo(() => {
    const options: { value: ConversationExecutionMode; label: string; description: string }[] = [
      {
        value: 'new_conversation',
        label: t('cron.page.form.newConversation'),
        description: t('cron.detail.executionModeDescriptionNew'),
      },
      {
        value: 'existing',
        label: t('cron.page.form.existingConversation'),
        description: t('cron.detail.executionModeDescriptionExisting'),
      },
    ];
    if (!isEditMode) {
      options.push({
        value: 'specified',
        label: t('cron.page.form.specifiedConversation'),
        description: t('cron.detail.executionModeDescriptionSpecified'),
      });
    }
    return options;
  }, [t, isEditMode]);

  const selectedModeDescription = (
    conversationModeOptions.find((o) => o.value === execution_mode) ?? conversationModeOptions[0]
  ).description;

  const showModelSelector = Boolean(!isPresetSelection && resolvedBackend && isProviderModelMode);

  const handleFrequencyChange = (value: FrequencyType) => {
    setFrequency(value);
    if (value === 'custom') {
      setCustomCronExpr((prev) => prev || '0 0 9 * * ?');
    } else {
      setCustomCronExpr('');
    }
  };

  const handleAgentChange = useCallback((value: string) => {
    setSelectedAgent(value);
    setModelId(undefined);
    setProviderId(undefined);
    setConfigOptions(undefined);
  }, []);

  const handleWorkspaceClear = useCallback(() => {
    setWorkspace(undefined);
  }, []);

  const resolveAgentConfig = (agentValue: string) => {
    const selection = parseCronAgentSelection(agentValue);
    if (!selection) throw new Error(t('cron.page.form.agentRequired'));

    let agent_config: ICronAgentConfig | undefined;
    let resolvedAgentType: ICreateCronJobParams['agent_type'] = 'claude';
    const shouldClearContextEachRun = execution_mode === 'existing' && clearContextEachRun;

    if (selection.kind === 'agent' || selection.kind === 'legacy') {
      const agent = findCronSelectedAgent(agentValue, cliAgents);
      if (!agent) throw new Error(t('cron.page.form.removedAgentRequired'));
      const backend = agent.backend || agent.agent_type;

      if (agent.agent_type === 'nomi' || backend === 'nomi') {
        if (!providerId || !geminiCurrentModel || !model) {
          throw new Error(t('cron.page.form.nomiModelRequired'));
        }
        resolvedAgentType = 'nomi';
        agent_config = {
          provider_id: providerId,
          name: geminiCurrentModel.name,
          mode: getFullAutoMode('nomi'),
          model,
          workspace,
          clear_context_each_run: shouldClearContextEachRun,
        };
      } else {
        resolvedAgentType = agent.agent_type;
        agent_config = {
          ...(agent.backend ? { backend: agent.backend } : {}),
          custom_agent_id: agent.agent_id,
          name: resolveCronAgentDisplayName(agent, i18n.language),
          workspace,
          clear_context_each_run: shouldClearContextEachRun,
        };
      }
    } else if (selection.kind === 'preset') {
      const preset = presetPresets.find((item) => item.preset_id === selection.id);
      if (!preset) {
        throw new Error(t('cron.page.form.removedPresetRequired'));
      }
      if (!presetSupportsTarget(preset, 'cron')) {
        throw new Error(
          t('cron.page.form.presetCronRequired', {
            name: resolvePresetCatalogName(preset, i18n.language),
          })
        );
      }
      const preferredAgentId = preset.preferred_agent_id || preset.agent_preferences[0]?.agent_id;
      const preferredAgent = cliAgents.find((agent) => agent.agent_id === preferredAgentId);
      const presetBackend = preferredAgent?.backend || preferredAgent?.agent_type || 'nomi';
      resolvedAgentType = preferredAgent?.agent_type || presetBackend;
      agent_config = {
        ...(presetBackend === 'nomi' ? {} : { backend: presetBackend }),
        // The backend freezes canonical preset.name in the resolved snapshot.
        // Localization is presentation-only and must not leak into persisted identity.
        name: preset.name,
        preset_id: preset.preset_id,
        workspace,
        clear_context_each_run: shouldClearContextEachRun,
      };
    }

    return { agent_config, resolvedAgentType };
  };

  const handleSubmit = async () => {
    try {
      const values = await form.validate();

      if (frequency !== 'manual' && !validateCronExpression(scheduleInfo.expr, getCurrentCronTimeZone()).valid) {
        Message.error(t('cron.page.cronExpression.invalid'));
        return;
      }

      const schedule = createCronSchedule(scheduleInfo.expr, scheduleInfo.description);
      const conversationTarget = resolveCronConversationTarget(execution_mode, specifiedConversationId);

      if (!conversationTarget) {
        Message.error(t('cron.page.form.specifiedConversationRequired'));
        return;
      }

      // ─── 指定会话 — 复用已存在的会话 ─────────────────────────────────
      // 复用的会话已经带有自己的执行 Agent 和项目（workspace），这里不再重复配置，
      // 也绝不能传 agent_config：否则 agent_config.workspace 会覆盖会话自身的工作目录
      // （见 nomifun-cron executor::resolve_execution_workspace_raw）。
      // 指定会话仅在新建模式提供，因此直接构造创建参数并返回。
      if (conversationTarget.kind === 'specified') {
        const specifiedConversationId = conversationTarget.conversationId;
        // Guard against reusing a conversation already bound by another task
        // (the picker hides bound targets, but a stale value can slip through).
        if (boundConversationIds.has(specifiedConversationId)) {
          Message.error(t('cron.page.form.conversationAlreadyBound', { defaultValue: '该会话已被其它定时任务绑定，请另选一个' }));
          return;
        }
        const selectedConversation = conversations.find((c) => c.id === specifiedConversationId);
        const specifiedAgentType =
          (selectedConversation && getBackendKeyFromConversation(selectedConversation)) || 'claude';

        setSubmitting(true);
        const params: ICreateCronJobParams = {
          name: values.name,
          description: values.description,
          schedule,
          prompt: values.prompt,
          conversation_title: selectedConversation?.name,
          agent_type: specifiedAgentType,
          created_by: 'user',
          ...buildCronConversationRequestFields(conversationTarget),
        };
        await ipcBridge.cron.addJob.invoke(params);
        Message.success(t('cron.page.createSuccess'));
        onClose();
        return;
      }

      // ─── Agent / conversation target (new_conversation / existing) ───
      // Both modes intentionally start unbound; the backend materializes their
      // first conversation and only the continuing mode reuses it later.
      const agentValue = (values.agent as string | undefined) || selectedAgent;
      if (!agentValue) throw new Error(t('cron.page.form.agentRequired'));
      setSubmitting(true);

      if (isEditMode) {
        // The backend does not support changing agent_type on update. It also
        // re-resolves any submitted preset_id, so omit an unchanged config to
        // preserve the task's frozen preset revision/snapshot.
        const agentConfigChanged = hasCronAgentConfigurationChanged(editJob!, cliAgents, {
          selection: agentValue,
          model,
          providerId,
          configOptions: config_options,
          workspace,
          clearContextEachRun,
        });
        const agent_config = agentConfigChanged ? resolveAgentConfig(agentValue).agent_config : undefined;
        await ipcBridge.cron.updateJob.invoke({
          cron_job_id: editJob!.cron_job_id,
          updates: {
            name: values.name,
            description: values.description,
            schedule,
            message: values.prompt,
            ...(agentConfigChanged ? { agent_config } : {}),
          },
        });
        Message.success(t('cron.page.updateSuccess'));
      } else {
        const { agent_config, resolvedAgentType } = resolveAgentConfig(agentValue);
        const params: ICreateCronJobParams = {
          name: values.name,
          description: values.description,
          schedule,
          prompt: values.prompt,
          agent_type: resolvedAgentType,
          created_by: 'user',
          agent_config,
          ...buildCronConversationRequestFields(conversationTarget),
        };
        await ipcBridge.cron.addJob.invoke(params);
        Message.success(t('cron.page.createSuccess'));
      }

      onClose();
    } catch (err) {
      Message.error(getConversationCreateErrorMessage(err, t));
    } finally {
      setSubmitting(false);
    }
  };

  const selectedIdentity = parseCronAgentSelection(selectedAgent);
  const unavailableLegacyAgentValue =
    selectedAgent &&
    selectedIdentity?.kind === 'legacy' &&
    !findCronSelectedAgent(selectedAgent, cliAgents)
      ? selectedAgent
      : undefined;
  const selectedPreset =
    selectedIdentity?.kind === 'preset'
      ? presetPresets.find((preset) => preset.preset_id === selectedIdentity.id)
      : undefined;
  const selectedIdentityStatus =
    selectedIdentity?.kind === 'agent' && removedAgentId === selectedIdentity.id
      ? t('cron.page.form.removedAgentUnavailable')
      : unavailableLegacyAgentValue && !identitiesLoading
        ? t('cron.page.form.legacyAgentUnavailable')
        : selectedIdentity?.kind === 'preset' && removedPresetId === selectedIdentity.id
          ? t('cron.page.form.removedPresetUnavailable')
          : selectedPreset && !presetSupportsTarget(selectedPreset, 'cron')
            ? t('cron.page.form.presetCronUnavailable')
            : undefined;

  // The agent selector is reused in two layouts (alone, or sharing a row with
  // the model selector), so build it once.
  const agentFormItem = (
    <FormItem
      label={t('cron.page.form.agent')}
      field='agent'
      rules={[{ required: true, message: t('cron.page.form.agentRequired') }]}
      extra={
        isEditMode ? (
          <span className='flex flex-col gap-2px text-12px text-t-tertiary'>
            <span>{t('cron.page.form.agentEditHint')}</span>
            {selectedIdentityStatus && <span>{selectedIdentityStatus}</span>}
          </span>
        ) : undefined
      }
    >
      <Select
        placeholder={t('cron.page.form.agentPlaceholder')}
        onChange={handleAgentChange}
        disabled={isEditMode}
        renderFormat={(_option, value) => {
          const strVal = value as unknown as string;
          if (!strVal) return '';
          const selection = parseCronAgentSelection(strVal);
          if (selection?.kind === 'agent' || selection?.kind === 'legacy') {
            const agent = findCronSelectedAgent(strVal, cliAgents);
            if (agent) return <CronAgentOptionIdentity agent={agent} language={i18n.language} compact />;
            if (editJob?.metadata.agent_config?.custom_agent_id === selection.id) {
              return (
                <CronUnavailableAgentIdentity
                  name={editJob.metadata.agent_config.name}
                  statusLabel={removedAgentId === selection.id ? t('cron.page.form.removedAgentUnavailable') : undefined}
                  compact
                />
              );
            }
            if (selection.kind === 'legacy' && editJob && selectedAgent === strVal) {
              return (
                <CronUnavailableAgentIdentity
                  name={editJob.metadata.agent_config?.name || selection.id}
                  statusLabel={identitiesLoading ? undefined : t('cron.page.form.legacyAgentUnavailable')}
                  compact
                />
              );
            }
          } else if (selection?.kind === 'preset') {
            const preset = presetPresets.find((item) => item.preset_id === selection.id);
            if (preset) {
              const supportsCron = presetSupportsTarget(preset, 'cron');
              const frozenName =
                editJob?.metadata.agent_config?.preset_id === preset.preset_id
                  ? editJob.metadata.agent_config.name
                  : undefined;
              return (
                <CronPresetOptionIdentity
                  preset={preset}
                  language={i18n.language}
                  nameOverride={frozenName}
                  statusLabel={supportsCron ? undefined : t('cron.page.form.presetCronUnavailable')}
                  compact
                />
              );
            }
            if (removedPresetId === selection.id) {
              return (
                <CronUnavailableAgentIdentity
                  name={editJob?.metadata.agent_config?.name || t('cron.page.form.removedPresetLabel', { id: selection.id })}
                  statusLabel={t('cron.page.form.removedPresetUnavailable')}
                  compact
                />
              );
            }
          }
          return <CronUnavailableAgentIdentity name={t('cron.page.form.unknownAgentLabel')} compact />;
        }}
      >
        {(cliAgents.length > 0 || removedAgentId || unavailableLegacyAgentValue) && (
          <OptGroup label={t('conversation.dropdown.cliAgents')}>
            {unavailableLegacyAgentValue && (
              <Option
                key={unavailableLegacyAgentValue}
                value={unavailableLegacyAgentValue}
                disabled
                aria-disabled='true'
              >
                <CronUnavailableAgentIdentity
                  name={editJob?.metadata.agent_config?.name || selectedIdentity?.id || t('cron.page.form.unknownAgentLabel')}
                  statusLabel={identitiesLoading ? undefined : t('cron.page.form.legacyAgentUnavailable')}
                />
              </Option>
            )}
            {removedAgentId && (
              <Option
                key={getCronAgentOptionValue(removedAgentId)}
                value={getCronAgentOptionValue(removedAgentId)}
                disabled
                aria-disabled='true'
              >
                <CronUnavailableAgentIdentity
                  name={editJob?.metadata.agent_config?.name || t('cron.page.form.unknownAgentLabel')}
                  statusLabel={t('cron.page.form.removedAgentUnavailable')}
                />
              </Option>
            )}
            {cliAgents.map((agent) => {
              const optionValue = getCronAgentOptionValue(agent.agent_id);
              const disabled = agent.agent_type === 'nomi' && !hasNomiProvider;
              return (
                <Option key={optionValue} value={optionValue} disabled={disabled} aria-disabled={disabled || undefined}>
                  <CronAgentOptionIdentity
                    agent={agent}
                    language={i18n.language}
                    statusLabel={disabled ? t('cron.page.form.nomiNoProvider') : undefined}
                  />
                </Option>
              );
            })}
          </OptGroup>
        )}
        {(presetPresets.length > 0 || removedPresetId) && (
          <OptGroup label={t('conversation.dropdown.presetPresets')}>
            {removedPresetId && (
              <Option
                key={getCronPresetOptionValue(removedPresetId)}
                value={getCronPresetOptionValue(removedPresetId)}
                disabled
                aria-disabled='true'
              >
                <CronUnavailableAgentIdentity
                  name={editJob?.metadata.agent_config?.name || t('cron.page.form.removedPresetLabel', { id: removedPresetId })}
                  statusLabel={t('cron.page.form.removedPresetUnavailable')}
                />
              </Option>
            )}
            {presetPresets.map((preset) => {
              const supportsCron = presetSupportsTarget(preset, 'cron');
              const optionValue = getCronPresetOptionValue(preset.preset_id);
              return (
                <Option
                  key={optionValue}
                  value={optionValue}
                  disabled={!supportsCron}
                  aria-disabled={!supportsCron || undefined}
                >
                  <CronPresetOptionIdentity
                    preset={preset}
                    language={i18n.language}
                    statusLabel={supportsCron ? undefined : t('cron.page.form.presetCronUnavailable')}
                  />
                </Option>
              );
            })}
          </OptGroup>
        )}
      </Select>
    </FormItem>
  );

  const modelFormItem = showModelSelector ? (
    <FormItem label={t('cron.page.form.model')}>
      <GuidModelSelector
        isProviderModelMode={isProviderModelMode}
        modelList={filteredProviders}
        current_model={geminiCurrentModel}
        setCurrentModel={handleGeminiModelSelect}
      />
    </FormItem>
  ) : null;

  return (
    <NomiModal
      title={isEditMode ? t('cron.page.editTask') : t('cron.page.createTask')}
      visible={visible}
      onCancel={onClose}
      onOk={handleSubmit}
      confirmLoading={submitting}
      okText={t('cron.page.save')}
      cancelText={t('cron.page.cancel')}
      className='w-[min(560px,calc(100vw-32px))] max-w-560px rd-16px'
      unmountOnExit
    >
      <div className='overflow-y-auto pb-4px max-h-[min(68vh,640px)]'>
        <Form form={form} layout='vertical'>
          <FormItem
            label={t('cron.page.form.name')}
            field='name'
            rules={[{ required: true, message: t('cron.page.form.nameRequired') }]}
          >
            <Input placeholder={t('cron.page.form.namePlaceholder')} />
          </FormItem>

          {/* Description — optional. */}
          <FormItem label={t('cron.page.form.description')} field='description'>
            <Input placeholder={t('cron.page.form.descriptionPlaceholder')} />
          </FormItem>

          <FormItem label={t('cron.page.form.executionMode')}>
            <Radio.Group
              value={execution_mode}
              disabled={lockInitialTarget || isEditMode}
              onChange={(value) => setExecutionMode(value as ConversationExecutionMode)}
              className='flex flex-wrap items-center gap-20px'
            >
              {conversationModeOptions.map((option) => (
                <Radio key={option.value} value={option.value} className='m-0 min-w-0 cursor-pointer'>
                  <span className='pl-4px text-14px font-medium text-t-primary'>{option.label}</span>
                </Radio>
              ))}
            </Radio.Group>
            <div className='mt-10px rounded-12px border border-solid border-[var(--color-border-2)] bg-fill-2 px-14px py-12px'>
              <p className='m-0 text-12px leading-18px text-t-primary'>{selectedModeDescription}</p>
            </div>
            {execution_mode === 'existing' && (
              <div className='mt-10px flex items-center justify-between gap-12px rounded-12px border border-solid border-[var(--color-border-2)] bg-fill-2 px-14px py-10px'>
                <div className='flex flex-col gap-2px'>
                  <span className='text-13px font-medium text-t-primary'>
                    {t('cron.page.form.clearContextEachRun', { defaultValue: 'Clear context each run' })}
                  </span>
                  <span className='text-12px leading-16px text-t-secondary'>
                    {t('cron.page.form.clearContextEachRunHint', {
                      defaultValue:
                        'Reset the agent context before each run so history does not accumulate across runs. Message records are kept.',
                    })}
                  </span>
                </div>
                <Switch checked={clearContextEachRun} onChange={setClearContextEachRun} />
              </div>
            )}
            {execution_mode === 'specified' && (
              <div className='mt-10px'>
                <Select
                  showSearch
                  disabled={lockInitialTarget}
                  value={specifiedConversationId}
                  onChange={setSpecifiedConversationId}
                  placeholder={t('cron.page.form.selectConversationPlaceholder')}
                  notFoundContent={conversationEmptyText}
                  renderFormat={(_option, value) => {
                    const conv = conversations.find((c) => c.id === value);
                    if (!conv) return '';
                    const idLabel = shortSessionId(conv.id);
                    return conv.name ? `${conv.name}  ${idLabel}` : idLabel;
                  }}
                  filterOption={(input, option) => {
                    const id = (option as React.ReactElement<{ value?: ConversationId }>)?.props?.value;
                    const conv = conversations.find((c) => c.id === id);
                    if (!conv) return false;
                    const lower = input.toLowerCase();
                    const ws = ((conv.extra as unknown as { workspace?: string } | undefined)?.workspace ?? '').toLowerCase();
                    const shortId = shortSessionId(conv.id).toLowerCase();
                    // Match name, workspace, full stable UUID, or its displayed suffix.
                    return (
                      conv.name.toLowerCase().includes(lower) ||
                      conv.id.includes(lower) ||
                      shortId.includes(lower) ||
                      ws.includes(lower)
                    );
                  }}
                >
                  {visibleConversations.map((conv) => (
                    <Option key={conv.id} value={conv.id}>
                      {renderConversationOption(conv)}
                    </Option>
                  ))}
                </Select>
              </div>
            )}
          </FormItem>

          {/* Agent (required) + Model — on the same row when a model is available. */}
          {/* 指定会话复用已存在会话，其 Agent 已固化，不在此重复选择。 */}
          {!isSpecifiedMode &&
            (modelFormItem ? (
              <div className='grid grid-cols-2 gap-12px items-start'>
                {agentFormItem}
                {modelFormItem}
              </div>
            ) : (
              agentFormItem
            ))}

          {/* Project (workspace) — agent tasks only. */}
          {/* 指定会话复用已存在会话，其项目已固化，不在此重复配置。 */}
          {!isSpecifiedMode && (
            <FormItem label={t('cron.page.form.workspace')}>
              <WorkspaceFolderSelect
                value={workspace}
                onChange={(next) => setWorkspace(next || undefined)}
                onClear={handleWorkspaceClear}
                placeholder={t('cron.page.form.selectFolder')}
                input_placeholder={t('cron.page.form.workspacePlaceholder')}
                recentLabel={t('common.filePicker.recent', { defaultValue: 'Recent' })}
                chooseDifferentLabel={t('common.filePicker.chooseDifferentFolder', {
                  defaultValue: 'Choose a different folder',
                })}
                triggerTestId='cron-workspace-trigger'
                menuTestId='cron-workspace-menu'
                menuZIndex={10020}
              />
            </FormItem>
          )}

          {/* Agent execution instruction */}
          <FormItem
            label={t('cron.page.form.prompt')}
            field='prompt'
            rules={[{ required: true, message: t('cron.page.form.promptRequired') }]}
          >
            <TextArea placeholder={t('cron.page.form.promptPlaceholder')} autoSize={{ minRows: 3, maxRows: 8 }} />
          </FormItem>

          {/* Frequency */}
          <FormItem label={t('cron.page.form.frequency')}>
            <Select value={frequency} onChange={handleFrequencyChange}>
              <Option value='manual'>{t('cron.page.freq.manual')}</Option>
              <Option value='hourly'>{t('cron.page.freq.hourly')}</Option>
              <Option value='daily'>{t('cron.page.freq.daily')}</Option>
              <Option value='weekdays'>{t('cron.page.freq.weekdays')}</Option>
              <Option value='weekly'>{t('cron.page.freq.weekly')}</Option>
              <Option value='custom'>{t('cron.page.freq.customCron')}</Option>
            </Select>
            {frequency === 'custom' && (
              <div className='mt-10px'>
                <CronExpressionBuilder value={customCronExpr} onChange={setCustomCronExpr} tz={getCurrentCronTimeZone()} />
              </div>
            )}
          </FormItem>

          {showTimePicker && (
            <div className='flex items-center gap-12px mb-16px'>
              <TimePicker
                format='HH:mm'
                value={dayjs(`2000-01-01 ${time}`)}
                onChange={(_timeStr, pickedTime) => {
                  if (pickedTime) setTime(pickedTime.format('HH:mm'));
                }}
                allowClear={false}
                className='w-120px'
              />
            </div>
          )}

          {showWeekdayPicker && (
            <div className='mb-16px'>
              <Select value={weekday} onChange={setWeekday}>
                {WEEKDAYS.map((d) => (
                  <Option key={d.value} value={d.value}>
                    {t(`cron.page.weekday.${d.label}`)}
                  </Option>
                ))}
              </Select>
            </div>
          )}
        </Form>
      </div>
    </NomiModal>
  );
};

export default CreateTaskDialog;
