/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ArrowUp, BookOne, Loading, SettingTwo } from '@icon-park/react';
import { Popover, Select } from '@arco-design/web-react';
import React, { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

import CreativeMediaPreview from '../../assets/components/CreativeMediaPreview';
import type {
  CreativeModelOption,
  CreativeModelSelectionRef,
} from '../../models';
import type {
  CanvasVideoComposeSettings,
  CanvasVideoComposeTaskSummary,
} from './canvasVideoComposerCanvas';
import CreativeCanvasComposerShell from './CreativeCanvasComposerShell';
import composerStyles from './CreativeCanvasComposerShell.module.css';
import styles from './CreativeCanvasVideoComposer.module.css';

export type CanvasVideoComposerMode = 't2v' | 'i2v' | 'unsupported';
export type CanvasVideoResolution = '720p' | '1080p';
export type CanvasVideoAspectRatio = '16:9' | '9:16' | '1:1';
export type CanvasVideoSeconds = 5 | 10;

export interface CanvasVideoReferenceSummary {
  name: string;
  previewUrl?: string | null;
  originalUrl?: string | null;
}

export interface CreativeCanvasVideoComposerProps {
  nodeId: string;
  mode: CanvasVideoComposerMode;
  reference?: CanvasVideoReferenceSummary | null;
  initialPrompt: string;
  settings: CanvasVideoComposeSettings;
  modelOptions: readonly CreativeModelOption[];
  task: CanvasVideoComposeTaskSummary;
  disabled?: boolean;
  error?: string | null;
  retrySubmission?: boolean;
  onPromptChange?(prompt: string): void;
  onOpenPromptLibrary(): void;
  onModelChange(model: CreativeModelSelectionRef | null): void;
  onResolutionChange(resolution: CanvasVideoResolution): void;
  onAspectRatioChange(aspectRatio: CanvasVideoAspectRatio): void;
  onSecondsChange(seconds: CanvasVideoSeconds): void;
  onGenerate(prompt: string): void;
  onRetrySubmission?(): void;
  onConfirmSubmission?(): void;
}

export interface CanvasVideoComposerSubmissionInput {
  mode: CanvasVideoComposerMode;
  disabled: boolean;
  busy: boolean;
  prompt: string;
  hasModel: boolean;
  retrySubmission: boolean;
  onGenerate(prompt: string): void;
  onRetrySubmission?(): void;
}

export type CanvasVideoComposerSubmissionResult =
  | 'generated'
  | 'retried'
  | null;

const RESOLUTION_OPTIONS: readonly CanvasVideoResolution[] = ['720p', '1080p'];
const ASPECT_RATIO_OPTIONS: readonly CanvasVideoAspectRatio[] = [
  '16:9',
  '9:16',
  '1:1',
];
const SECONDS_OPTIONS: readonly CanvasVideoSeconds[] = [5, 10];

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
  (trigger.closest('[data-canvas-video-composer]') as HTMLElement | null) ??
  document.body;

export const isCanvasVideoComposerSubmitKey = (
  key: string,
  shiftKey: boolean
): boolean => key === 'Enter' && !shiftKey;

/**
 * Keep retry and generation mutually exclusive. A retry reuses the canonical
 * task identity and therefore never needs the draft prompt or model again.
 */
export function dispatchCanvasVideoComposerSubmission(
  input: CanvasVideoComposerSubmissionInput
): CanvasVideoComposerSubmissionResult {
  if (input.disabled || input.mode === 'unsupported') return null;
  if (input.retrySubmission) {
    if (!input.onRetrySubmission) return null;
    input.onRetrySubmission();
    return 'retried';
  }
  const prompt = input.prompt.trim();
  if (input.busy || !input.hasModel || !prompt) return null;
  input.onGenerate(prompt);
  return 'generated';
}

const CreativeCanvasVideoComposer: React.FC<
  CreativeCanvasVideoComposerProps
> = ({
  nodeId,
  mode,
  reference,
  initialPrompt,
  settings,
  modelOptions,
  task,
  disabled = false,
  error,
  retrySubmission = false,
  onPromptChange,
  onOpenPromptLibrary,
  onModelChange,
  onResolutionChange,
  onAspectRatioChange,
  onSecondsChange,
  onGenerate,
  onRetrySubmission,
  onConfirmSubmission,
}) => {
  const { t } = useTranslation();
  const [prompt, setPrompt] = useState(initialPrompt);
  const busy = task.state === 'queued' || task.state === 'running';
  const unsupported = mode === 'unsupported';
  const interactionDisabled = disabled || unsupported;
  const selectedModel = settings.model
    ? modelOptions.find((option) => modelKey(option) === modelKey(settings.model!)) ??
      null
    : null;
  const canSubmit = retrySubmission
    ? !interactionDisabled && onRetrySubmission !== undefined
    : !interactionDisabled &&
      !busy &&
      prompt.trim().length > 0 &&
      selectedModel !== null;
  const unsupportedModeLabel = unsupported
    ? t('creativeStudio.canvas.video.unsupportedMode', {
        defaultValue: '当前节点不支持视频生成',
      })
    : null;

  useEffect(() => setPrompt(initialPrompt), [initialPrompt, nodeId]);

  const submit = (): void => {
    const result = dispatchCanvasVideoComposerSubmission({
      mode,
      disabled,
      busy,
      prompt,
      hasModel: selectedModel !== null,
      retrySubmission,
      onGenerate,
      onRetrySubmission,
    });
    if (result === 'generated') setPrompt('');
  };

  const modelStatus =
    modelOptions.length === 0
      ? t('creativeStudio.canvas.video.noModels', {
          defaultValue: '没有可用的视频生成模型，请先在模型管理中配置。',
        })
      : settings.model !== null && selectedModel === null
        ? t('creativeStudio.canvas.video.modelUnavailable', {
            defaultValue: '已选视频模型当前不可用，请重新选择。',
          })
        : null;

  return (
    <CreativeCanvasComposerShell
      kind='video'
      nodeId={nodeId}
      mode={mode}
    >
        {unsupportedModeLabel || (mode === 'i2v' && reference) ? (
          <div className={styles.contextRow}>
            {unsupportedModeLabel ? (
              <span className={styles.modePill}>{unsupportedModeLabel}</span>
            ) : null}
            {mode === 'i2v' && reference ? (
              <span className={styles.reference} title={reference.name}>
                {reference.previewUrl || reference.originalUrl ? (
                  <span className={styles.referencePreview}>
                    <CreativeMediaPreview
                      kind='image'
                      src={reference.originalUrl ?? reference.previewUrl}
                      posterSrc={reference.previewUrl}
                      alt=''
                    />
                  </span>
                ) : null}
                <span className={styles.referenceName}>{reference.name}</span>
              </span>
            ) : null}
          </div>
        ) : null}

        <textarea
          className={composerStyles.prompt}
          value={prompt}
          maxLength={1_000_000}
          placeholder={
            mode === 'i2v'
              ? t('creativeStudio.canvas.video.i2vPromptPlaceholder', {
                  defaultValue: '描述参考图要如何运动、变化与运镜',
                })
              : t('creativeStudio.canvas.video.t2vPromptPlaceholder', {
                  defaultValue: '描述要生成的视频内容、动作与镜头',
                })
          }
          aria-label={t('creativeStudio.canvas.video.promptLabel', {
            defaultValue: '视频创作提示词',
          })}
          disabled={interactionDisabled}
          onChange={(event) => {
            setPrompt(event.target.value);
            onPromptChange?.(event.target.value);
          }}
          onKeyDown={(event) => {
            if (isCanvasVideoComposerSubmitKey(event.key, event.shiftKey)) {
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
              aria-label={t('creativeStudio.canvas.video.openPromptLibrary', {
                defaultValue: '打开视频提示词库',
              })}
              title={t('creativeStudio.canvas.promptLibrary', {
                defaultValue: '提示词库',
              })}
              disabled={interactionDisabled}
              onClick={onOpenPromptLibrary}
            >
              <BookOne theme='outline' size={17} fill='currentColor' />
            </button>

            <Select
              className={composerStyles.modelSelect}
              size='mini'
              value={selectedModel ? modelKey(selectedModel) : undefined}
              placeholder={
                modelOptions.length > 0
                  ? t('creativeStudio.canvas.video.selectModel', {
                      defaultValue: '选择视频生成模型',
                    })
                  : t('creativeStudio.canvas.video.noModelOptions', {
                      defaultValue: '没有可用视频生成模型',
                    })
              }
              aria-label={t('creativeStudio.canvas.video.modelLabel', {
                defaultValue: '视频生成模型',
              })}
              disabled={interactionDisabled || modelOptions.length === 0}
              getPopupContainer={popupContainer}
              onChange={(key) => {
                const option = findModel(key, modelOptions);
                onModelChange(
                  option
                    ? { providerId: option.providerId, model: option.model }
                    : null
                );
              }}
            >
              {modelOptions.map((option) => (
                <Select.Option key={modelKey(option)} value={modelKey(option)}>
                  {option.model} · {option.providerName}
                </Select.Option>
              ))}
            </Select>

            <Popover
              trigger='click'
              position='top'
              getPopupContainer={popupContainer}
              content={
                <div className={composerStyles.popoverSettingsPanel}>
                  <label className={composerStyles.field}>
                    <span>
                      {t('creativeStudio.canvas.video.resolutionLabel', {
                        defaultValue: '分辨率',
                      })}
                    </span>
                    <select
                      className={composerStyles.settingsControl}
                      value={settings.resolution}
                      aria-label={t('creativeStudio.canvas.video.resolutionAriaLabel', {
                        defaultValue: '视频分辨率',
                      })}
                      disabled={interactionDisabled}
                      onChange={(event) =>
                        onResolutionChange(event.target.value as CanvasVideoResolution)
                      }
                    >
                      {RESOLUTION_OPTIONS.map((option) => (
                        <option key={option} value={option}>
                          {option}
                        </option>
                      ))}
                    </select>
                  </label>
                  <label className={composerStyles.field}>
                    <span>
                      {t('creativeStudio.canvas.video.aspectRatioLabel', {
                        defaultValue: '宽高比',
                      })}
                    </span>
                    <select
                      className={composerStyles.settingsControl}
                      value={settings.aspectRatio}
                      aria-label={t('creativeStudio.canvas.video.aspectRatioAriaLabel', {
                        defaultValue: '视频宽高比',
                      })}
                      disabled={interactionDisabled}
                      onChange={(event) =>
                        onAspectRatioChange(event.target.value as CanvasVideoAspectRatio)
                      }
                    >
                      {ASPECT_RATIO_OPTIONS.map((option) => (
                        <option key={option} value={option}>
                          {option}
                        </option>
                      ))}
                    </select>
                  </label>
                  <label className={composerStyles.field}>
                    <span>
                      {t('creativeStudio.canvas.video.durationLabel', {
                        defaultValue: '时长',
                      })}
                    </span>
                    <select
                      className={composerStyles.settingsControl}
                      value={settings.seconds}
                      aria-label={t('creativeStudio.canvas.video.durationAriaLabel', {
                        defaultValue: '视频时长',
                      })}
                      disabled={interactionDisabled}
                      onChange={(event) =>
                        onSecondsChange(Number(event.target.value) as CanvasVideoSeconds)
                      }
                    >
                      {SECONDS_OPTIONS.map((option) => (
                        <option key={option} value={option}>
                          {t('creativeStudio.canvas.video.secondsOption', {
                            seconds: option,
                            defaultValue: `${option} 秒`,
                          })}
                        </option>
                      ))}
                    </select>
                  </label>
                </div>
              }
            >
              <button
                type='button'
                className={`${composerStyles.controlButton} ${composerStyles.settingsButton}`}
                aria-label={t('creativeStudio.canvas.video.settingsLabel', {
                  defaultValue: '视频生成设置',
                })}
                disabled={interactionDisabled}
              >
                <SettingTwo theme='outline' size={15} fill='currentColor' />
                <span className={composerStyles.settingsSummary}>
                  {settings.resolution} · {settings.aspectRatio} ·{' '}
                  {t('creativeStudio.canvas.video.secondsSummary', {
                    seconds: settings.seconds,
                    defaultValue: `${settings.seconds} 秒`,
                  })}
                </span>
              </button>
            </Popover>
          </div>

          <button
            type='button'
            className={`${composerStyles.controlButton} ${composerStyles.submitButton}`}
            aria-label={t('creativeStudio.canvas.video.generateLabel', {
              defaultValue: '生成视频',
            })}
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

        {unsupported ? (
          <div
            className={`${composerStyles.message} ${styles.unsupported}`}
            role='status'
          >
            {t('creativeStudio.canvas.video.unsupportedMessage', {
              defaultValue:
                '当前节点不支持直接生成视频。请选择空视频节点，或为它添加一张图片参考。',
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

export default CreativeCanvasVideoComposer;
