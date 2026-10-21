/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useState, useMemo, useEffect, useCallback, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { Form, Input, Select, Message, TimePicker, Radio, Switch } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import NomiModal from '@renderer/components/base/NomiModal';
import type { ICreateCronJobParams, ICronJob } from '@/common/adapter/ipcBridge';
import AgentSelector from '@/renderer/components/agent/AgentSelector';
import { DEFAULT_AGENT_SELECTION, type AgentSelection } from '@/renderer/components/agent/types';
import { agentSelectionFromWire, agentSelectionToWire } from '@/renderer/components/agent/agentSelection';
import { useConversationAgentCatalog } from '@/renderer/components/agent/useConversationAgentCatalog';
import { useAgentLaunchCapabilities } from '@/renderer/components/agent/useAgentLaunchCapabilities';
import { useAgentLaunchModel } from '@/renderer/components/agent/useAgentLaunchModel';
import { scheduledWorkspace, scheduledAgentName } from './scheduledAgentPresentation';
import dayjs from 'dayjs';
import type { ConversationId } from '@/common/types/ids';
import ChatModelSelector from '@renderer/components/chat/ChatModelSelector';
import {
  WorkspaceDirectoryUnavailableError,
  WorkspaceFolderSelect,
  validateExistingWorkspaceDirectory,
} from '@renderer/components/workspace';
import { createCronSchedule, getCurrentCronTimeZone } from '@renderer/pages/cron/cronUtils';
import { useAllCronJobs } from '@renderer/pages/cron/useCronJobs';
import { getConversationCreateErrorMessage } from '@renderer/pages/conversation/utils/conversationCreateError';
import CronExpressionBuilder, { validateCronExpression } from './CronExpressionBuilder';
import { useConversationListSync } from '@renderer/pages/conversation/SessionList/hooks/useConversationListSync';
import { renderConversationOption } from '@renderer/pages/conversation/components/renderConversationOption';
import { shortSessionId } from '@renderer/utils/ui/shortId';
import {
  buildCronConversationRequestFields,
  resolveCronConversationTarget,
  type ConversationExecutionMode,
} from './cronConversationTarget';

const FormItem = Form.Item;
const TextArea = Input.TextArea;
const Option = Select.Option;

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

