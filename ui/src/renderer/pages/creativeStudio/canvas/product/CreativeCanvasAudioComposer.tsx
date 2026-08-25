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
import React, { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { CreativeGenerationStatus } from '../../domain';
import type {
  CreativeModelOption,
  CreativeModelSelectionRef,
} from '../../models';
import CreativeCanvasComposerShell from './CreativeCanvasComposerShell';
import composerStyles from './CreativeCanvasComposerShell.module.css';

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
  const [prompt, setPrompt] = useState(initialPrompt);
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

  return (
    <CreativeCanvasComposerShell
      kind='audio'
      nodeId={nodeId}
      voiceProfile={
        !voiceVisible ? 'unsupported' : voiceRequired ? 'required' : 'optional'
      }
    >
        <textarea
          className={composerStyles.prompt}
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

        <div className={composerStyles.footer}>
          <div className={composerStyles.controls}>
            <button
              type='button'
              className={`${composerStyles.controlButton} ${composerStyles.iconButton}`}
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
              className={composerStyles.modelSelect}
              size='mini'
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
                  <div className={composerStyles.popoverSettingsPanel}>
                    {voiceVisible ? (
                      <label className={composerStyles.field}>
                        <span>
                          {t('creativeStudio.canvas.audio.voiceIdLabel', {
                            required: voiceRequired ? ' *' : '',
                            defaultValue: 'Voice ID{{required}}',
                          })}
                        </span>
                        <input
                          className={composerStyles.settingsControl}
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
                      <label className={composerStyles.field}>
                        <span>
                          {t('creativeStudio.canvas.audio.formatLabel', {
                            defaultValue: '音频格式',
                          })}
                        </span>
                        <select
                          className={composerStyles.settingsControl}
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
                  className={`${composerStyles.controlButton} ${composerStyles.settingsButton}`}
                  aria-label={t('creativeStudio.canvas.audio.settingsLabel', {
                    defaultValue: '语音生成设置',
                  })}
                  disabled={disabled}
                >
                  <SettingTwo theme='outline' size={15} fill='currentColor' />
                  <span className={composerStyles.settingsSummary}>
                    {settingsSummary}
                  </span>
                </button>
              </Popover>
            ) : null}
          </div>

          <button
            type='button'
            className={`${composerStyles.controlButton} ${composerStyles.submitButton} ${
              retrySubmission ? composerStyles.retrySubmitButton : ''
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
                className={composerStyles.spin}
                theme='outline'
                size={17}
                fill='currentColor'
              />
            ) : retrySubmission ? (
              <>
                <Refresh theme='outline' size={15} fill='currentColor' />
                <span className={composerStyles.retryLabel}>
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
          <div className={composerStyles.message} role='status'>
            {t('creativeStudio.canvas.audio.voiceRequiredMessage', {
              defaultValue: '当前协议要求填写 provider Voice ID。',
            })}
          </div>
        ) : null}
        {!promptLengthReady ? (
          <div className={composerStyles.message} role='status'>
            {t('creativeStudio.canvas.audio.promptTooLong', {
              max: normalizedMaxTextLength,
              defaultValue: `朗读文本不能超过 ${normalizedMaxTextLength} 个字符。`,
            })}
          </div>
        ) : null}
        {modelStatus ? (
          <div className={composerStyles.message} role='status'>
            {modelStatus}
          </div>
        ) : null}
        {error || task.message ? (
          <div
            className={composerStyles.message}
            role={error ? 'alert' : 'status'}
          >
            {error ?? task.message}
          </div>
        ) : null}
        {retrySubmission && onConfirmSubmission ? (
          <button
            type='button'
            className={composerStyles.confirmSubmissionButton}
            disabled={disabled}
            onClick={onConfirmSubmission}
          >
            {t('creativeStudio.canvas.confirmTaskStatus', {
              defaultValue: '确认任务状态',
            })}
          </button>
        ) : null}
    </CreativeCanvasComposerShell>
  );
};

export default CreativeCanvasAudioComposer;
