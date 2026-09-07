/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  BookOne,
  BottomBar,
  Clipboard,
  Delete,
  FolderOpen,
  LeftBar,
  Loading,
  MagicWand,
  Pic,
  Time,
  Upload,
} from '@icon-park/react';
import { Button, Input, InputNumber, Radio, Select, Tag, Tooltip } from '@arco-design/web-react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import NomiSelect from '@/renderer/components/base/NomiSelect';
import CreativeMediaPreview from '../../assets/components/CreativeMediaPreview';
import {
  DEFAULT_IMAGE_WORKBENCH_ASPECT_RATIOS,
  IMAGE_WORKBENCH_QUALITY_OPTIONS,
  imageWorkbenchAspectRatioChoices,
  imageWorkbenchAspectRatioValue,
  imageWorkbenchModelKey,
  imageWorkbenchResolutionOptionLabel,
  imageWorkbenchResolutionOptions,
  imageWorkbenchSizeOptionForAspectRatio,
  imageWorkbenchSizeOptionForSettings,
  parseImageWorkbenchModelKey,
  type ImageWorkbenchAspectRatioOption,
  type ImageWorkbenchInterfaceMode,
  type ImageWorkbenchLayout,
  type ImageWorkbenchModelIdentity,
  type ImageWorkbenchModelOption,
  type ImageWorkbenchQuality,
  type ImageWorkbenchReference,
  type ImageWorkbenchSettings,
  type ImageWorkbenchTaskSummary,
} from './types';
import ImageSizePicker from './ImageSizePicker';
import styles from './ImageWorkbench.module.css';

interface ImageWorkbenchComposerProps {
  layout: ImageWorkbenchLayout;
  prompt: string;
  references: readonly ImageWorkbenchReference[];
  settings: ImageWorkbenchSettings;
  modelOptions: readonly ImageWorkbenchModelOption[];
  aspectRatioOptions?: readonly ImageWorkbenchAspectRatioOption[];
  maxCount?: number;
  modelSlot?: React.ReactNode;
  task: ImageWorkbenchTaskSummary;
  disabled?: boolean;
  uploadingReferenceCount?: number;
  onLayoutChange(layout: ImageWorkbenchLayout): void;
  onPromptChange(prompt: string): void;
  onPastePrompt?(): void;
  onClearPrompt?(): void;
  onOpenPromptLibrary?(): void;
  onPasteReferences?(): void;
  onUploadReferences?(): void;
  onChooseReferences?(): void;
  onRemoveReference(referenceId: string): void;
  onModelChange(model: ImageWorkbenchModelIdentity | null): void;
  onInterfaceModeChange(mode: ImageWorkbenchInterfaceMode): void;
  onQualityChange(quality: ImageWorkbenchQuality): void;
  onAspectRatioChange(option: ImageWorkbenchAspectRatioOption): void;
  onCountChange(count: number): void;
  onGenerate(): void;
}

const clampCount = (value: number | undefined, maximum = 10): number =>
  Math.max(1, Math.min(maximum, Math.floor(value || 1)));

const LayoutSwitch: React.FC<{
  layout: ImageWorkbenchLayout;
  onChange(layout: ImageWorkbenchLayout): void;
}> = ({ layout, onChange }) => {
  const { t } = useTranslation();
  return (
    <div
      className={styles.layoutSwitch}
      role='group'
      aria-label={t('creativeStudio.image.layout.label', { defaultValue: '工作台布局' })}
    >
      <Button
        size='small'
        type={layout === 'side' ? 'primary' : 'text'}
        icon={<LeftBar />}
        aria-pressed={layout === 'side'}
        onClick={() => onChange('side')}
      >
        {t('creativeStudio.image.layout.side', { defaultValue: '侧边' })}
      </Button>
      <Button
        size='small'
        type={layout === 'bottom' ? 'primary' : 'text'}
        icon={<BottomBar />}
        aria-pressed={layout === 'bottom'}
        onClick={() => onChange('bottom')}
      >
        {t('creativeStudio.image.layout.bottom', { defaultValue: '底部' })}
      </Button>
    </div>
  );
};

