/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, Dropdown, Input } from '@arco-design/web-react';
import { ArrowUp, Brain, Down, Send, Shield } from '@icon-park/react';
import {
  MAX_AGENT_EXECUTION_MODELS,
  type TExecutionModelRef,
  type TPlanGate,
} from '@/common/types/agentExecution/agentExecutionTypes';
import NomiSelect from '@/renderer/components/base/NomiSelect';
import { useInputFocusRing } from '@/renderer/hooks/chat/useInputFocusRing';
import { useCompositionInput } from '@/renderer/hooks/chat/useCompositionInput';
import { iconColors } from '@/renderer/styles/colors';
import { type ExecutionModelMode, encodePair, useExecutionModelPool } from './useExecutionModelPool';
import styles from './executionPlanEditor.module.css';

/** The model-range selection the composer's pill edits. Carries the mode plus
 * the encoded-pair selections for `single` / `range`; the parent resolves it
 * into the canonical execution model pool. */
export interface ExecutionModelPoolSelection {
  mode: ExecutionModelMode;
  /** Encoded provider+model pair for `single` mode. */
  single: string;
  /** Encoded provider+model pairs for `range` mode. */
  range: string[];
}

export interface ExecutionPlanEditorProps {
  /** Controlled intent text. */
  value: string;
  onChange: (value: string) => void;
  /** Submit the trimmed intent (create / adjust). Awaited so the composer can
   * keep the in-flight (submitting) affordance until it resolves. */
  onSubmit: (text: string) => Promise<void>;
  /** In-flight — disables input + spins the send affordance. */
  submitting?: boolean;
  placeholder?: string;
  /** Small primary-tinted label inside the card. */
  label?: string;
  /** Drop the centered column so the editor fills a narrow container. */
  fluid?: boolean;

  // ── Toolbar pills (advanced controls) ──────────────────────────────────────
  /** Show the model-pool pill. */
  showModelPool?: boolean;
  modelPool?: ExecutionModelPoolSelection;
  onModelPoolChange?: (next: ExecutionModelPoolSelection) => void;
  /** Show the plan-gate pill. */
  showPlanGate?: boolean;
  planGate?: TPlanGate;
  onPlanGateChange?: (next: TPlanGate) => void;
}

/** Case-insensitive substring match against an option's text label. */
const filterByLabel = (input: string, option: React.ReactNode): boolean => {
  const children = (option as React.ReactElement<{ children?: React.ReactNode }>)?.props?.children;
  return String(children ?? '')
    .toLowerCase()
    .includes(input.toLowerCase());
};

