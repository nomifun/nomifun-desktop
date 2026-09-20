/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Brain, Refresh } from '@icon-park/react';
import { Button, InputNumber, Popover, Select, Switch, Tooltip } from '@arco-design/web-react';
import classNames from 'classnames';

import { ipcBridge } from '@/common';
import type {
  IIdmmState,
  IdmmMode,
  IdmmRunState,
  IdmmScanScope,
} from '@/common/adapter/ipcBridge';
import type { IIdmmConfig } from '@/common/types/idmm';
import { createDefaultIdmmConfig } from '@/common/types/idmm';
import type { ConversationId } from '@/common/types/ids';
import TaskModelSelect, {
  type TaskModelSelection,
} from '@/renderer/components/model/TaskModelSelect';
import { CAPABILITY_COLORS } from '@/renderer/components/capability/CapabilityIcon';
import { IDMM_STATUS_COLOR } from '@/renderer/components/capability/capabilityStatusColors';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import {
  capabilityHeaderButtonClass,
  capabilityHeaderButtonStyle,
} from './CapabilityHeaderButton';

export const defaultIdmmConfig = createDefaultIdmmConfig;

export type IdmmDraft = {
  value: IIdmmConfig;
  onChange: (next: IIdmmConfig) => void;
};

interface IdmmControlProps {
  target?: { id: ConversationId };
  draft?: IdmmDraft;
  disabledReason?: string;
  applyNote?: string;
  presentation?: 'popover' | 'embedded';
}

const statusLabel = (state: IdmmRunState, t: ReturnType<typeof useTranslation>['t']) =>
  t(`idmm.state.${state}`);

const sectionClass =
  'flex min-w-0 flex-col gap-8px rounded-12px border border-solid border-[var(--color-border-2)] bg-[var(--color-bg-1)] px-12px py-10px';
const settingRowClass =
  'grid min-w-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-12px';
const dividedSettingRowClass = `${settingRowClass} border-t border-t-solid border-t-[var(--color-border-1)] pt-8px`;
const fieldLabelClass =
  'min-w-0 text-11px font-500 leading-16px text-[var(--color-text-1)]';
const tintBg = (color: string, amount = 10): string =>
  `color-mix(in srgb, ${color} ${amount}%, var(--color-bg-1))`;

