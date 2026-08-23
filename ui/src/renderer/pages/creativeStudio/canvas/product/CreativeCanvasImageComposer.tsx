/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ArrowUp, BookOne, Loading, SettingTwo } from '@icon-park/react';
import { InputNumber, Popover, Radio, Select } from '@arco-design/web-react';
import React, { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';

import {
  DEFAULT_IMAGE_WORKBENCH_ASPECT_RATIOS,
  IMAGE_WORKBENCH_QUALITY_OPTIONS,
  imageWorkbenchModelKey,
  parseImageWorkbenchModelKey,
  type ImageWorkbenchAspectRatioOption,
  type ImageWorkbenchInterfaceMode,
  type ImageWorkbenchModelIdentity,
  type ImageWorkbenchModelOption,
  type ImageWorkbenchQuality,
  type ImageWorkbenchSettings,
  type ImageWorkbenchTaskSummary,
} from '../../workbenches/image';
import styles from './CreativeCanvasImageComposer.module.css';

export interface CreativeCanvasImageComposerProps {
  nodeId: string;
  hasImageContent: boolean;
  initialPrompt: string;
  settings: ImageWorkbenchSettings;
  modelOptions: readonly ImageWorkbenchModelOption[];
  task: ImageWorkbenchTaskSummary;
  disabled?: boolean;
  error?: string | null;
  retrySubmission?: boolean;
  onPromptChange?(prompt: string): void;
  onOpenPromptLibrary(): void;
  onModelChange(model: ImageWorkbenchModelIdentity | null): void;
  onInterfaceModeChange(mode: ImageWorkbenchInterfaceMode): void;
  onQualityChange(quality: ImageWorkbenchQuality): void;
  onDimensionsChange(dimensions: { width: number | null; height: number | null }): void;
  onAspectRatioChange(option: ImageWorkbenchAspectRatioOption): void;
  onCountChange(count: number): void;
  onGenerate(prompt: string): void;
  onRetrySubmission?(): void;
}

const clampDimension = (value: number | undefined): number =>
  Math.max(1, Math.min(8192, Math.floor(value || 1)));

const clampCount = (value: number | undefined): number =>
  Math.max(1, Math.min(10, Math.floor(value || 1)));

const popupContainer = (trigger: HTMLElement): HTMLElement =>
  (trigger.closest('[data-canvas-image-composer]') as HTMLElement | null) ??
  document.body;

const CreativeCanvasImageComposer: React.FC<CreativeCanvasImageComposerProps> = ({
  nodeId,
  hasImageContent,
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
  onInterfaceModeChange,
  onQualityChange,
  onDimensionsChange,
  onAspectRatioChange,
  onCountChange,
  onGenerate,
  onRetrySubmission,
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
  const qualityLabel =
    IMAGE_WORKBENCH_QUALITY_OPTIONS.find(
      (option) => option.value === settings.quality
    )?.label ??
    t('creativeStudio.canvas.image.autoQuality', {
      defaultValue: '自动',
    });
  const canGenerate = retrySubmission
    ? !disabled && onRetrySubmission !== undefined
    : !disabled && !busy && prompt.trim().length > 0 && settings.model !== null;
  const modelValue = settings.model ? imageWorkbenchModelKey(settings.model) : undefined;

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

      setOverlay((current) => (current === shouldOverlay ? current : shouldOverlay));
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
                  Math.min(window.innerHeight - panelHeight - inset, nodeRect.top)
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
    if (retrySubmission && onRetrySubmission) {
      onRetrySubmission();
      return;
    }
    const value = prompt.trim();
    if (!canGenerate || !value) return;
    onGenerate(value);
    setPrompt('');
  };

  const content = (
    <div
      ref={positionerRef}
      className={styles.positioner}
      data-canvas-image-composer
      data-overlay={overlay || undefined}
      data-placement={placement}
      data-node-id={nodeId}
      style={
        {
          '--creative-canvas-image-composer-offset-x': `${horizontalOffset}px`,
          '--creative-canvas-image-composer-overlay-left': `${overlayLayout.left}px`,
          '--creative-canvas-image-composer-overlay-top': `${overlayLayout.top}px`,
          '--creative-canvas-image-composer-overlay-width': `${overlayLayout.width}px`,
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
          maxLength={1_000_000}
          placeholder={
            hasImageContent
              ? t('creativeStudio.canvas.image.editPromptPlaceholder', {
                  defaultValue: '请输入你想要把这张图修改成什么',
                })
              : t('creativeStudio.canvas.image.generatePromptPlaceholder', {
                  defaultValue: '描述要生成的图片内容',
                })
          }
          aria-label={t('creativeStudio.canvas.image.promptLabel', {
            defaultValue: '图片创作提示词',
          })}
          disabled={disabled}
          onChange={(event) => {
            setPrompt(event.target.value);
            onPromptChange?.(event.target.value);
          }}
          onKeyDown={(event) => {
            if (event.key === 'Enter' && !event.shiftKey) {
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
              aria-label={t('creativeStudio.canvas.openPromptLibrary', {
                defaultValue: '打开提示词库',
              })}
              disabled={disabled}
              onClick={onOpenPromptLibrary}
            >
              <BookOne theme='outline' size={17} fill='currentColor' />
            </button>

            <Select
              className={styles.modelSelect}
              value={modelValue}
              placeholder={
                modelOptions.length > 0
                  ? hasImageContent
                    ? t('creativeStudio.canvas.image.selectEditModel', {
                        defaultValue: '选择图片编辑模型',
                      })
                    : t('creativeStudio.canvas.image.selectGenerateModel', {
                        defaultValue: '选择图片生成模型',
                      })
                  : hasImageContent
                    ? t('creativeStudio.canvas.image.noEditModels', {
                        defaultValue: '没有可用图片编辑模型',
                      })
                    : t('creativeStudio.canvas.image.noGenerateModels', {
                        defaultValue: '没有可用图片生成模型',
                      })
              }
              aria-label={
                hasImageContent
                  ? t('creativeStudio.canvas.image.editModelLabel', {
                      defaultValue: '图片编辑模型',
                    })
                  : t('creativeStudio.canvas.image.generateModelLabel', {
                      defaultValue: '图片生成模型',
                    })
              }
              disabled={disabled || modelOptions.length === 0}
              getPopupContainer={popupContainer}
              onChange={(key) =>
                onModelChange(
                  typeof key === 'string'
                    ? parseImageWorkbenchModelKey(key, modelOptions)
                    : null
                )
              }
            >
              {modelOptions.map((option) => (
                <Select.Option
                  key={imageWorkbenchModelKey(option)}
                  value={imageWorkbenchModelKey(option)}
                  disabled={option.disabled}
                >
                  {option.label}
                  {option.providerLabel ? ` · ${option.providerLabel}` : ''}
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
                      {t('creativeStudio.canvas.image.interfaceModeLabel', {
                        defaultValue: '接口模式',
                      })}
                    </span>
                    <Radio.Group
                      type='button'
                      size='small'
                      value={settings.interfaceMode}
                      disabled={disabled}
                      onChange={(value) =>
                        onInterfaceModeChange(value as ImageWorkbenchInterfaceMode)
                      }
                    >
                      <Radio value='images'>{t('creativeStudio.image.interface.images')}</Radio>
                      <Radio value='responses'>
                        {t('creativeStudio.image.interface.responses')}
                      </Radio>
                    </Radio.Group>
                  </label>
                  <label className={styles.field}>
                    <span>
                      {t('creativeStudio.canvas.image.qualityLabel', {
                        defaultValue: '质量',
                      })}
                    </span>
                    <Select
                      value={settings.quality}
                      disabled={disabled}
                      getPopupContainer={popupContainer}
                      onChange={(value) => onQualityChange(value as ImageWorkbenchQuality)}
                    >
                      {IMAGE_WORKBENCH_QUALITY_OPTIONS.map((option) => (
                        <Select.Option key={option.value} value={option.value}>
                          {option.label}
                        </Select.Option>
                      ))}
                    </Select>
                  </label>
                  <label className={styles.field}>
                    <span>
                      {t('creativeStudio.canvas.image.aspectRatioLabel', {
                        defaultValue: '宽高比',
                      })}
                    </span>
                    <Select
                      value={settings.aspectRatio}
                      disabled={disabled}
                      getPopupContainer={popupContainer}
                      onChange={(value) => {
                        const option = DEFAULT_IMAGE_WORKBENCH_ASPECT_RATIOS.find(
                          (candidate) => candidate.value === value
                        );
                        if (option) onAspectRatioChange(option);
                      }}
                    >
                      {DEFAULT_IMAGE_WORKBENCH_ASPECT_RATIOS.map((option) => (
                        <Select.Option key={option.value} value={option.value} disabled={option.disabled}>
                          {option.label}
                        </Select.Option>
                      ))}
                    </Select>
                  </label>
                  <div className={styles.dimensionRow}>
                    <label className={styles.field}>
                      <span>
                        {t('creativeStudio.canvas.image.widthLabel', {
                          defaultValue: '宽度',
                        })}
                      </span>
                      <InputNumber
                        value={settings.width ?? undefined}
                        min={1}
                        max={8192}
                        placeholder={t('creativeStudio.canvas.autoValue', {
                          defaultValue: '自动',
                        })}
                        disabled={disabled || settings.width === null}
                        onChange={(value) =>
                          onDimensionsChange({
                            width: clampDimension(value),
                            height: settings.height,
                          })
                        }
                      />
                    </label>
                    <label className={styles.field}>
                      <span>
                        {t('creativeStudio.canvas.image.heightLabel', {
                          defaultValue: '高度',
                        })}
                      </span>
                      <InputNumber
                        value={settings.height ?? undefined}
                        min={1}
                        max={8192}
                        placeholder={t('creativeStudio.canvas.autoValue', {
                          defaultValue: '自动',
                        })}
                        disabled={disabled || settings.height === null}
                        onChange={(value) =>
                          onDimensionsChange({
                            width: settings.width,
                            height: clampDimension(value),
                          })
                        }
                      />
                    </label>
                  </div>
                  <label className={styles.field}>
                    <span>
                      {t('creativeStudio.canvas.image.countLabel', {
                        defaultValue: '生成张数',
                      })}
                    </span>
                    <InputNumber
                      value={settings.count}
                      min={1}
                      max={10}
                      disabled={disabled}
                      onChange={(value) => onCountChange(clampCount(value))}
                    />
                  </label>
                </div>
              }
            >
              <button
                type='button'
                className={styles.settingsButton}
                aria-label={t('creativeStudio.canvas.image.settingsLabel', {
                  defaultValue: '图片生成设置',
                })}
                disabled={disabled}
              >
                <SettingTwo theme='outline' size={15} fill='currentColor' />
                <span className={styles.settingsSummary}>
                  {t('creativeStudio.canvas.image.settingsSummary', {
                    quality: qualityLabel,
                    aspectRatio: settings.aspectRatio,
                    count: settings.count,
                    defaultValue: `${qualityLabel} · ${settings.aspectRatio} · ${settings.count} 张`,
                  })}
                </span>
              </button>
            </Popover>
          </div>

          <button
            type='button'
            className={styles.submitButton}
            aria-label={t('creativeStudio.canvas.image.generateLabel', {
              defaultValue: '生成图片',
            })}
            disabled={!canGenerate}
            onClick={submit}
          >
            {busy ? (
              <Loading className={styles.spin} theme='outline' size={17} fill='currentColor' />
            ) : (
              <ArrowUp theme='outline' size={17} fill='currentColor' strokeWidth={4} />
            )}
          </button>
        </div>

        {error || task.message ? (
          <div className={styles.message} role={error ? 'alert' : 'status'}>
            {error ?? task.message}
          </div>
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
        data-canvas-image-composer-anchor
        data-placement={placement}
      />
      {overlay && typeof document !== 'undefined'
        ? createPortal(content, document.body)
        : content}
    </>
  );
};

export default CreativeCanvasImageComposer;