const ComposerActions: React.FC<{
  compact?: boolean;
  onPastePrompt?(): void;
  onClearPrompt?(): void;
  onOpenPromptLibrary?(): void;
  onChooseReferences?(): void;
}> = ({ compact, onPastePrompt, onClearPrompt, onOpenPromptLibrary, onChooseReferences }) => {
  const { t } = useTranslation();
  return (
    <div className={compact ? styles.compactActions : styles.actionRow}>
      {onPastePrompt ? (
        <Tooltip content={t('creativeStudio.image.actions.pastePrompt', { defaultValue: '读取剪贴板' })}>
          <Button size='small' icon={<Clipboard />} onClick={onPastePrompt}>
            {compact
              ? null
              : t('creativeStudio.image.actions.pastePrompt', { defaultValue: '读取剪贴板' })}
          </Button>
        </Tooltip>
      ) : null}
      {onClearPrompt ? (
        <Tooltip content={t('creativeStudio.image.actions.clearInput', { defaultValue: '清空输入' })}>
          <Button size='small' icon={<Delete />} onClick={onClearPrompt}>
            {compact
              ? null
              : t('creativeStudio.image.actions.clear', { defaultValue: '清空' })}
          </Button>
        </Tooltip>
      ) : null}
      {onOpenPromptLibrary ? (
        <Tooltip content={t('creativeStudio.image.actions.promptLibrary', { defaultValue: '提示词库' })}>
          <Button size='small' icon={<BookOne />} onClick={onOpenPromptLibrary}>
            {compact
              ? null
              : t('creativeStudio.image.actions.promptLibrary', { defaultValue: '提示词库' })}
          </Button>
        </Tooltip>
      ) : null}
      {onChooseReferences ? (
        <Tooltip content={t('creativeStudio.image.actions.chooseFromLibrary', { defaultValue: '从素材库选择' })}>
          <Button size='small' icon={<FolderOpen />} onClick={onChooseReferences}>
            {compact
              ? null
              : t('creativeStudio.image.actions.myAssets', { defaultValue: '我的素材' })}
          </Button>
        </Tooltip>
      ) : null}
    </div>
  );
};

const ReferenceStrip: React.FC<{
  references: readonly ImageWorkbenchReference[];
  compact?: boolean;
  uploadingCount: number;
  onRemove(referenceId: string): void;
}> = ({ references, compact, uploadingCount, onRemove }) => {
  const { t } = useTranslation();
  return (
    <div
      className={`${styles.referenceStrip} ${compact ? styles.referenceStripCompact : ''}`}
      data-reference-count={references.length}
    >
      {references.map((reference) => (
        <div key={reference.id} className={styles.referenceItem}>
          <CreativeMediaPreview
            kind='image'
            src={reference.originalUrl ?? reference.previewUrl}
            posterSrc={reference.previewUrl}
            alt={reference.name}
            className={styles.referenceMedia}
          />
          <button
            type='button'
            className={styles.referenceRemove}
            aria-label={t('creativeStudio.image.references.remove', {
              defaultValue: '移除参考图 {{name}}',
              name: reference.name,
            })}
            onClick={() => onRemove(reference.id)}
          >
            <Delete size={13} />
          </button>
          {compact ? null : <span className={styles.referenceName}>{reference.name}</span>}
        </div>
      ))}
      {Array.from({ length: uploadingCount }, (_, index) => (
        <div
          key={`uploading-${index}`}
          className={styles.referenceLoading}
          aria-label={t('creativeStudio.image.references.adding', {
            defaultValue: '正在添加参考图',
          })}
        >
          <Loading className={styles.spin} />
        </div>
      ))}
      {references.length === 0 && uploadingCount === 0 ? (
        <div className={styles.referenceEmpty}>
          {t('creativeStudio.image.references.empty', { defaultValue: '暂无参考图' })}
        </div>
      ) : null}
    </div>
  );
};

interface SettingsFieldsProps {
  compact?: boolean;
  settings: ImageWorkbenchSettings;
  modelOptions: readonly ImageWorkbenchModelOption[];
  aspectRatioOptions: readonly ImageWorkbenchAspectRatioOption[];
  maxCount: number;
  modelSlot?: React.ReactNode;
  disabled?: boolean;
  onModelChange(model: ImageWorkbenchModelIdentity | null): void;
  onInterfaceModeChange(mode: ImageWorkbenchInterfaceMode): void;
  onQualityChange(quality: ImageWorkbenchQuality): void;
  onAspectRatioChange(option: ImageWorkbenchAspectRatioOption): void;
  onCountChange(count: number): void;
}

