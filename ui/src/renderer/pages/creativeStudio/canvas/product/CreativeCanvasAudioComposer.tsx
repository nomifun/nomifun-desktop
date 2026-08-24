/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  ArrowUp,
  BookOne,
  Loading,
  Refresh,
  SettingTwo,
} from '@icon-park/react';
import { Popover, Select } from '@arco-design/web-react';
import React, { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';

import type { CreativeGenerationStatus } from '../../domain';
import type {
  CreativeModelOption,
  CreativeModelSelectionRef,
} from '../../models';
import styles from './CreativeCanvasAudioComposer.module.css';

export type CanvasAudioFormat = 'mp3' | 'wav';

export interface CanvasAudioComposeSettings {
  model: CreativeModelSelectionRef | null;
  voice: string;
  format: CanvasAudioFormat;
}

export interface CanvasAudioComposeTaskSummary {
  state: CreativeGenerationStatus;
  pendingCount: number;
  message?: string;
}

export interface CreativeCanvasAudioComposerProps {
  nodeId: string;
  initialPrompt: string;
  settings: CanvasAudioComposeSettings;
  modelOptions: readonly CreativeModelOption[];
  task: CanvasAudioComposeTaskSummary;
  disabled?: boolean;
  error?: string | null;
  retrySubmission?: boolean;
  voiceSupported?: boolean;
  voiceRequired?: boolean;
  formatSupported?: boolean;
  maxTextLength?: number;
  onPromptChange(prompt: string): void;
  onModelChange(model: CreativeModelSelectionRef | null): void;
  onVoiceChange(voice: string): void;
  onFormatChange(format: CanvasAudioFormat): void;
  onOpenPromptLibrary(): void;
  onGenerate(prompt: string): void;
  onRetrySubmission?(): void;
  onConfirmSubmission?(): void;
}

export interface CanvasAudioComposerSubmissionInput {
  disabled: boolean;
  busy: boolean;
  prompt: string;
  hasModel: boolean;
  requiredVoiceReady: boolean;
  promptLengthReady: boolean;
  retrySubmission: boolean;
  onGenerate(prompt: string): void;
  onRetrySubmission?(): void;
}

export type CanvasAudioComposerSubmissionResult =
  | 'generated'
  | 'retried'
  | null;

const AUDIO_FORMAT_OPTIONS: readonly CanvasAudioFormat[] = ['mp3', 'wav'];

const modelKey = (model: CreativeModelSelectionRef): string =>
  JSON.stringify([model.providerId, model.model]);

const findModel = (
  key: unknown,
  options: readonly CreativeModelOption[]
): CreativeModelOption | null =>
  typeof key === 'string'
    ? (options.find((option) => modelKey(option) === key) ?? null)
    : null;

const popupContainer = (trigger: HTMLElement): HTMLElement =>
  (trigger.closest('[data-canvas-audio-composer]') as HTMLElement | null) ??
  document.body;

export const isCanvasAudioComposerSubmitKey = (
  key: string,
  shiftKey: boolean
): boolean => key === 'Enter' && !shiftKey;

/** Dispatch either a new synthesis or the existing idempotent submission. */
export function dispatchCanvasAudioComposerSubmission(
  input: CanvasAudioComposerSubmissionInput
): CanvasAudioComposerSubmissionResult {
  if (input.disabled) return null;
  if (input.retrySubmission) {
    if (!input.onRetrySubmission) return null;
    input.onRetrySubmission();
    return 'retried';
  }
  const prompt = input.prompt.trim();
  if (
    input.busy ||
    !input.hasModel ||
    !input.requiredVoiceReady ||
    !input.promptLengthReady ||
    !prompt
  ) {
    return null;
  }
  input.onGenerate(prompt);
  return 'generated';
}

const CreativeCanvasAudioComposer: React.FC<
  CreativeCanvasAudioComposerProps
> = ({
  nodeId,
  initialPrompt,
  settings,
  modelOptions,
  task,
  disabled = false,
  error,
  retrySubmission = false,
  voiceSupported = false,
  voiceRequired = false,
  formatSupported = false,
  maxTextLength = 4096,
  onPromptChange,
  onModelChange,
  onVoiceChange,
  onFormatChange,
  onOpenPromptLibrary,
  onGenerate,
  onRetrySubmission,
  onConfirmSubmission,
}) => {
  const { t } = useTranslation();
  const positionerRef = useRef<HTMLDivElement>(null);
  const anchorRef = useRef<HTMLSpanElement>(null);
  const horizontalOffsetRef = useRef(0);
  const [prompt, setPrompt] = useState(initialPrompt);
  const [placement, setPlacement] = useState<'above' | 'below'>('below');
  const [horizontalOffset, setHorizontalOffset] = useState(0);
  const [overlay, setOverlay] = useState(false);
  const [overlayLayout, setOverlayLayout] = useState({
    left: 16,
    top: 16,
    width: 358,
  });
  const eligibleModelOptions = modelOptions.filter(
    (option) => option.task === 'speech_synthesis'
  );
  const selectedModel = settings.model
    ? eligibleModelOptions.find(
        (option) => modelKey(option) === modelKey(settings.model!)
      ) ?? null
    : null;
  const normalizedMaxTextLength = Math.max(
    1,
    Math.min(
      1_000_000,
      Math.floor(Number.isFinite(maxTextLength) ? maxTextLength : 4096)
    )
  );
  const voiceVisible = selectedModel !== null && voiceSupported;
  const formatVisible = selectedModel !== null && formatSupported;
  const settingsAvailable = voiceVisible || formatVisible;
  const requiredVoiceReady =
    !voiceVisible || !voiceRequired || settings.voice.trim().length > 0;
  const promptLengthReady =
    Array.from(prompt).length <= normalizedMaxTextLength;
  const busy = task.state === 'queued' || task.state === 'running';
  const canSubmit = retrySubmission
    ? !disabled && onRetrySubmission !== undefined
    : !disabled &&
      !busy &&
      prompt.trim().length > 0 &&
      selectedModel !== null &&
      requiredVoiceReady &&
      promptLengthReady;
  const voiceSummary =
    settings.voice.trim() ||
    t(
      voiceRequired
        ? 'creativeStudio.canvas.audio.voiceRequired'
        : 'creativeStudio.canvas.audio.defaultVoice',
      { defaultValue: voiceRequired ? '待填写音色' : '默认音色' }
    );
  const settingsSummary = [
    formatVisible ? settings.format.toUpperCase() : null,
    voiceVisible ? voiceSummary : null,
  ]
    .filter(Boolean)
    .join(' · ');

  useEffect(() => setPrompt(initialPrompt), [initialPrompt, nodeId]);

  useLayoutEffect(() => {
    const positioner = positionerRef.current;
    const anchor = anchorRef.current;
    const surface = anchor?.closest<HTMLElement>('[data-canvas-surface]');
    const node = anchor?.closest<HTMLElement>('[data-canvas-node-id]');
    if (!positioner || !surface || !node) return;

    const updatePlacement = (): void => {
      const surfaceRect = surface.getBoundingClientRect();
      const nodeRect = node.getBoundingClientRect();
      const panelRect = positioner.getBoundingClientRect();
      const panelHeight = panelRect.height;
      const inset = 12;
      const gap = 16;
      const compactWidth = Math.min(580, Math.max(0, window.innerWidth - 32));
      const shouldOverlay = surfaceRect.width < compactWidth + inset * 2;
      const spaceBelow = surfaceRect.bottom - nodeRect.bottom - gap - inset;
      const spaceAbove = nodeRect.top - surfaceRect.top - gap - inset;
      const next =
        panelHeight <= spaceBelow || spaceBelow >= spaceAbove ? 'below' : 'above';
      setPlacement((current) => (current === next ? current : next));

      setOverlay((current) =>
        current === shouldOverlay ? current : shouldOverlay
      );
      if (shouldOverlay) {
        const belowTop = nodeRect.bottom + gap;
        const aboveTop = nodeRect.top - panelHeight - gap;
        const preferredTop =
          belowTop + panelHeight <= window.innerHeight - inset
            ? belowTop
            : aboveTop >= inset
              ? aboveTop
              : Math.max(
                  inset,
                  Math.min(
                    window.innerHeight - panelHeight - inset,
                    nodeRect.top
                  )
                );
        const nextLayout = {
          left: Math.max(16, (window.innerWidth - compactWidth) / 2),
          top: preferredTop,
          width: compactWidth,
        };
        setOverlayLayout((current) =>
          Math.abs(current.left - nextLayout.left) < 0.5 &&
          Math.abs(current.top - nextLayout.top) < 0.5 &&
          Math.abs(current.width - nextLayout.width) < 0.5
            ? current
            : nextLayout
        );
        if (horizontalOffsetRef.current !== 0) {
          horizontalOffsetRef.current = 0;
          setHorizontalOffset(0);
        }
        return;
      }

      const naturalLeft = panelRect.left - horizontalOffsetRef.current;
      const naturalRight = panelRect.right - horizontalOffsetRef.current;
      const surfaceCanContainPanel =
        surfaceRect.width >= panelRect.width + inset * 2;
      const minimumLeft = (surfaceCanContainPanel ? surfaceRect.left : 0) + inset;
      const maximumRight =
        (surfaceCanContainPanel ? surfaceRect.right : window.innerWidth) - inset;
      const desiredOffset =
        panelRect.width > maximumRight - minimumLeft
          ? (minimumLeft + maximumRight) / 2 -
            (naturalLeft + naturalRight) / 2
          : naturalLeft < minimumLeft
            ? minimumLeft - naturalLeft
            : naturalRight > maximumRight
              ? maximumRight - naturalRight
              : 0;
      if (Math.abs(horizontalOffsetRef.current - desiredOffset) >= 0.5) {
        horizontalOffsetRef.current = desiredOffset;
        setHorizontalOffset(desiredOffset);
      }
    };

    updatePlacement();
    const observer =
      typeof ResizeObserver === 'undefined'
        ? null
        : new ResizeObserver(updatePlacement);
    observer?.observe(surface);
    observer?.observe(positioner);
    window.addEventListener('resize', updatePlacement);
    return () => {
      observer?.disconnect();
      window.removeEventListener('resize', updatePlacement);
    };
  }, [nodeId, overlay]);

  const submit = (): void => {
    const result = dispatchCanvasAudioComposerSubmission({
      disabled,
      busy,
      prompt,
      hasModel: selectedModel !== null,
      requiredVoiceReady,
      promptLengthReady,
      retrySubmission,
      onGenerate,
      onRetrySubmission,
    });
    if (result === 'generated') setPrompt('');
  };

  const modelStatus =
    eligibleModelOptions.length === 0
      ? t('creativeStudio.canvas.audio.noModels', {
          defaultValue: '没有可用的语音合成模型，请先在模型管理中配置。',
        })
      : settings.model !== null && selectedModel === null
        ? t('creativeStudio.canvas.audio.modelUnavailable', {
            defaultValue: '已选语音合成模型当前不可用，请重新选择。',
          })
        : null;

  const content = (
    <div
      ref={positionerRef}
      className={styles.positioner}
      data-canvas-audio-composer
      data-overlay={overlay || undefined}
      data-placement={placement}
      data-node-id={nodeId}
      data-voice-profile={
        !voiceVisible ? 'unsupported' : voiceRequired ? 'required' : 'optional'
      }
      style={
        {
          '--creative-canvas-audio-composer-offset-x': `${horizontalOffset}px`,
          '--creative-canvas-audio-composer-overlay-left': `${overlayLayout.left}px`,
          '--creative-canvas-audio-composer-overlay-top': `${overlayLayout.top}px`,
          '--creative-canvas-audio-composer-overlay-width': `${overlayLayout.width}px`,
        } as React.CSSProperties
      }
      onMouseDown={(event) => event.stopPropagation()}
      onPointerDown={(event) => event.stopPropagation()}
      onDoubleClick={(event) => event.stopPropagation()}
      onWheel={(event) => event.stopPropagation()}
    >
      <div className={styles.panel}>
        <textarea
          className={styles.prompt}
          value={prompt}
          maxLength={normalizedMaxTextLength}
          placeholder={t('creativeStudio.canvas.audio.promptPlaceholder', {
            defaultValue: '输入要朗读的文本',
          })}
          aria-label={t('creativeStudio.canvas.audio.promptLabel', {
            defaultValue: '朗读文本',
          })}
          disabled={disabled}
          onChange={(event) => {
            setPrompt(event.target.value);
            onPromptChange(event.target.value);
          }}
          onKeyDown={(event) => {
            if (isCanvasAudioComposerSubmitKey(event.key, event.shiftKey)) {
              event.preventDefault();
              submit();
            }
          }}
        />

        <div className={styles.footer}>
          <div className={styles.controls}>
            <button
              type='button'
              className={styles.iconButton}
              aria-label={t('creativeStudio.canvas.audio.openPromptLibrary', {
                defaultValue: '打开朗读提示词库',
              })}
              title={t('creativeStudio.canvas.promptLibrary', {
                defaultValue: '提示词库',
              })}
              disabled={disabled}
              onClick={onOpenPromptLibrary}
            >
              <BookOne theme='outline' size={17} fill='currentColor' />
            </button>

            <Select
              className={styles.modelSelect}
              value={selectedModel ? modelKey(selectedModel) : undefined}
              placeholder={
                eligibleModelOptions.length > 0
                  ? t('creativeStudio.canvas.audio.selectModel', {
                      defaultValue: '选择语音合成模型',
                    })
                  : t('creativeStudio.canvas.audio.noModelOptions', {
                      defaultValue: '没有可用语音合成模型',
                    })
              }
              aria-label={t('creativeStudio.canvas.audio.modelLabel', {
                defaultValue: '语音合成模型',
              })}
              disabled={disabled || eligibleModelOptions.length === 0}
              getPopupContainer={popupContainer}
              onChange={(key) => {
                const option = findModel(key, eligibleModelOptions);
                onModelChange(
                  option
                    ? { providerId: option.providerId, model: option.model }
                    : null
                );
              }}
            >
              {eligibleModelOptions.map((option) => (
                <Select.Option key={modelKey(option)} value={modelKey(option)}>
                  {option.model} · {option.providerName}
                </Select.Option>
              ))}
            </Select>

            {settingsAvailable ? (
              <Popover
                trigger='click'
                position='top'
                getPopupContainer={popupContainer}
                content={
                  <div className={styles.settingsPanel}>
                    {voiceVisible ? (
                      <label className={styles.field}>
                        <span>
                          {t('creativeStudio.canvas.audio.voiceIdLabel', {
                            required: voiceRequired ? ' *' : '',
                            defaultValue: 'Voice ID{{required}}',
                          })}
                        </span>
                        <input
                          className={styles.voiceInput}
                          value={settings.voice}
                          maxLength={256}
                          placeholder={
                            voiceRequired
                              ? t('creativeStudio.canvas.audio.voiceIdPlaceholderRequired', {
                                  defaultValue: '输入 provider voice ID',
                                })
                              : t('creativeStudio.canvas.audio.voiceIdPlaceholder', {
                                  defaultValue: '使用模型默认音色，或输入 provider voice ID',
                                })
                          }
                          aria-label={t('creativeStudio.canvas.audio.voiceIdAriaLabel', {
                            defaultValue: '语音合成 Voice ID',
                          })}
                          aria-required={voiceRequired}
                          disabled={disabled}
                          onChange={(event) =>
                            onVoiceChange(event.target.value)
                          }
                        />
                      </label>
                    ) : null}
                    {formatVisible ? (
                      <label className={styles.field}>
                        <span>
                          {t('creativeStudio.canvas.audio.formatLabel', {
                            defaultValue: '音频格式',
                          })}
                        </span>
                        <select
                          className={styles.settingsSelect}
                          value={settings.format}
                          aria-label={t('creativeStudio.canvas.audio.formatAriaLabel', {
                            defaultValue: '音频格式',
                          })}
                          disabled={disabled}
                          onChange={(event) =>
                            onFormatChange(event.target.value as CanvasAudioFormat)
                          }
                        >
                          {AUDIO_FORMAT_OPTIONS.map((option) => (
                            <option key={option} value={option}>
                              {option.toUpperCase()}
                            </option>
                          ))}
                        </select>
                      </label>
                    ) : null}
                  </div>
                }
              >
                <button
                  type='button'
                  className={styles.settingsButton}
                  aria-label={t('creativeStudio.canvas.audio.settingsLabel', {
                    defaultValue: '语音生成设置',
                  })}
                  disabled={disabled}
                >
                  <SettingTwo theme='outline' size={15} fill='currentColor' />
                  <span className={styles.settingsSummary}>
                    {settingsSummary}
                  </span>
                </button>
              </Popover>
            ) : null}
          </div>

          <button
            type='button'
            className={`${styles.submitButton} ${
              retrySubmission ? styles.retrySubmitButton : ''
            }`}
            aria-label={
              retrySubmission
                ? t('creativeStudio.canvas.audio.retryLabel', {
                    defaultValue: '同键重试音频任务',
                  })
                : t('creativeStudio.canvas.audio.generateLabel', {
                    defaultValue: '生成音频',
                  })
            }
            disabled={!canSubmit}
            onClick={submit}
          >
            {busy && !retrySubmission ? (
              <Loading
                className={styles.spin}
                theme='outline'
                size={17}
                fill='currentColor'
              />
            ) : retrySubmission ? (
              <>
                <Refresh theme='outline' size={15} fill='currentColor' />
                <span className={styles.retryLabel}>
                  {t('creativeStudio.canvas.audio.retryShortLabel', {
                    defaultValue: '同键重试',
                  })}
                </span>
              </>
            ) : (
              <ArrowUp
                theme='outline'
                size={17}
                fill='currentColor'
                strokeWidth={4}
              />
            )}
          </button>
        </div>

        {voiceVisible && voiceRequired && !requiredVoiceReady ? (
          <div className={styles.message} role='status'>
            {t('creativeStudio.canvas.audio.voiceRequiredMessage', {
              defaultValue: '当前协议要求填写 provider Voice ID。',
            })}
          </div>
        ) : null}
        {!promptLengthReady ? (
          <div className={styles.message} role='status'>
            {t('creativeStudio.canvas.audio.promptTooLong', {
              max: normalizedMaxTextLength,
              defaultValue: `朗读文本不能超过 ${normalizedMaxTextLength} 个字符。`,
            })}
          </div>
        ) : null}
        {modelStatus ? (
          <div className={styles.message} role='status'>
            {modelStatus}
          </div>
        ) : null}
        {error || task.message ? (
          <div className={styles.message} role={error ? 'alert' : 'status'}>
            {error ?? task.message}
          </div>
        ) : null}
        {retrySubmission && onConfirmSubmission ? (
          <button
            type='button'
            className={styles.confirmSubmissionButton}
            disabled={disabled}
            onClick={onConfirmSubmission}
          >
            {t('creativeStudio.canvas.confirmTaskStatus', {
              defaultValue: '确认任务状态',
            })}
          </button>
        ) : null}
      </div>
    </div>
  );

  return (
    <>
      <span
        ref={anchorRef}
        hidden
        aria-hidden='true'
        data-canvas-audio-composer-anchor
        data-placement={placement}
      />
      {overlay && typeof document !== 'undefined'
        ? createPortal(content, document.body)
        : content}
    </>
  );
};

export default CreativeCanvasAudioComposer;
