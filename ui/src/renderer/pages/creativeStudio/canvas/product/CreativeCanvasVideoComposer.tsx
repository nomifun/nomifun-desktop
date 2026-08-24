/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ArrowUp, BookOne, Loading, SettingTwo } from '@icon-park/react';
import { Popover, Select } from '@arco-design/web-react';
import React, { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';

import type {
  CreativeModelOption,
  CreativeModelSelectionRef,
} from '../../models';
import { videoWorkbenchSizeOptionLabel } from '../../workbenches/video';
import type {
  CanvasVideoComposeSettings,
  CanvasVideoComposeTaskSummary,
} from './canvasVideoComposerCanvas';
import styles from './CreativeCanvasVideoComposer.module.css';

export type CanvasVideoComposerMode = 't2v' | 'i2v' | 'unsupported';
export type CanvasVideoResolution = '720p' | '1080p';
export type CanvasVideoAspectRatio = '16:9' | '9:16' | '1:1';
export type CanvasVideoSeconds = 5 | 10;

export interface CanvasVideoReferenceSummary {
  name: string;
  previewUrl?: string | null;
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
  const modeLabel =
    mode === 't2v'
      ? t('creativeStudio.canvas.video.textToVideo', {
          defaultValue: '文生视频',
        })
      : mode === 'i2v'
        ? t('creativeStudio.canvas.video.imageToVideo', {
            defaultValue: '图生视频·1张参考图',
          })
        : t('creativeStudio.canvas.video.unsupportedMode', {
            defaultValue: '当前节点不支持视频生成',
          });

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

  const content = (
    <div
      ref={positionerRef}
      className={styles.positioner}
      data-canvas-video-composer
      data-overlay={overlay || undefined}
      data-placement={placement}
      data-mode={mode}
      data-node-id={nodeId}
      style={
        {
          '--creative-canvas-video-composer-offset-x': `${horizontalOffset}px`,
          '--creative-canvas-video-composer-overlay-left': `${overlayLayout.left}px`,
          '--creative-canvas-video-composer-overlay-top': `${overlayLayout.top}px`,
          '--creative-canvas-video-composer-overlay-width': `${overlayLayout.width}px`,
        } as React.CSSProperties
      }
      onMouseDown={(event) => event.stopPropagation()}
      onPointerDown={(event) => event.stopPropagation()}
      onDoubleClick={(event) => event.stopPropagation()}
      onWheel={(event) => event.stopPropagation()}
    >
      <div className={styles.panel}>
        <div className={styles.contextRow}>
          <span className={styles.modePill}>{modeLabel}</span>
          {mode === 'i2v' && reference ? (
            <span className={styles.reference} title={reference.name}>
              {reference.previewUrl ? (
                <img
                  className={styles.referencePreview}
                  src={reference.previewUrl}
                  alt=''
                />
              ) : null}
              <span className={styles.referenceName}>{reference.name}</span>
            </span>
          ) : null}
        </div>

        <textarea
          className={styles.prompt}
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

        <div className={styles.footer}>
          <div className={styles.controls}>
            <button
              type='button'
              className={styles.iconButton}
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
              className={styles.modelSelect}
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
                <div className={styles.settingsPanel}>
                  <label className={styles.field}>
                    <span>
                      {t('creativeStudio.canvas.video.resolutionLabel', {
                        defaultValue: '分辨率',
                      })}
                    </span>
                    <select
                      className={styles.settingsSelect}
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
                  <label className={styles.field}>
                    <span>
                      {t('creativeStudio.canvas.video.aspectRatioLabel', {
                        defaultValue: '画幅',
                      })}
                    </span>
                    <select
                      className={styles.settingsSelect}
                      value={settings.aspectRatio}
                      aria-label={t('creativeStudio.canvas.video.aspectRatioAriaLabel', {
                        defaultValue: '视频画幅',
                      })}
                      disabled={interactionDisabled}
                      onChange={(event) =>
                        onAspectRatioChange(event.target.value as CanvasVideoAspectRatio)
                      }
                    >
                      {ASPECT_RATIO_OPTIONS.map((option) => (
                        <option key={option} value={option}>
                          {videoWorkbenchSizeOptionLabel(
                            settings.resolution,
                            option
                          )}
                        </option>
                      ))}
                    </select>
                  </label>
                  <label className={styles.field}>
                    <span>
                      {t('creativeStudio.canvas.video.durationLabel', {
                        defaultValue: '时长',
                      })}
                    </span>
                    <select
                      className={styles.settingsSelect}
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
                className={styles.settingsButton}
                aria-label={t('creativeStudio.canvas.video.settingsLabel', {
                  defaultValue: '视频生成设置',
                })}
                disabled={interactionDisabled}
              >
                <SettingTwo theme='outline' size={15} fill='currentColor' />
                <span className={styles.settingsSummary}>
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
            className={styles.submitButton}
            aria-label={t('creativeStudio.canvas.video.generateLabel', {
              defaultValue: '生成视频',
            })}
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
          <div className={styles.unsupported} role='status'>
            {t('creativeStudio.canvas.video.unsupportedMessage', {
              defaultValue:
                '当前节点不支持直接生成视频。请选择空视频节点，或为它添加一张图片参考。',
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
        data-canvas-video-composer-anchor
        data-placement={placement}
      />
      {overlay && typeof document !== 'undefined'
        ? createPortal(content, document.body)
        : content}
    </>
  );
};

export default CreativeCanvasVideoComposer;