const SettingsFields: React.FC<SettingsFieldsProps> = ({
  compact,
  settings,
  modelOptions,
  aspectRatioOptions,
  maxCount,
  modelSlot,
  disabled,
  onModelChange,
  onInterfaceModeChange,
  onQualityChange,
  onAspectRatioChange,
  onCountChange,
}) => {
  const { t } = useTranslation();
  const modelValue = settings.model ? imageWorkbenchModelKey(settings.model) : undefined;
  const selectedSizeOption = imageWorkbenchSizeOptionForSettings(aspectRatioOptions, settings);
  const aspectRatioChoices = imageWorkbenchAspectRatioChoices(aspectRatioOptions);
  const selectedAspectRatio = selectedSizeOption
    ? imageWorkbenchAspectRatioValue(selectedSizeOption)
    : '';
  const resolutionOptions = imageWorkbenchResolutionOptions(
    aspectRatioOptions,
    selectedAspectRatio
  );
  const changeAspectRatio = (value: string): void => {
    const option = imageWorkbenchSizeOptionForAspectRatio(
      aspectRatioOptions,
      selectedSizeOption,
      value
    );
    if (option) onAspectRatioChange(option);
  };
  return (
    <div className={compact ? styles.compactSettings : styles.settingsStack}>
      {modelSlot ? (
        <div
          className={`${styles.modelSlot} ${compact ? styles.compactModelField : ''}`}
        >
          {modelSlot}
        </div>
      ) : (
        <label
          className={`${styles.field} ${compact ? styles.compactModelField : ''}`}
        >
          <span>{t('creativeStudio.image.settings.model', { defaultValue: '模型' })}</span>
          <NomiSelect
            value={modelValue}
            placeholder={
              modelOptions.length > 0
                ? t('creativeStudio.image.settings.modelPlaceholder', {
                    defaultValue: '选择生图模型',
                  })
                : t('creativeStudio.image.settings.noModel', {
                    defaultValue: '没有可用生图模型',
                  })
            }
            disabled={disabled || modelOptions.length === 0}
            allowClear
            aria-label={t('creativeStudio.image.settings.modelAria', {
              defaultValue: '生图模型',
            })}
            onChange={(key) =>
              onModelChange(
                typeof key === 'string' ? parseImageWorkbenchModelKey(key, modelOptions) : null
              )
            }
          >
            {modelOptions.map((option) => (
              <NomiSelect.Option
                key={imageWorkbenchModelKey(option)}
                value={imageWorkbenchModelKey(option)}
                disabled={option.disabled}
              >
                <span className={styles.modelOption}>
                  <span>{option.label}</span>
                  {option.providerLabel ? <small>{option.providerLabel}</small> : null}
                </span>
              </NomiSelect.Option>
            ))}
          </NomiSelect>
        </label>
      )}

      <label
        className={`${styles.field} ${compact ? styles.compactInterfaceField : ''}`}
      >
        <span>{t('creativeStudio.image.settings.interfaceMode', { defaultValue: '接口模式' })}</span>
        <Radio.Group
          type='button'
          size='small'
          value={settings.interfaceMode}
          disabled={disabled}
          onChange={(value) => onInterfaceModeChange(value as ImageWorkbenchInterfaceMode)}
        >
          <Radio value='images'>
            {t('creativeStudio.image.interface.images', { defaultValue: 'Images' })}
          </Radio>
          <Radio value='responses'>
            {t('creativeStudio.image.interface.responses', { defaultValue: 'Responses' })}
          </Radio>
        </Radio.Group>
      </label>

      {compact ? (
        <>
          <label className={`${styles.field} ${styles.compactAspectField}`}>
            <span>{t('creativeStudio.image.settings.aspectRatio', { defaultValue: '宽高比' })}</span>
            <Select
              value={selectedAspectRatio}
              aria-label={t('creativeStudio.image.settings.aspectRatio', { defaultValue: '宽高比' })}
              disabled={disabled || !selectedSizeOption}
              onChange={changeAspectRatio}
            >
              {aspectRatioChoices.map((choice) => (
                <Select.Option key={choice.value} value={choice.value} disabled={choice.disabled}>
                  <span className={styles.sizeOptionIdentity}>
                    <span
                      className={styles.aspectShape}
                      style={
                        choice.width && choice.height
                          ? { aspectRatio: `${choice.width} / ${choice.height}` }
                          : undefined
                      }
                      aria-hidden='true'
                    />
                    <span>{choice.label}</span>
                  </span>
                </Select.Option>
              ))}
            </Select>
          </label>
          <label className={`${styles.field} ${styles.compactResolutionField}`}>
            <span>{t('creativeStudio.image.settings.resolution', { defaultValue: '分辨率' })}</span>
            <Select
              value={selectedSizeOption?.value}
              aria-label={t('creativeStudio.image.settings.resolution', { defaultValue: '分辨率' })}
              disabled={disabled || resolutionOptions.length === 0}
              onChange={(value) => {
                const option = resolutionOptions.find(
                  (candidate) => candidate.value === value && !candidate.disabled
                );
                if (option) onAspectRatioChange(option);
              }}
            >
              {resolutionOptions.map((option) => (
                <Select.Option key={option.value} value={option.value} disabled={option.disabled}>
                  {imageWorkbenchResolutionOptionLabel(option)}
                </Select.Option>
              ))}
            </Select>
          </label>
          <label className={`${styles.field} ${styles.compactQualityField}`}>
            <span>{t('creativeStudio.image.settings.quality', { defaultValue: '质量' })}</span>
            <Select
              value={settings.quality}
              disabled={disabled}
              onChange={(value) => onQualityChange(value as ImageWorkbenchQuality)}
            >
              {IMAGE_WORKBENCH_QUALITY_OPTIONS.map((option) => (
                <Select.Option key={option.value} value={option.value}>
                  {option.label}
                </Select.Option>
              ))}
            </Select>
          </label>
          <label className={`${styles.field} ${styles.compactCountField}`}>
            <span>{t('creativeStudio.image.settings.count', { defaultValue: '数量' })}</span>
            <InputNumber
              value={Math.min(settings.count, maxCount)}
              min={1}
              max={maxCount}
              disabled={disabled}
              onChange={(value) => onCountChange(clampCount(value, maxCount))}
            />
          </label>
        </>
      ) : (
        <>
          <div className={styles.settingGroup}>
            <span className={styles.settingLabel}>
              {t('creativeStudio.image.settings.quality', { defaultValue: '质量' })}
            </span>
            <div className={styles.qualityGrid}>
              {IMAGE_WORKBENCH_QUALITY_OPTIONS.map((option) => (
                <button
                  key={option.value}
                  type='button'
                  className={styles.optionPill}
                  data-selected={settings.quality === option.value}
                  disabled={disabled}
                  onClick={() => onQualityChange(option.value)}
                >
                  {option.label}
                </button>
              ))}
            </div>
          </div>

          <ImageSizePicker
            options={aspectRatioOptions}
            value={settings.aspectRatio}
            disabled={disabled}
            onChange={onAspectRatioChange}
          />

          <div className={styles.settingGroup}>
            <span className={styles.settingLabel}>
              {t('creativeStudio.image.settings.outputCount', { defaultValue: '生成张数' })}
            </span>
            <div className={styles.countGrid} data-max-count={maxCount}>
              {[1, 2, 3, 4]
                .filter((count) => count <= maxCount)
                .map((count) => (
                  <button
                    key={count}
                    type='button'
                    className={styles.optionPill}
                    data-selected={settings.count === count}
                    disabled={disabled}
                    onClick={() => onCountChange(count)}
                  >
                    {t('creativeStudio.image.settings.imageCount', {
                      defaultValue: '{{imageCount}} 张',
                      imageCount: count,
                    })}
                  </button>
                ))}
              <InputNumber
                value={Math.min(settings.count, maxCount)}
                min={1}
                max={maxCount}
                disabled={disabled}
                aria-label={t('creativeStudio.image.settings.customCount', {
                  defaultValue: '自定义生成张数',
                })}
                onChange={(value) => onCountChange(clampCount(value, maxCount))}
              />
            </div>
          </div>
        </>
      )}

    </div>
  );
};

