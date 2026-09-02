/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ArrowUp, BookOne, Down, Loading, SettingTwo } from '@icon-park/react';
import { InputNumber, Radio, Select } from '@arco-design/web-react';
import { autoUpdate, flip, offset, shift, size, useFloating } from '@floating-ui/react';
import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { useTranslation } from 'react-i18next';

import {
  IMAGE_WORKBENCH_QUALITY_OPTIONS,
  imageWorkbenchAspectRatioValue,
  imageWorkbenchModelKey,
  imageWorkbenchResolutionLabel,
  imageWorkbenchSizeOptionForSettings,
  parseImageWorkbenchModelKey,
  type ImageWorkbenchAspectRatioOption,
  type ImageWorkbenchInterfaceMode,
  type ImageWorkbenchModelIdentity,
  type ImageWorkbenchModelOption,
  type ImageWorkbenchQuality,
  type ImageWorkbenchSettings,
  type ImageWorkbenchTaskSummary,
} from '../../workbenches/image';
import ImageSizePicker from '../../workbenches/image/ImageSizePicker';
import CreativeCanvasReferencePromptInput, {
  relabelCreativeCanvasPromptMentions,
  type CreativeCanvasPromptMentionBinding,
  type CreativeCanvasReferencePromptChange,
} from './CreativeCanvasReferencePromptInput';
import CreativeCanvasComposerShell from './CreativeCanvasComposerShell';
import CreativeCanvasReferenceList, {
  type CreativeCanvasImageComposerReference,
} from './CreativeCanvasReferenceList';
import composerStyles from './CreativeCanvasComposerShell.module.css';
import styles from './CreativeCanvasImageComposer.module.css';

export type { CreativeCanvasImageComposerReference } from './CreativeCanvasReferenceList';

export interface CreativeCanvasImageComposerProps {
  nodeId: string;
  hasImageContent: boolean;
  initialPrompt: string;
  initialMentions?: readonly CreativeCanvasPromptMentionBinding[];
  references?: readonly CreativeCanvasImageComposerReference[];
  settings: ImageWorkbenchSettings;
  aspectRatioOptions: readonly ImageWorkbenchAspectRatioOption[];
  maxCount: number;
  modelOptions: readonly ImageWorkbenchModelOption[];
  task: ImageWorkbenchTaskSummary;
  disabled?: boolean;
  generateBlocked?: boolean;
  error?: string | null;
  retrySubmission?: boolean;
  onPromptChange?(change: CreativeCanvasReferencePromptChange): void;
  onReferenceActivate?(sourceNodeId: string): void;
  onReferenceDisconnect?(connectionId: string): void;
  onReferencesDisconnect?(connectionIds: readonly string[]): void;
  onOpenPromptLibrary(): void;
  onModelChange(model: ImageWorkbenchModelIdentity | null): void;
  onInterfaceModeChange(mode: ImageWorkbenchInterfaceMode): void;
  onQualityChange(quality: ImageWorkbenchQuality): void;
  onAspectRatioChange(option: ImageWorkbenchAspectRatioOption): void;
  onCountChange(count: number): void;
  onGenerate(
    prompt: string,
    mentions: readonly CreativeCanvasPromptMentionBinding[]
  ): void;
  onRetrySubmission?(): void;
}

const clampCount = (value: number | undefined, maxCount: number): number =>
  Math.max(1, Math.min(maxCount, Math.floor(value || 1)));

const popupContainer = (trigger: HTMLElement): HTMLElement =>
  (trigger.closest('[data-canvas-image-composer]') as HTMLElement | null) ??
  document.body;

const EMPTY_MENTIONS: readonly CreativeCanvasPromptMentionBinding[] = [];
const EMPTY_REFERENCES: readonly CreativeCanvasImageComposerReference[] = [];