/** Shared editor for creating or revising an execution plan. */
const ExecutionPlanEditor: React.FC<ExecutionPlanEditorProps> = ({
  value,
  onChange,
  onSubmit,
  submitting = false,
  placeholder,
  label,
  fluid = false,
  showModelPool = false,
  modelPool,
  onModelPoolChange,
  showPlanGate = false,
  planGate = 'require_approval',
  onPlanGateChange,
}) => {
  const { t } = useTranslation();
  const { activeBorderColor, inactiveBorderColor, activeShadow } = useInputFocusRing();
  const { isComposing, compositionHandlers } = useCompositionInput();
  const { providers, getAvailableModels, formatModelLabel, allPairs, hasModels } = useExecutionModelPool();

  const [isFocused, setIsFocused] = useState(false);
  const [modelOpen, setModelOpen] = useState(false);
  const [planGateOpen, setPlanGateOpen] = useState(false);

  const trimmed = value.trim();
  const canSubmit = trimmed.length > 0 && !submitting;

  const handleSubmit = useCallback(() => {
    if (!canSubmit) return;
    void onSubmit(value.trim());
  }, [canSubmit, onSubmit, value]);

  // Enter sends; Shift+Enter inserts a newline; the IME guard (ref + the
  // native `isComposing`) prevents an accidental send mid-composition.
  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (isComposing.current || e.nativeEvent.isComposing) return;
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        handleSubmit();
      }
    },
    [handleSubmit, isComposing],
  );

  // Focus glow — swap the inner card's border + shadow exactly like GuidInputCard.
  const innerBorderColor = isFocused ? activeBorderColor : inactiveBorderColor;
  const innerShadow = isFocused ? activeShadow : 'none';

  // ── Model-range pill ────────────────────────────────────────────────────────
  const modelLabel = useMemo(() => {
    if (!hasModels) return t('agentExecution.editor.model.automatic');
    const mode = modelPool?.mode ?? 'automatic';
    if (mode === 'automatic') return t('agentExecution.editor.model.automatic');
    if (mode === 'single') {
      if (!modelPool?.single) return t('agentExecution.editor.model.single');
      const ref = allPairs.find((p) => encodePair(p) === modelPool.single);
      return ref ? ref.model : t('agentExecution.editor.model.single');
    }
    const count = modelPool?.range.length ?? 0;
    return count > 0 ? t('agentExecution.editor.model.rangeCount', { count }) : t('agentExecution.editor.model.range');
  }, [hasModels, modelPool?.mode, modelPool?.single, modelPool?.range, allPairs, t]);

  const setMode = useCallback(
    (mode: ExecutionModelMode) => {
      onModelPoolChange?.({
        mode,
        single: modelPool?.single ?? '',
        range: modelPool?.range ?? [],
      });
    },
    [onModelPoolChange, modelPool?.single, modelPool?.range],
  );

  const modelModeItems: { key: ExecutionModelMode; label: string }[] = useMemo(
    () => [
      { key: 'automatic', label: t('agentExecution.editor.model.automatic') },
      { key: 'single', label: t('agentExecution.editor.model.single') },
      { key: 'range', label: t('agentExecution.editor.model.range') },
    ],
    [t],
  );

  const modelPanel = (
    <div className={styles.composerPopover}>
      <div className='flex flex-col gap-10px'>
        <div className='flex items-center gap-8px'>
          <Brain theme='outline' size='14' fill='rgb(var(--primary-6))' className='shrink-0' />
          <span className={styles.composerPopoverTitle}>{t('agentExecution.editor.modelLabel')}</span>
        </div>

        {!hasModels ? (
          <div className='text-12px leading-18px text-warning-6'>{t('agentExecution.editor.noModels')}</div>
        ) : (
          <>
            <div className={styles.composerSegment}>
              {modelModeItems.map((item) => {
                const active = (modelPool?.mode ?? 'automatic') === item.key;
                return (
                  <div
                    key={item.key}
                    role='button'
                    tabIndex={0}
                    aria-pressed={active}
                    onClick={() => setMode(item.key)}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter' || e.key === ' ') {
                        e.preventDefault();
                        setMode(item.key);
                      }
                    }}
                    className={`${styles.composerSegmentItem} ${active ? styles.composerSegmentItemActive : ''}`}
                  >
                    {item.label}
                  </div>
                );
              })}
            </div>

            {(modelPool?.mode ?? 'automatic') === 'automatic' ? (
              <div className={styles.composerHint}>
                {t('agentExecution.editor.model.automaticHint', {
                  count: allPairs.length,
                })}
              </div>
            ) : (modelPool?.mode ?? 'automatic') === 'single' ? (
              <NomiSelect
                value={modelPool?.single || undefined}
                onChange={(v) =>
                  onModelPoolChange?.({
                    mode: 'single',
                    single: v as string,
                    range: modelPool?.range ?? [],
                  })
                }
                placeholder={t('agentExecution.editor.model.singlePlaceholder')}
                showSearch
                filterOption={filterByLabel}
                className='w-full'
              >
                {providers.map((p) => (
                  <NomiSelect.OptGroup key={p.id} label={p.name || p.platform}>
                    {getAvailableModels(p).map((m) => {
                      const ref: TExecutionModelRef = {
                        provider_id: p.id,
                        model: m,
                      };
                      return (
                        <NomiSelect.Option key={encodePair(ref)} value={encodePair(ref)}>
                          {formatModelLabel(p, m)}
                        </NomiSelect.Option>
                      );
                    })}
                  </NomiSelect.OptGroup>
                ))}
              </NomiSelect>
            ) : (
              <NomiSelect
                mode='multiple'
                value={modelPool?.range ?? []}
                onChange={(v) => {
                  const range = Array.from(new Set(v as string[])).slice(0, MAX_AGENT_EXECUTION_MODELS);
                  onModelPoolChange?.({
                    mode: 'range',
                    single: modelPool?.single ?? '',
                    range,
                  });
                }}
                placeholder={t('agentExecution.editor.model.rangePlaceholder')}
                showSearch
                filterOption={filterByLabel}
                className='w-full'
              >
                {providers.map((p) => (
                  <NomiSelect.OptGroup key={p.id} label={p.name || p.platform}>
                    {getAvailableModels(p).map((m) => {
                      const ref: TExecutionModelRef = {
                        provider_id: p.id,
                        model: m,
                      };
                      const key = encodePair(ref);
                      const selected = modelPool?.range.includes(key) ?? false;
                      const modelLimitReached = (modelPool?.range.length ?? 0) >= MAX_AGENT_EXECUTION_MODELS;
                      return (
                        <NomiSelect.Option key={key} value={key} disabled={modelLimitReached && !selected}>
                          {formatModelLabel(p, m)}
                        </NomiSelect.Option>
                      );
                    })}
                  </NomiSelect.OptGroup>
                ))}
              </NomiSelect>
            )}
          </>
        )}
      </div>
    </div>
  );

  // ── Autonomy pill ────────────────────────────────────────────────────────────
  const planGateItems: { key: TPlanGate; label: string; hint: string }[] = useMemo(
    () => [
      {
        key: 'require_approval',
        label: t('agentExecution.editor.planGate.require_approval'),
        hint: t('agentExecution.editor.planGate.require_approvalHint'),
      },
      {
        key: 'automatic',
        label: t('agentExecution.editor.planGate.automatic'),
        hint: t('agentExecution.editor.planGate.automaticHint'),
      },
    ],
    [t],
  );

  const planGatePanel = (
    <div className={styles.composerPopover}>
      <div className='flex flex-col gap-10px'>
        <div className='flex items-center gap-8px'>
          <Shield theme='outline' size='14' fill='rgb(var(--primary-6))' className='shrink-0' />
          <span className={styles.composerPopoverTitle}>{t('agentExecution.editor.planGateLabel')}</span>
        </div>
        <div className='flex flex-col gap-6px'>
          {planGateItems.map((item) => {
            const active = planGate === item.key;
            return (
              <div
                key={item.key}
                role='button'
                tabIndex={0}
                aria-pressed={active}
                onClick={() => {
                  onPlanGateChange?.(item.key);
                  setPlanGateOpen(false);
                }}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' || e.key === ' ') {
                    e.preventDefault();
                    onPlanGateChange?.(item.key);
                    setPlanGateOpen(false);
                  }
                }}
                className='flex cursor-pointer flex-col gap-2px rd-8px px-10px py-8px transition-colors'
                style={{
                  background: active ? 'color-mix(in srgb, rgb(var(--primary-6)) 10%, transparent)' : 'transparent',
                  border: `1px solid ${active ? 'color-mix(in srgb, rgb(var(--primary-6)) 26%, transparent)' : 'var(--color-border-2)'}`,
                }}
              >
                <span
                  className='text-12px font-600 leading-none'
                  style={{
                    color: active ? 'rgb(var(--primary-6))' : 'var(--color-text-1)',
                  }}
                >
                  {item.label}
                </span>
                <span className='text-11px leading-15px text-t-secondary'>{item.hint}</span>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );

  const planGateLabel =
    planGate === 'automatic' ? t('agentExecution.editor.planGate.automatic') : t('agentExecution.editor.planGate.require_approval');

  return (
    <div className={`${styles.composerLayout} ${fluid ? styles.composerLayoutFluid : ''}`}>
      {/* Outer `--bg-2` shell wrapping the inner rd-24 card (mirrors GuidInputCard). */}
      <div className={styles.composerWrap} style={{ padding: 6 }}>
        <div
          className={`${styles.composerInner} flex flex-col gap-8px p-12px`}
          style={{ borderColor: innerBorderColor, boxShadow: innerShadow }}
        >
          {label && (
            <span className={styles.composerLabel}>
              <Send theme='outline' size='12' strokeWidth={3} fill='rgb(var(--primary-6))' />
              <span>{label}</span>
            </span>
          )}

          <Input.TextArea
            value={value}
            onChange={onChange}
            disabled={submitting}
            autoSize={{ minRows: 2, maxRows: 12 }}
            placeholder={placeholder}
            spellCheck={false}
            className={styles.composerTextarea}
            onFocus={() => setIsFocused(true)}
            onBlur={() => setIsFocused(false)}
            {...compositionHandlers}
            onKeyDown={handleKeyDown}
            data-testid='execution-composer-input'
          />

          {/* Bottom toolbar — pills on the right, circular send at the far right. */}
          <div className={styles.composerToolbar}>
            {showModelPool && (
              <Dropdown trigger='click' popupVisible={modelOpen} onVisibleChange={setModelOpen} droplist={modelPanel} position='tr'>
                <Button className='sendbox-model-btn' shape='round' size='small' data-testid='execution-model-pill'>
                  <span className='flex items-center gap-6px min-w-0'>
                    <Brain theme='outline' size='14' fill={iconColors.secondary} className='shrink-0' />
                    <span className='truncate'>{modelLabel}</span>
                    <Down theme='outline' size='12' fill={iconColors.secondary} className='shrink-0' />
                  </span>
                </Button>
              </Dropdown>
            )}

            {showPlanGate && (
              <Dropdown
                trigger='click'
                popupVisible={planGateOpen}
                onVisibleChange={setPlanGateOpen}
                droplist={planGatePanel}
                position='tr'
              >
                <Button className='sendbox-model-btn' shape='round' size='small' data-testid='execution-planGate-pill'>
                  <span className='flex items-center gap-6px min-w-0'>
                    <Shield theme='outline' size='14' fill={iconColors.secondary} className='shrink-0' />
                    <span className='truncate'>{planGateLabel}</span>
                    <Down theme='outline' size='12' fill={iconColors.secondary} className='shrink-0' />
                  </span>
                </Button>
              </Dropdown>
            )}

            {/* Circular send button — Arco primary circle (mirrors GuidActionRow's
                send affordance). White ArrowUp; disabled goes through the
                `.send-button-custom` class default (no inline override). */}
            <Button
              shape='circle'
              type='primary'
              loading={submitting}
              disabled={!canSubmit}
              className='send-button-custom'
              icon={<ArrowUp theme='filled' size='14' fill='white' strokeWidth={5} />}
              onClick={handleSubmit}
              data-testid='execution-send-btn'
              aria-label={placeholder}
            />
          </div>
        </div>
      </div>
    </div>
  );
};

export default ExecutionPlanEditor;