const ImageWorkbenchComposer: React.FC<ImageWorkbenchComposerProps> = (props) => {
  const {
    layout,
    prompt,
    references,
    settings,
    modelOptions,
    aspectRatioOptions = DEFAULT_IMAGE_WORKBENCH_ASPECT_RATIOS,
    maxCount = 10,
    modelSlot,
    task,
    disabled,
    uploadingReferenceCount = 0,
    onLayoutChange,
    onPromptChange,
    onPastePrompt,
    onClearPrompt,
    onOpenPromptLibrary,
    onPasteReferences,
    onUploadReferences,
    onChooseReferences,
    onRemoveReference,
    onModelChange,
    onInterfaceModeChange,
    onQualityChange,
    onAspectRatioChange,
    onCountChange,
    onGenerate,
  } = props;
  const { t } = useTranslation();
  const canGenerate = !disabled && prompt.trim().length > 0 && settings.model !== null;
  const pendingLabel =
    task.state === 'queued'
      ? t('creativeStudio.image.task.queued', { defaultValue: '排队中' })
      : t('creativeStudio.image.task.running', { defaultValue: '生成中' });
  const generateLabel =
    task.pendingCount > 0
      ? t('creativeStudio.image.generate.pending', {
          defaultValue: '{{taskCount}} 个{{status}}',
          taskCount: task.pendingCount,
          status: pendingLabel,
        })
      : t('creativeStudio.image.generate.startCreating', { defaultValue: '开始创作' });
  const generateIcon =
    task.state === 'queued' ? (
      <Time />
    ) : task.state === 'running' ? (
      <Loading className={styles.spin} />
    ) : (
      <MagicWand />
    );

  if (layout === 'bottom') {
    return (
      <div className={styles.bottomComposerDock} data-image-workbench-composer='bottom'>
        <div className={styles.bottomComposer}>
          <div className={styles.bottomComposerBody}>
            <div className={styles.bottomPromptPane}>
              <Input.TextArea
                value={prompt}
                autoSize={{ minRows: 4, maxRows: 6 }}
                placeholder={t('creativeStudio.image.prompt.bottomPlaceholder', {
                  defaultValue: '描述你想生成的图片，可通过参考图锁定人物、风格或构图…',
                })}
                disabled={disabled}
                onChange={onPromptChange}
                onPressEnter={(event) => {
                  if (!event.shiftKey && canGenerate) {
                    event.preventDefault();
                    onGenerate();
                  }
                }}
              />
              {references.length > 0 || uploadingReferenceCount > 0 ? (
                <ReferenceStrip
                  compact
                  references={references}
                  uploadingCount={uploadingReferenceCount}
                  onRemove={onRemoveReference}
                />
              ) : null}
              <div className={styles.bottomActionRow}>
                <Button
                  className={styles.bottomGenerateButton}
                  type='primary'
                  icon={generateIcon}
                  disabled={!canGenerate}
                  onClick={onGenerate}
                >
                  {generateLabel}
                </Button>
                <div className={styles.bottomTools}>
                  <ComposerActions
                    compact
                    onPastePrompt={onPastePrompt}
                    onClearPrompt={onClearPrompt}
                    onOpenPromptLibrary={onOpenPromptLibrary}
                    onChooseReferences={onChooseReferences}
                  />
                  {onUploadReferences ? (
                    <Tooltip
                      content={
                        references.length
                          ? t('creativeStudio.image.references.addWithCount', {
                              defaultValue: '添加参考图，当前 {{imageCount}} 张',
                              imageCount: references.length,
                            })
                          : t('creativeStudio.image.references.add', {
                              defaultValue: '添加参考图',
                            })
                      }
                    >
                      <Button icon={<Upload />} onClick={onUploadReferences}>
                        {references.length > 0 ? references.length : null}
                      </Button>
                    </Tooltip>
                  ) : null}
                  <Tooltip
                    content={t('creativeStudio.image.layout.switchToSide', {
                      defaultValue: '切换到侧边工作台',
                    })}
                  >
                    <Button icon={<LeftBar />} onClick={() => onLayoutChange('side')} />
                  </Tooltip>
                </div>
              </div>
            </div>
            <SettingsFields
              compact
              settings={settings}
              modelOptions={modelOptions}
              aspectRatioOptions={aspectRatioOptions}
              maxCount={maxCount}
              modelSlot={modelSlot}
              disabled={disabled}
              onModelChange={onModelChange}
              onInterfaceModeChange={onInterfaceModeChange}
              onQualityChange={onQualityChange}
              onAspectRatioChange={onAspectRatioChange}
              onCountChange={onCountChange}
            />
          </div>
        </div>
      </div>
    );
  }

  return (
    <aside className={styles.sideComposer} data-image-workbench-composer='side'>
      <header className={styles.composerHeader}>
        <div className={styles.composerHeading}>
          <Pic size={20} />
          <span className={styles.composerHeadingText}>
            <h1>{t('creativeStudio.image.header.title', { defaultValue: '生图工作台' })}</h1>
            <small>
              {t('creativeStudio.image.header.settings', { defaultValue: '生成设置' })}
            </small>
          </span>
        </div>
        <LayoutSwitch layout={layout} onChange={onLayoutChange} />
      </header>

      <div className={styles.composerScroll}>
        <section className={styles.composerSection}>
          <div className={styles.sectionHeader}>
            <span>{t('creativeStudio.image.prompt.label', { defaultValue: '提示词' })}</span>
          </div>
          <div className={styles.sectionBody}>
            <ComposerActions
              onPastePrompt={onPastePrompt}
              onClearPrompt={onClearPrompt}
              onOpenPromptLibrary={onOpenPromptLibrary}
              onChooseReferences={onChooseReferences}
            />
            <Input.TextArea
              value={prompt}
              rows={6}
              placeholder={t('creativeStudio.image.prompt.sidePlaceholder', {
                defaultValue: '描述画面主体、风格、构图、光线和用途',
              })}
              disabled={disabled}
              onChange={onPromptChange}
            />
          </div>
        </section>

        <section className={styles.composerSection}>
          <div className={styles.sectionHeader}>
            <span>{t('creativeStudio.image.references.title', { defaultValue: '参考图' })}</span>
            <Tag>{references.length}</Tag>
          </div>
          <div className={styles.sectionBody}>
            <div className={styles.actionRow}>
              {onPasteReferences ? (
                <Button size='small' icon={<Clipboard />} onClick={onPasteReferences}>
                  {t('creativeStudio.image.actions.clipboard', { defaultValue: '剪贴板' })}
                </Button>
              ) : null}
              {onUploadReferences ? (
                <Button size='small' icon={<Upload />} onClick={onUploadReferences}>
                  {t('creativeStudio.image.actions.upload', { defaultValue: '上传' })}
                </Button>
              ) : null}
              {onChooseReferences ? (
                <Button size='small' icon={<FolderOpen />} onClick={onChooseReferences}>
                  {t('creativeStudio.image.actions.chooseFromLibrary', {
                    defaultValue: '从素材库选择',
                  })}
                </Button>
              ) : null}
            </div>
            <ReferenceStrip
              references={references}
              uploadingCount={uploadingReferenceCount}
              onRemove={onRemoveReference}
            />
          </div>
        </section>

        <section className={styles.composerSection}>
          <div className={styles.sectionHeader}>
            <span>
              {t('creativeStudio.image.settings.generationParameters', {
                defaultValue: '生成参数',
              })}
            </span>
          </div>
          <div className={styles.sectionBody}>
            <SettingsFields
              settings={settings}
              modelOptions={modelOptions}
              aspectRatioOptions={aspectRatioOptions}
              maxCount={maxCount}
              modelSlot={modelSlot}
              disabled={disabled}
              onModelChange={onModelChange}
              onInterfaceModeChange={onInterfaceModeChange}
              onQualityChange={onQualityChange}
              onAspectRatioChange={onAspectRatioChange}
              onCountChange={onCountChange}
            />
          </div>
        </section>
      </div>

      <footer className={styles.generateFooter}>
        {task.message ? <span className={styles.taskMessage}>{task.message}</span> : null}
        <Button
          type='primary'
          long
          size='large'
          icon={generateIcon}
          disabled={!canGenerate}
          onClick={onGenerate}
        >
          {task.pendingCount > 0
            ? t('creativeStudio.image.generate.continue', {
                defaultValue: '继续提交（{{taskCount}} 个{{status}}）',
                taskCount: task.pendingCount,
                status: pendingLabel,
              })
            : t('creativeStudio.image.generate.start', { defaultValue: '开始生成' })}
        </Button>
      </footer>
    </aside>
  );
};

export default ImageWorkbenchComposer;