const CreativeCanvasImageComposer: React.FC<CreativeCanvasImageComposerProps> = ({
  nodeId,
  hasImageContent,
  initialPrompt,
  initialMentions = EMPTY_MENTIONS,
  references = EMPTY_REFERENCES,
  settings,
  aspectRatioOptions,
  maxCount,
  modelOptions,
  task,
  disabled = false,
  generateBlocked = false,
  error,
  retrySubmission = false,
  onPromptChange,
  onReferenceActivate,
  onReferenceDisconnect,
  onReferencesDisconnect,
  onOpenPromptLibrary,
  onModelChange,
  onInterfaceModeChange,
  onQualityChange,
  onAspectRatioChange,
  onCountChange,
  onGenerate,
  onRetrySubmission,
}) => {
  const { t, i18n } = useTranslation();
  const settingsHostRef = useRef<HTMLDivElement>(null);
  const referenceMentionLabel = useCallback(
    (ordinal: number) =>
      t('creativeStudio.canvas.image.referenceMentionLabel', {
        index: ordinal,
        defaultValue: `图片${ordinal}` as const,
      }),
    [t]
  );
  const referenceAliasSignature = `${i18n.resolvedLanguage ?? i18n.language}:${references
    .map(
      (reference) =>
        `${reference.nodeId}:${reference.ordinal}:${reference.mentionLabel ?? ''}:${reference.disabledReason ? 'disabled' : 'enabled'}`
    )
    .join(',')}`;
  const normalizedInitialDraft = useMemo(
    () =>
      relabelCreativeCanvasPromptMentions(
        initialPrompt,
        initialMentions,
        references,
        referenceMentionLabel
      ),
    // Reference display names and thumbnails do not affect prompt aliases.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [initialMentions, initialPrompt, referenceAliasSignature, referenceMentionLabel]
  );
  const [prompt, setPrompt] = useState(normalizedInitialDraft.value);
  const [mentions, setMentions] = useState<CreativeCanvasPromptMentionBinding[]>(
    () => structuredClone(normalizedInitialDraft.mentions)
  );
  const [settingsOpen, setSettingsOpen] = useState(false);
  const { refs: settingsRefs, floatingStyles: settingsPosition, placement: settingsPlacement } = useFloating({
    open: settingsOpen,
    placement: 'top-end',
    middleware: [
      offset(10),
      flip({ padding: 12 }),
      shift({ padding: 12 }),
      size({
        padding: 12,
        apply({ availableHeight, elements }) {
          elements.floating.style.setProperty(
            '--creative-image-settings-available-height', `${Math.max(0, availableHeight)}px`
          );
        },
      }),
    ],
    whileElementsMounted: autoUpdate,
  });
  const busy = task.state === 'queued' || task.state === 'running';
  const hasTextInput = references.some((reference) =>
    reference.kind === 'text' && reference.textContent?.trim() && !reference.disabledReason
  );
  const canGenerate = retrySubmission
    ? !disabled && onRetrySubmission !== undefined
    : !disabled && !generateBlocked && !busy && (prompt.trim().length > 0 || hasTextInput) && settings.model !== null;
  const modelValue = settings.model ? imageWorkbenchModelKey(settings.model) : undefined;
  const modelOptionByKey = useMemo(
    () => new Map(modelOptions.map((option) => [imageWorkbenchModelKey(option), option])),
    [modelOptions]
  );
  const selectedSizeOption = imageWorkbenchSizeOptionForSettings(aspectRatioOptions, settings);
  const selectedAspectRatio = selectedSizeOption
    ? selectedSizeOption.value === 'auto'
      ? selectedSizeOption.label
      : imageWorkbenchAspectRatioValue(selectedSizeOption)
    : '';
  const selectedResolution = selectedSizeOption
    ? imageWorkbenchResolutionLabel(selectedSizeOption)
    : '';
  const sizeSummary = [...new Set([selectedAspectRatio, selectedResolution])]
    .filter(Boolean).join(' · ');

  useEffect(() => {
    setPrompt(normalizedInitialDraft.value);
    setMentions(structuredClone(normalizedInitialDraft.mentions));
    if (normalizedInitialDraft.value !== initialPrompt) {
      onPromptChange?.({
        value: normalizedInitialDraft.value,
        mentions: structuredClone(normalizedInitialDraft.mentions),
      });
    }
    // onPromptChange is an inline route callback; the normalized value is the
    // idempotency boundary that prevents a migration loop.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [initialPrompt, nodeId, normalizedInitialDraft]);

  useEffect(() => {
    setSettingsOpen(false);
  }, [nodeId]);

  useEffect(() => {
    if (disabled) setSettingsOpen(false);
  }, [disabled]);

  useEffect(() => {
    if (!settingsOpen) return;
    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (target instanceof Node) {
        if (!settingsHostRef.current?.contains(target)) {
          setSettingsOpen(false);
        }
      }
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return;
      setSettingsOpen(false);
    };
    document.addEventListener('pointerdown', handlePointerDown, true);
    document.addEventListener('keydown', handleKeyDown);
    return () => {
      document.removeEventListener('pointerdown', handlePointerDown, true);
      document.removeEventListener('keydown', handleKeyDown);
    };
  }, [settingsOpen]);

  const submit = (
    change: CreativeCanvasReferencePromptChange = { value: prompt, mentions }
  ): void => {
    if (retrySubmission && onRetrySubmission) {
      onRetrySubmission();
      return;
    }
    if (!canGenerate || (!change.value.trim() && !hasTextInput)) return;
    // Mention offsets are relative to the authored string. Preserve it exactly;
    // trimming here would make otherwise valid UTF-16 ranges stale.
    onGenerate(change.value, change.mentions);
    setPrompt('');
    setMentions([]);
  };

  return (
    <CreativeCanvasComposerShell
      kind='image'
      nodeId={nodeId}
    >
        <CreativeCanvasReferenceList
          key={nodeId}
          references={references}
          disabled={disabled}
          onActivate={onReferenceActivate}
          onDisconnect={onReferenceDisconnect}
          onDisconnectMany={onReferencesDisconnect}
        />

        <CreativeCanvasReferencePromptInput
          value={prompt}
          mentions={mentions}
          references={references}
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
          disabled={disabled}
          labels={{
            input: t('creativeStudio.canvas.image.promptLabel', {
              defaultValue: '图片创作提示词',
            }),
            insertReference: t('creativeStudio.canvas.image.insertReference', {
              defaultValue: '引用已连接素材',
            }),
            connectedReferences: t('creativeStudio.canvas.image.connectedReferences', {
              defaultValue: '已连接参考',
            }),
            emptyReferences: t('creativeStudio.canvas.image.noMatchingReferences', {
              defaultValue: '没有匹配的已连接素材',
            }),
            disconnectedReference: t('creativeStudio.canvas.image.referenceDisconnected', {
              defaultValue: '引用已断开',
            }),
            referenceMentionLabel,
          }}
          onChange={(change) => {
            setPrompt(change.value);
            setMentions(change.mentions);
            onPromptChange?.(change);
          }}
          onSubmit={submit}
        />

        <div className={composerStyles.footer}>
          <div className={composerStyles.controls}>
            <button
              type='button'
              className={`${composerStyles.controlButton} ${composerStyles.iconButton}`}
              aria-label={t('creativeStudio.canvas.openPromptLibrary', {
                defaultValue: '打开提示词库',
              })}
              disabled={disabled}
              onClick={onOpenPromptLibrary}
            >
              <BookOne theme='outline' size={17} fill='currentColor' />
            </button>

            <Select
              className={composerStyles.modelSelect}
              size='mini'
              showSearch
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
              dropdownMenuClassName={composerStyles.modelMenu}
              triggerProps={{
                autoAlignPopupWidth: false,
                popupStyle: {
                  width: 'min(440px, calc(100vw - 24px))',
                  maxWidth: 'calc(100vw - 24px)',
                },
              }}
              filterOption={(input, candidate) => {
                const key = (candidate as React.ReactElement<{ value?: unknown }>).props.value;
                const option = typeof key === 'string' ? modelOptionByKey.get(key) : undefined;
                if (!option) return false;
                const query = input.trim().toLocaleLowerCase();
                return [option.label, option.model, option.providerLabel]
                  .filter((part): part is string => Boolean(part))
                  .some((part) => part.toLocaleLowerCase().includes(query));
              }}
              renderFormat={(_candidate, key) => {
                const option = typeof key === 'string' ? modelOptionByKey.get(key) : undefined;
                if (!option) return typeof key === 'string' ? key : '';
                const title = [option.label, option.rawModelId, option.providerLabel]
                  .filter(Boolean)
                  .join(' · ');
                return (
                  <span className={composerStyles.selectedModelLabel} title={title}>
                    {option.label}
                  </span>
                );
              }}
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
                  <span className={composerStyles.modelOption}>
                    <span className={composerStyles.modelOptionIdentity}>
                      <span className={composerStyles.modelOptionLabel} title={option.label}>
                        {option.label}
                      </span>
                      {option.rawModelId ? (
                        <span className={composerStyles.modelOptionId} title={option.rawModelId}>
                          <span aria-hidden='true'>·</span> {option.rawModelId}
                        </span>
                      ) : null}
                    </span>
                    {option.providerLabel ? (
                      <span className={composerStyles.modelOptionProvider}>
                        {option.providerLabel}
                      </span>
                    ) : null}
                  </span>
                </Select.Option>
              ))}
            </Select>

            <div ref={settingsHostRef} className={styles.settingsHost}>
              {settingsOpen ? (
                <div
                  id={`canvas-image-settings-${nodeId}`}
                  className={styles.settingsPopover}
                  ref={settingsRefs.setFloating}
                  style={settingsPosition}
                  data-placement={settingsPlacement}
                  role='dialog'
                  aria-label={t('creativeStudio.canvas.image.settingsLabel', {
                    defaultValue: '图片生成设置',
                  })}
                >
                <div className={styles.settingsPanel}>
                  <div className={composerStyles.field}>
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
                  <label className={composerStyles.field}>
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
                  <ImageSizePicker
                    options={aspectRatioOptions}
                    value={settings.aspectRatio}
                    disabled={disabled}
                    onChange={onAspectRatioChange}
                  />
                  <label className={composerStyles.field}>
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
                ref={settingsRefs.setReference}
                className={`${composerStyles.controlButton} ${composerStyles.settingsButton}`}
                aria-label={t('creativeStudio.canvas.image.settingsLabel', {
                  defaultValue: '图片生成设置',
                })}
                aria-expanded={settingsOpen}
                aria-controls={`canvas-image-settings-${nodeId}`}
                disabled={disabled}
                onClick={() => setSettingsOpen((open) => !open)}
              >
                <SettingTwo theme='outline' size={15} fill='currentColor' />
                <span className={composerStyles.settingsSummary}>
                  {t('creativeStudio.canvas.image.settingsSummary', {
                    size: sizeSummary,
                    count: settings.count,
                    defaultValue: `${sizeSummary} · ${settings.count} 张`,
                  })}
                </span>
              </button>
            </div>
          </div>

          <button
            type='button'
            className={`${composerStyles.controlButton} ${composerStyles.submitButton}`}
            aria-label={t('creativeStudio.canvas.image.generateLabel', {
              defaultValue: '生成图片',
            })}
            disabled={!canGenerate}
            onClick={() => submit()}
          >
            {busy ? (
              <Loading
                className={composerStyles.spin}
                theme='outline'
                size={17}
                fill='currentColor'
              />
            ) : (
              <ArrowUp theme='outline' size={17} fill='currentColor' strokeWidth={4} />
            )}
          </button>
        </div>

        {error || task.message ? (
          <div
            className={composerStyles.message}
            role={error ? 'alert' : 'status'}
          >
            {error ?? task.message}
          </div>
        ) : null}
    </CreativeCanvasComposerShell>
  );
};

export default CreativeCanvasImageComposer;
