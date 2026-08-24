/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ArrowUp, BookOne, Check, Down, Loading, SettingTwo } from '@icon-park/react';
import { InputNumber, Radio, Select } from '@arco-design/web-react';
import React, { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';

import {
  IMAGE_WORKBENCH_QUALITY_OPTIONS,
  imageWorkbenchSizeDimensionsLabel,
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
  aspectRatioOptions: readonly ImageWorkbenchAspectRatioOption[];
  maxCount: number;
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
  onAspectRatioChange(option: ImageWorkbenchAspectRatioOption): void;
  onCountChange(count: number): void;
  onGenerate(prompt: string): void;
  onRetrySubmission?(): void;
}

const clampCount = (value: number | undefined, maxCount: number): number =>
  Math.max(1, Math.min(maxCount, Math.floor(value || 1)));

const popupContainer = (trigger: HTMLElement): HTMLElement =>
  (trigger.closest('[data-canvas-image-composer]') as HTMLElement | null) ??
  document.body;

const CreativeCanvasImageComposer: React.FC<CreativeCanvasImageComposerProps> = ({
  nodeId,
  hasImageContent,
  initialPrompt,
  settings,
  aspectRatioOptions,
  maxCount,
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
  onAspectRatioChange,
  onCountChange,
  onGenerate,
  onRetrySubmission,
}) => {
  const { t } = useTranslation();
  const positionerRef = useRef<HTMLDivElement>(null);
  const anchorRef = useRef<HTMLSpanElement>(null);
  const settingsHostRef = useRef<HTMLDivElement>(null);
  const sizeSelectRef = useRef<HTMLDivElement>(null);
  const horizontalOffsetRef = useRef(0);
  const [prompt, setPrompt] = useState(initialPrompt);
  const [placement, setPlacement] = useState<'above' | 'below'>('below');
  const [horizontalOffset, setHorizontalOffset] = useState(0);
  const [overlay, setOverlay] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [sizeMenuOpen, setSizeMenuOpen] = useState(false);
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
  const selectedSizeOption =
    aspectRatioOptions.find(
      (option) => !option.disabled && option.value === settings.aspectRatio
    ) ?? aspectRatioOptions.find((option) => !option.disabled) ?? null;

  useEffect(() => setPrompt(initialPrompt), [initialPrompt, nodeId]);

  useEffect(() => {
    setSettingsOpen(false);
    setSizeMenuOpen(false);
  }, [nodeId]);

  useEffect(() => {
    if (disabled) {
      setSettingsOpen(false);
      setSizeMenuOpen(false);
    }
  }, [disabled]);

  useEffect(() => {
    if (!settingsOpen) return;
    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (target instanceof Node) {
        if (sizeMenuOpen && !sizeSelectRef.current?.contains(target)) {
          setSizeMenuOpen(false);
        }
        if (!settingsHostRef.current?.contains(target)) {
          setSettingsOpen(false);
        }
      }
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return;
      if (sizeMenuOpen) setSizeMenuOpen(false);
      else setSettingsOpen(false);
    };
    document.addEventListener('pointerdown', handlePointerDown, true);
    document.addEventListener('keydown', handleKeyDown);
    return () => {
      document.removeEventListener('pointerdown', handlePointerDown, true);
      document.removeEventListener('keydown', handleKeyDown);
    };
  }, [settingsOpen, sizeMenuOpen]);

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
              size='mini'
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

            <div ref={settingsHostRef} className={styles.settingsHost}>
              {settingsOpen ? (
                <div
                  id={`canvas-image-settings-${nodeId}`}
                  className={styles.settingsPopover}
                  role='dialog'
                  aria-label={t('creativeStudio.canvas.image.settingsLabel', {
                    defaultValue: '图片生成设置',
                  })}
                >
                <div className={styles.settingsPanel}>
                  <div className={styles.field}>
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
                  </div>
                  <label className={styles.field}>
                    <span>
                      {t('creativeStudio.canvas.image.qualityLabel', {
                        defaultValue: '质量',
                      })}
                    </span>
                    <span className={styles.settingsSelect}>
                      <select
                        value={settings.quality}
                        aria-label={t('creativeStudio.canvas.image.qualityLabel', {
                          defaultValue: '质量',
                        })}
                        disabled={disabled}
                        onChange={(event) =>
                          onQualityChange(event.target.value as ImageWorkbenchQuality)
                        }
                      >
                        {IMAGE_WORKBENCH_QUALITY_OPTIONS.map((option) => (
                          <option key={option.value} value={option.value}>
                            {option.label}
                          </option>
                        ))}
                      </select>
                      <Down theme='outline' size={12} fill='currentColor' />
                    </span>
                  </label>
                  <div className={styles.field}>
                    <span>
                      {t('creativeStudio.canvas.image.aspectRatioLabel', {
                        defaultValue: '宽高比',
                      })}
                    </span>
                    <div ref={sizeSelectRef} className={styles.sizeSelect}>
                      <button
                        type='button'
                        className={styles.sizeSelectTrigger}
                        aria-label={t('creativeStudio.canvas.image.aspectRatioLabel', {
                          defaultValue: '宽高比',
                        })}
                        aria-haspopup='listbox'
                        aria-expanded={sizeMenuOpen}
                        data-open={sizeMenuOpen || undefined}
                        aria-controls={`canvas-image-size-options-${nodeId}`}
                        disabled={disabled || !selectedSizeOption}
                        onClick={() => setSizeMenuOpen((open) => !open)}
                      >
                        <span>{selectedSizeOption?.label ?? settings.aspectRatio}</span>
                        <small>
                          {selectedSizeOption
                            ? imageWorkbenchSizeDimensionsLabel(selectedSizeOption)
                            : null}
                        </small>
                        <Down theme='outline' size={12} fill='currentColor' />
                      </button>
                      {sizeMenuOpen ? (
                        <div
                          id={`canvas-image-size-options-${nodeId}`}
                          className={styles.sizeMenu}
                          role='listbox'
                          aria-label={t('creativeStudio.canvas.image.aspectRatioLabel', {
                            defaultValue: '宽高比',
                          })}
                        >
                          {aspectRatioOptions.map((option) => {
                            const selected = option.value === selectedSizeOption?.value;
                            return (
                              <button
                                key={option.value}
                                type='button'
                                className={styles.sizeMenuOption}
                                role='option'
                                aria-selected={selected}
                                disabled={option.disabled}
                                onClick={() => {
                                  onAspectRatioChange(option);
                                  setSizeMenuOpen(false);
                                }}
                              >
                                <span className={styles.sizeMenuIdentity}>
                                  <span className={styles.sizeMenuCheck} aria-hidden='true'>
                                    {selected ? (
                                      <Check theme='outline' size={11} fill='currentColor' />
                                    ) : null}
                                  </span>
                                  <span
                                    className={styles.sizeMenuShape}
                                    data-auto={option.value === 'auto' || undefined}
                                    style={
                                      option.width && option.height
                                        ? {
                                            aspectRatio: `${option.width} / ${option.height}`,
                                          }
                                        : undefined
                                    }
                                    aria-hidden='true'
                                  />
                                  <span>{option.label}</span>
                                </span>
                                <small>{imageWorkbenchSizeDimensionsLabel(option)}</small>
                              </button>
                            );
                          })}
                        </div>
                      ) : null}
                    </div>
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
                      max={maxCount}
                      disabled={disabled}
                      onChange={(value) => onCountChange(clampCount(value, maxCount))}
                    />
                  </label>
                </div>
                </div>
              ) : null}
              <button
                type='button'
                className={styles.settingsButton}
                aria-label={t('creativeStudio.canvas.image.settingsLabel', {
                  defaultValue: '图片生成设置',
                })}
                aria-expanded={settingsOpen}
                aria-controls={`canvas-image-settings-${nodeId}`}
                disabled={disabled}
                onClick={() =>
                  setSettingsOpen((open) => {
                    if (open) setSizeMenuOpen(false);
                    return !open;
                  })
                }
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
            </div>
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