type FrequencyType = 'manual' | 'hourly' | 'daily' | 'weekdays' | 'weekly' | 'custom' | 'preserve';
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
  if (!expr.trim()) return { frequency: 'manual', time: '09:00', weekday: 'MON' };

  let parts = expr.trim().split(/\s+/);
  if (parts.length === 5) parts = ['0', ...parts];
  if (parts.length !== 6) return { frequency: 'custom', time: '09:00', weekday: 'MON' };

  const [seconds, min, hour, dayRaw, month, dowRaw] = parts;
  if (seconds !== '0') return { frequency: 'custom', time: '09:00', weekday: 'MON' };
  const day = dayRaw === '?' ? '*' : dayRaw;
  const dow = dowRaw === '?' ? '*' : dowRaw;

  if (hour === '*' && min === '0' && day === '*' && month === '*' && dow === '*') {
    return { frequency: 'hourly', time: '09:00', weekday: 'MON' };
  }
  if (!/^\d+$/.test(hour) || !/^\d+$/.test(min) || Number(hour) > 23 || Number(min) > 59) {
    return { frequency: 'custom', time: '09:00', weekday: 'MON' };
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
    return { frequency: 'custom', time: '09:00', weekday: 'MON' };
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
  const { t } = useTranslation();
  const [form] = Form.useForm();
  const [submitting, setSubmitting] = useState(false);
  const sessionRef = useRef(0);
  const submittingRef = useRef(false);
  const [scheduleChanged, setScheduleChanged] = useState(false);
  const agentCatalog = useConversationAgentCatalog();
  const [frequency, setFrequency] = useState<FrequencyType>('manual');
  const [time, setTime] = useState('09:00');
  const [weekday, setWeekday] = useState('MON');
  const [customCronExpr, setCustomCronExpr] = useState<string>('');

  const isEditMode = !!editJob;
  const isBoundConversation = editJob?.execution_mode === 'existing' && Boolean(editJob.metadata.conversation_id);
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
      // Only continuing execution owns a stable target Session. Each
      // new-conversation run exposes its own Session through run history.
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

  const [workspace, setWorkspace] = useState<string | undefined>(undefined);
  const [selection, setSelection] = useState<AgentSelection>(DEFAULT_AGENT_SELECTION);
  const [agentChoiceChanged, setAgentChoiceChanged] = useState(false);
  const usesAuthoringSelection = !editJob || agentChoiceChanged;
  const isSpecifiedMode = execution_mode === 'specified';
  const selectedTemplate = selection.kind === 'template'
    ? agentCatalog.officialTemplates.find(template => template.template_key === selection.templateKey)
    : undefined;
  const selectedPreset = selection.kind === 'preset'
    ? agentCatalog.presets.find(preset => preset.preset_id === selection.presetId)
    : undefined;
  const presetCapabilities = useAgentLaunchCapabilities(
    isSpecifiedMode || isBoundConversation || !usesAuthoringSelection ? undefined : selectedPreset,
    isSpecifiedMode || isBoundConversation || !usesAuthoringSelection ? undefined : selectedTemplate,
  );
  const requiresToolCalls = presetCapabilities.actionIds.size > 0;
  const modelSelection = useAgentLaunchModel({
    defaultModel: presetCapabilities.defaultModel,
    ready: usesAuthoringSelection && !presetCapabilities.isLoading && !presetCapabilities.error && !agentCatalog.isLoading,
    requiresToolCalls: usesAuthoringSelection && requiresToolCalls,
  });

  // A session owns its draft and submission; live updates to the same job do
  // not reinitialize the form. Closing/switching invalidates UI continuations,
  // not a backend write that was already dispatched.
  useEffect(() => {
    sessionRef.current += 1;
    submittingRef.current = false;
    setSubmitting(false);
    setScheduleChanged(false);
    setAgentChoiceChanged(false);
    if (!visible) return;
    if (editJob) {
      const cronExpr = editJob.schedule.kind === 'cron' ? editJob.schedule.expr : '';
      const parsed = parseCronExpr(cronExpr);
      setFrequency(editJob.schedule.kind === 'cron' ? parsed.frequency : 'preserve');
      setTime(parsed.time);
      setWeekday(parsed.weekday);
      setCustomCronExpr(parsed.frequency === 'custom' ? cronExpr : '');

      setExecutionMode(editJob.execution_mode);
      setSpecifiedConversationId(undefined);
      setSelection(editJob.metadata.agent_selection ? agentSelectionFromWire(editJob.metadata.agent_selection) : DEFAULT_AGENT_SELECTION);
      form.setFieldsValue({
        name: editJob.name,
        description: getDescriptionInitialValue(editJob),
        prompt: editJob.message,
      });
      modelSelection.reset(editJob.metadata.model);
      setWorkspace(scheduledWorkspace(editJob));
      setClearContextEachRun(editJob.metadata.clear_context_each_run);
    } else {
      form.resetFields();
      setFrequency('manual');
      setTime('09:00');
      setWeekday('MON');
      setCustomCronExpr('');
      setExecutionMode(initialSpecifiedConversationId ? 'specified' : 'new_conversation');
      setSpecifiedConversationId(initialSpecifiedConversationId);
      modelSelection.reset();
      setWorkspace(undefined);
      setSelection(DEFAULT_AGENT_SELECTION);
      setClearContextEachRun(false);
    }
    return () => { sessionRef.current += 1; };
  }, [visible, editJob?.cron_job_id, form, initialSpecifiedConversationId]);

  const showTimePicker = frequency === 'daily' || frequency === 'weekdays' || frequency === 'weekly';
  const showWeekdayPicker = frequency === 'weekly';
  const scheduleTimeZone =
    (editJob?.schedule.kind === 'cron' && editJob.schedule.tz) || getCurrentCronTimeZone();

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

  const handleFrequencyChange = (value: FrequencyType) => {
    setScheduleChanged(value !== 'preserve');
    setFrequency(value);
    if (value === 'custom') {
      setCustomCronExpr((prev) => prev || '0 0 9 * * ?');
    } else {
      setCustomCronExpr('');
    }
  };

  const handleWorkspaceClear = useCallback(() => {
    setWorkspace(undefined);
  }, []);

  const handleSubmit = async () => {
    if (!visible || submittingRef.current) return;
    submittingRef.current = true;
    const session = sessionRef.current;
    const isCurrent = () => session === sessionRef.current;
    try {
      const values = await form.validate();
      if (!isCurrent()) return;

      if ((!isEditMode || scheduleChanged) && frequency !== 'manual' && !validateCronExpression(scheduleInfo.expr, scheduleTimeZone).valid) {
        Message.error(t('cron.page.cronExpression.invalid'));
        return;
      }

      const schedule = { ...createCronSchedule(scheduleInfo.expr, scheduleInfo.description), tz: scheduleTimeZone };
      const conversationTarget = resolveCronConversationTarget(execution_mode, specifiedConversationId);

      if (!conversationTarget) {
        Message.error(t('cron.page.form.specifiedConversationRequired'));
        return;
      }

      const selectedConversation = conversationTarget.kind === 'specified'
        ? conversations.find((conversation) => conversation.id === conversationTarget.conversationId)
        : undefined;
      const specifiedWorkspace = selectedConversation?.extra?.workspace;
      const workspaceCandidate = conversationTarget.kind === 'specified'
        ? (typeof specifiedWorkspace === 'string' ? specifiedWorkspace : undefined)
        : workspace;
      let workspaceForSubmission = workspace;
      if (!isBoundConversation && workspaceCandidate?.trim()) {
        const canonicalWorkspace = await validateExistingWorkspaceDirectory(workspaceCandidate);
        if (!isCurrent()) return;
        if (conversationTarget.kind !== 'specified') {
          workspaceForSubmission = canonicalWorkspace;
          if (canonicalWorkspace !== workspace) setWorkspace(canonicalWorkspace);
        }
      }

      const base = {
        name: values.name, description: values.description, message: values.prompt,
      };
      setSubmitting(true);
      if (conversationTarget.kind === 'specified') {
        if (boundConversationIds.has(conversationTarget.conversationId)) {
          throw new Error(t('cron.page.form.conversationAlreadyBound'));
        }
        await ipcBridge.cron.addJob.invoke({
          ...base, schedule, created_by: 'user', conversation_title: selectedConversation?.name,
          ...buildCronConversationRequestFields(conversationTarget),
        });
      } else {
        const wireSelection = agentSelectionToWire(selection);
        const previous = editJob?.metadata;
        const agentChanged = !isBoundConversation && usesAuthoringSelection;
        const modelChanged = !isBoundConversation && (!editJob
          || modelSelection.selection?.provider_id !== previous?.model?.provider_id
          || modelSelection.selection?.model !== previous?.model?.model);
        const workspaceChanged = !isBoundConversation && (!editJob
          || (workspaceForSubmission || '') !== (scheduledWorkspace(editJob) || ''));
        const configurationChanged = agentChanged || modelChanged || workspaceChanged;
        const contextChanged = !editJob || clearContextEachRun !== Boolean(previous?.clear_context_each_run);
        if (agentChanged) {
          if (agentCatalog.isLoading || presetCapabilities.isLoading || modelSelection.isLoading) throw new Error(t('guid.agentEntries.loading'));
          if (presetCapabilities.error) throw presetCapabilities.error;
          if (selection.kind === 'preset' && !selectedPreset) throw new Error(t('cron.page.form.removedPresetRequired'));
          if (!modelSelection.compatible) throw new Error(t('cron.page.form.modelIncompatible'));
        }
        if (agentChanged || modelChanged) {
          if (modelSelection.isLoading) throw new Error(t('guid.agentEntries.loading'));
          if (!modelSelection.selection || !modelSelection.available) throw new Error(t('cron.page.form.modelUnavailable'));
        }
        const configuration = configurationChanged ? {
          ...(agentChanged ? { agent_selection: wireSelection } : {}),
          ...(modelChanged ? { model: modelSelection.selection } : {}),
          ...(workspaceChanged ? (workspaceForSubmission ? { workspace: workspaceForSubmission } : editJob ? { workspace: '' } : {}) : {}),
        } : {};
        const contextConfiguration = contextChanged ? { clear_context_each_run: execution_mode === 'existing' && clearContextEachRun } : {};
        if (editJob) {
          await ipcBridge.cron.updateJob.invoke({
            cron_job_id: editJob.cron_job_id,
            updates: { ...base, ...(scheduleChanged ? { schedule } : {}), ...configuration, ...contextConfiguration },
          });
        } else {
          const params: ICreateCronJobParams = {
            ...base, schedule, created_by: 'user', ...configuration, ...contextConfiguration,
            ...buildCronConversationRequestFields(conversationTarget),
          };
          await ipcBridge.cron.addJob.invoke(params);
        }
      }
      if (!isCurrent()) return;
      Message.success(t(isEditMode ? 'cron.page.updateSuccess' : 'cron.page.createSuccess'));

      onClose();
    } catch (err) {
      if (isCurrent()) {
        Message.error(
          err instanceof WorkspaceDirectoryUnavailableError
            ? t('cron.page.form.workspaceUnavailable', { workspacePath: err.workspacePath })
            : getConversationCreateErrorMessage(err, t)
        );
      }
    } finally {
      if (isCurrent()) {
        submittingRef.current = false;
        setSubmitting(false);
      }
    }
  };

  const agentFormItem = <FormItem label={t('cron.page.form.agent')}>
    <AgentSelector presets={agentCatalog.presets} draftPresets={agentCatalog.draftPresets}
      officialTemplates={agentCatalog.officialTemplates} selection={selection}
      isLoading={agentCatalog.isLoading} loadError={agentCatalog.error} onRetry={agentCatalog.refresh}
      selectedLabelOverride={editJob && (isBoundConversation || !agentChoiceChanged) ? scheduledAgentName(editJob, t) : undefined}
      disabled={isBoundConversation} disabledReason={t('cron.page.form.boundConversationHint')} menuZIndex={10030}
      onSelectPreset={presetId => { setSelection({ kind: 'preset', presetId }); setAgentChoiceChanged(true); }}
      onSelectTemplate={templateKey => { setSelection({ kind: 'template', templateKey }); setAgentChoiceChanged(true); }} />
  </FormItem>;
  const modelFormItem = <FormItem label={t('cron.page.form.model')} extra={isBoundConversation ? t('cron.page.form.boundConversationHint') : undefined}>
    <ChatModelSelector providers={modelSelection.groups.map(group => group.provider)}
      currentModel={modelSelection.currentModel}
      readOnlyLabel={!modelSelection.currentModel ? modelSelection.selection?.model : undefined}
      getAvailableModels={provider => modelSelection.groups.find(group => group.provider.id === provider.id)?.models ?? []}
      disabled={isBoundConversation}
      requiredTechnicalCapabilities={requiresToolCalls ? ['function_calling'] : []}
      onSelectModel={async (provider, model) => modelSelection.select(provider.id, model)} />
    {modelSelection.selection && !modelSelection.isLoading && (!modelSelection.available || !modelSelection.compatible) &&
      <div role='alert' className='text-12px text-t-secondary'>{t(modelSelection.available ? 'cron.page.form.modelIncompatible' : 'cron.page.form.modelUnavailable')}</div>}
  </FormItem>;

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

          {!isSpecifiedMode && <div className='grid grid-cols-2 gap-12px items-start'>
            {agentFormItem}{modelFormItem}
          </div>}

          {/* Project (workspace) — agent tasks only. */}
          {/* 指定会话复用已存在会话，其项目已固化，不在此重复配置。 */}
          {!isSpecifiedMode && (
            <FormItem label={t('cron.page.form.workspace')}>
              {isBoundConversation ? <div className='text-13px text-t-secondary'>{workspace || '-'}<p>{t('cron.page.form.boundConversationHint')}</p></div> : <WorkspaceFolderSelect
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
              />}
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
              {editJob && editJob.schedule.kind !== 'cron' && (
                <Option value='preserve'>{editJob.schedule.description || t('cron.page.freq.customCron')}</Option>
              )}
              <Option value='manual'>{t('cron.page.freq.manual')}</Option>
              <Option value='hourly'>{t('cron.page.freq.hourly')}</Option>
              <Option value='daily'>{t('cron.page.freq.daily')}</Option>
              <Option value='weekdays'>{t('cron.page.freq.weekdays')}</Option>
              <Option value='weekly'>{t('cron.page.freq.weekly')}</Option>
              <Option value='custom'>{t('cron.page.freq.customCron')}</Option>
            </Select>
            {frequency === 'custom' && (
              <div className='mt-10px'>
                <CronExpressionBuilder value={customCronExpr} onChange={(expr) => {
                  setScheduleChanged(true);
                  setCustomCronExpr(expr);
                }} tz={scheduleTimeZone} />
              </div>
            )}
          </FormItem>

          {showTimePicker && (
            <div className='flex items-center gap-12px mb-16px'>
              <TimePicker
                format='HH:mm'
                value={dayjs(`2000-01-01 ${time}`)}
                onChange={(_timeStr, pickedTime) => {
                  if (pickedTime) {
                    setScheduleChanged(true);
                    setTime(pickedTime.format('HH:mm'));
                  }
                }}
                allowClear={false}
                className='w-120px'
              />
            </div>
          )}

          {showWeekdayPicker && (
            <div className='mb-16px'>
              <Select value={weekday} onChange={(value) => { setScheduleChanged(true); setWeekday(value); }}>
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