const IdmmControl: React.FC<IdmmControlProps> = ({
  target,
  draft,
  disabledReason,
  applyNote,
  presentation = 'popover',
}) => {
  const { t } = useTranslation();
  const [message, messageContext] = useArcoMessage({ maxCount: 1 });
  const [state, setState] = useState<IIdmmState | null>(null);
  const [edited, setEdited] = useState<IIdmmConfig>(defaultIdmmConfig);
  const [saving, setSaving] = useState(false);
  const [evaluating, setEvaluating] = useState(false);
  const [dirty, setDirty] = useState(false);
  const requestRef = useRef(0);
  const dirtyRef = useRef(false);
  const isDraft = draft !== undefined;
  const embedded = presentation === 'embedded';
  const controlsDisabled = Boolean(disabledReason);
  const config = draft?.value ?? edited;
  const sessionId = target?.id;

  useEffect(() => {
    if (isDraft) return;
    dirtyRef.current = false;
    setDirty(false);
    setState(null);
    setEdited(defaultIdmmConfig());
  }, [isDraft, sessionId]);

  const update = useCallback(
    (next: IIdmmConfig) => {
      if (draft) draft.onChange(next);
      else {
        dirtyRef.current = true;
        setDirty(true);
        setEdited(next);
      }
    },
    [draft]
  );

  const load = useCallback(async () => {
    if (isDraft || !sessionId) return;
    const request = ++requestRef.current;
    try {
      const next = await ipcBridge.idmm.getStatus.invoke({ agent_session_id: sessionId });
      if (request !== requestRef.current) return;
      setState(next);
      if (!dirtyRef.current) setEdited(next.config);
    } catch {
      // Preserve the last authoritative snapshot. Save/evaluate will surface a
      // concrete error and reconnect/poll will try again.
    }
  }, [isDraft, sessionId]);

  useEffect(() => {
    if (isDraft || !sessionId) return;
    void load();
    const timer = window.setInterval(() => void load(), 10_000);
    const unsubscribe = ipcBridge.conversation.reconnected.on(() => void load());
    return () => {
      requestRef.current += 1;
      window.clearInterval(timer);
      unsubscribe();
    };
  }, [isDraft, load, sessionId]);

  const modeOptions = useMemo(
    () => [
      { value: 'off', label: t('idmm.mode.off') },
      { value: 'rule_only', label: t('idmm.mode.ruleOnly') },
      { value: 'rule_plus_model', label: t('idmm.mode.rulePlusModel') },
    ],
    [t]
  );
  const scopeOptions = useMemo(
    () => [
      { value: 'last_turn', label: t('idmm.scope.lastTurn') },
      { value: 'last_messages', label: t('idmm.scope.lastMessages') },
      { value: 'full_session', label: t('idmm.scope.fullSession') },
    ],
    [t]
  );

  const bypassSelection: TaskModelSelection | null =
    config.bypass_model.provider_id && config.bypass_model.model
      ? {
          provider_id: config.bypass_model.provider_id,
          model: config.bypass_model.model,
          voice: null,
        }
      : null;
  const bypassSelectionMissing =
    config.mode === 'rule_plus_model' &&
    (!config.bypass_model.provider_id || !config.bypass_model.model);

  const save = async () => {
    if (!sessionId) return;
    if (
      config.mode === 'rule_plus_model' &&
      (!config.bypass_model.provider_id || !config.bypass_model.model)
    ) {
      message.warning(t('idmm.validation.bypassRequired'));
      return;
    }
    setSaving(true);
    try {
      const next = await ipcBridge.idmm.setConfig.invoke({
        agent_session_id: sessionId,
        config,
      });
      setState(next);
      setEdited(next.config);
      dirtyRef.current = false;
      setDirty(false);
      message.success(t('idmm.saved'));
    } catch (error) {
      message.error(String(error));
    } finally {
      setSaving(false);
    }
  };

  const evaluate = async () => {
    if (!sessionId) return;
    setEvaluating(true);
    try {
      const next = await ipcBridge.idmm.evaluateNow.invoke({ agent_session_id: sessionId });
      setState(next);
      setEdited(next.config);
      dirtyRef.current = false;
      setDirty(false);
      message.success(t('idmm.evaluated'));
    } catch (error) {
      message.error(String(error));
    } finally {
      setEvaluating(false);
    }
  };

  const enabled = config.mode !== 'off';
  const runState: IdmmRunState = isDraft
    ? enabled
      ? 'monitoring'
      : 'off'
    : state?.run_state ?? (enabled ? 'monitoring' : 'off');
  const color = isDraft
    ? enabled
      ? CAPABILITY_COLORS.primary
      : CAPABILITY_COLORS.off
    : IDMM_STATUS_COLOR[runState];

  const panel = (
    <div
      className={classNames(
        'box-border flex flex-col gap-10px',
        embedded ? 'min-w-0' : 'w-340px overflow-hidden p-12px'
      )}
      style={
        embedded
          ? undefined
          : {
              maxWidth: 'calc(100vw - 24px)',
              maxHeight: 'min(500px, calc(100vh - 32px))',
            }
      }
    >
      {messageContext}
      <div className='flex shrink-0 flex-col gap-6px'>
        <div className='flex items-center justify-between gap-10px'>
          <span className='inline-flex min-w-0 items-center gap-8px'>
            <span
              className='inline-flex h-24px w-24px shrink-0 items-center justify-center rounded-6px'
              style={{ background: tintBg(color), color }}
            >
              <Brain theme='outline' size='15' fill='currentColor' />
            </span>
            <span className='min-w-0 truncate text-13px font-600 text-t-primary'>
              {t('idmm.title')}
            </span>
          </span>
          <span className='inline-flex shrink-0 items-center gap-5px rounded-full border border-solid border-[var(--color-border-2)] bg-[var(--color-bg-1)] px-7px py-3px text-11px font-500 text-[var(--color-text-1)]'>
            <span className='inline-block h-6px w-6px rounded-full' style={{ backgroundColor: color }} />
            {statusLabel(runState, t)}
          </span>
        </div>
        <div className='text-11px leading-16px text-[var(--color-text-2)]'>
          {t('idmm.description')}
        </div>
      </div>

      <div
        className={classNames(
          'flex min-h-0 flex-1 flex-col gap-8px',
          embedded ? 'min-w-0' : 'overflow-x-hidden overflow-y-auto pr-2px'
        )}
      >
        <div className={sectionClass}>
          <div className='flex min-w-0 flex-col gap-4px'>
            <span className={fieldLabelClass}>{t('idmm.mode.label')}</span>
            <Select
              size='small'
              value={config.mode}
              options={modeOptions}
              disabled={controlsDisabled}
              onChange={(mode: IdmmMode) => update({ ...config, mode })}
              className='w-full'
            />
          </div>
          {enabled && (
            <div
              className='rounded-8px px-10px py-8px text-11px leading-16px text-[var(--color-text-2)]'
              style={{ background: tintBg(color, 6) }}
            >
              {config.mode === 'rule_only'
                ? t('idmm.mode.ruleOnlyHint')
                : t('idmm.mode.rulePlusModelHint')}
            </div>
          )}
        </div>

        {enabled && (
          <div className={sectionClass}>
            <span className='text-12px font-600 text-t-primary'>{t('idmm.mode.ruleOnly')}</span>
            <div className={settingRowClass}>
              <span className={fieldLabelClass}>{t('idmm.recoverProviderFailures')}</span>
              <Switch
                size='small'
                disabled={controlsDisabled}
                checked={config.recover_provider_failures}
                onChange={(value) => update({ ...config, recover_provider_failures: value })}
              />
            </div>
            <div className={dividedSettingRowClass}>
              <span className={fieldLabelClass}>{t('idmm.recoverStalledTurns')}</span>
              <Switch
                size='small'
                disabled={controlsDisabled}
                checked={config.recover_stalled_turns}
                onChange={(value) => update({ ...config, recover_stalled_turns: value })}
              />
            </div>
            <div className={dividedSettingRowClass}>
              <span className={fieldLabelClass}>{t('idmm.autoSelectOptions')}</span>
              <Switch
                size='small'
                disabled={controlsDisabled}
                checked={config.auto_select_options}
                onChange={(value) => update({ ...config, auto_select_options: value })}
              />
            </div>
            <div className={dividedSettingRowClass}>
              <span
                className={classNames(
                  fieldLabelClass,
                  !config.auto_select_options && 'text-[var(--color-text-3)]'
                )}
              >
                {t('idmm.preferRecommended')}
              </span>
              <Switch
                size='small'
                checked={config.prefer_recommended}
                disabled={controlsDisabled || !config.auto_select_options}
                onChange={(value) => update({ ...config, prefer_recommended: value })}
              />
            </div>
            <div className={dividedSettingRowClass}>
              <span className={fieldLabelClass}>{t('idmm.idleTimeout')}</span>
              <InputNumber
                size='small'
                min={30}
                max={1800}
                disabled={controlsDisabled}
                value={config.idle_timeout_secs}
                suffix={t('idmm.seconds')}
                onChange={(value) =>
                  update({ ...config, idle_timeout_secs: Number(value) || 90 })
                }
                className='w-112px'
              />
            </div>
          </div>
        )}

        {config.mode === 'rule_plus_model' && (
          <div className={sectionClass}>
            <span className='text-12px font-600 text-t-primary'>{t('idmm.bypassModel')}</span>
            <TaskModelSelect
              task='chat'
              value={bypassSelection}
              onChange={(selection) =>
                update({
                  ...config,
                  bypass_model: {
                    provider_id: selection.provider_id,
                    model: selection.model,
                  },
                })
              }
              size='small'
              layout='stacked'
              disabled={controlsDisabled}
              emptyHint={t('idmm.validation.noChatModel')}
            />
            {bypassSelectionMissing && (
              <span className='text-11px leading-15px text-warning-6' role='alert'>
                {t('idmm.validation.bypassRequired')}
              </span>
            )}
            <div className={dividedSettingRowClass}>
              <span className={fieldLabelClass}>{t('idmm.scanScope')}</span>
              <Select
                size='small'
                value={config.scan_scope}
                options={scopeOptions}
                disabled={controlsDisabled}
                onChange={(scan_scope: IdmmScanScope) => update({ ...config, scan_scope })}
                className='w-150px'
              />
            </div>
            {config.scan_scope === 'last_messages' && (
              <div className={dividedSettingRowClass}>
                <span className={fieldLabelClass}>{t('idmm.contextMessages')}</span>
                <InputNumber
                  size='small'
                  min={1}
                  max={100}
                  disabled={controlsDisabled}
                  value={config.max_context_messages}
                  onChange={(value) =>
                    update({ ...config, max_context_messages: Number(value) || 12 })
                  }
                  className='w-90px'
                />
              </div>
            )}
          </div>
        )}

        {!isDraft && state?.recent_interventions.length ? (
          <div className={sectionClass}>
            <span className='text-12px font-600 text-t-primary'>{t('idmm.recentActivity')}</span>
            {state.recent_interventions.slice(0, 3).map((item) => (
              <div key={item.intervention_id} className='rounded-8px bg-fill-2 px-9px py-7px'>
                <div className='flex min-w-0 items-center justify-between gap-8px text-11px'>
                  <span className='min-w-0 truncate text-t-secondary'>
                    {t(`idmm.interventionReason.${item.reason}`, { defaultValue: item.reason })}
                  </span>
                  <span
                    className={classNames(
                      'shrink-0 font-500',
                      item.status === 'failed'
                        ? 'text-danger-6'
                        : item.status === 'halted'
                          ? 'text-warning-6'
                          : 'text-success-6'
                    )}
                  >
                    {t(`idmm.interventionStatus.${item.status}`)}
                  </span>
                </div>
                {item.detail && (
                  <div className='mt-3px line-clamp-2 break-words text-10px leading-14px text-t-tertiary'>
                    {item.detail}
                  </div>
                )}
              </div>
            ))}
          </div>
        ) : null}

        {applyNote && (
          <div className='px-2px text-11px leading-15px text-t-quaternary'>{applyNote}</div>
        )}
      </div>

      {!isDraft && (
        <div className='flex shrink-0 items-center justify-between gap-8px border-t border-t-solid border-t-[var(--color-border-1)] pt-8px'>
          <Button
            size='mini'
            type='text'
            icon={<Refresh theme='outline' size='13' />}
            disabled={!enabled || dirty}
            loading={evaluating}
            onClick={() => void evaluate()}
          >
            {t('idmm.evaluateNow')}
          </Button>
          <Button size='mini' type='primary' loading={saving} onClick={() => void save()}>
            {t('common.save')}
          </Button>
        </div>
      )}
    </div>
  );

  const button = (
    <Button
      size='mini'
      shape='round'
      type='secondary'
      disabled={Boolean(disabledReason)}
      className={capabilityHeaderButtonClass(enabled, 'shrink-0')}
      style={capabilityHeaderButtonStyle(color)}
      aria-label={t('idmm.title')}
    >
      <span className='inline-flex items-center gap-6px leading-none'>
        <Brain theme='outline' size='14' fill={color} />
        <span className='text-12px'>{t('idmm.shortLabel')}</span>
        <span className='sr-only'>{statusLabel(runState, t)}</span>
      </span>
    </Button>
  );

  if (embedded) return panel;

  if (disabledReason) {
    return (
      <Tooltip content={disabledReason}>
        <span className='inline-flex'>{button}</span>
      </Tooltip>
    );
  }
  return (
    <Popover className='idmm-control-popover' trigger='click' position='br' content={panel}>
      {button}
    </Popover>
  );
};

export default IdmmControl;
