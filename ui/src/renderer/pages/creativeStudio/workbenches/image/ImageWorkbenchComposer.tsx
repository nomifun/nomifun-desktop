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
  SettingTwo,
  Time,
  Upload,
} from '@icon-park/react';
import { Button, Input, InputNumber, Radio, Select, Tag, Tooltip } from '@arco-design/web-react';
import React from 'react';
import {
  DEFAULT_IMAGE_WORKBENCH_ASPECT_RATIOS,
  IMAGE_WORKBENCH_QUALITY_OPTIONS,
  imageWorkbenchModelKey,
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
import styles from './ImageWorkbench.module.css';

interface ImageWorkbenchComposerProps {
  layout: ImageWorkbenchLayout;
  prompt: string;
  references: readonly ImageWorkbenchReference[];
  settings: ImageWorkbenchSettings;
  modelOptions: readonly ImageWorkbenchModelOption[];
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
  onDimensionsChange(dimensions: { width: number | null; height: number | null }): void;
  onAspectRatioChange(option: ImageWorkbenchAspectRatioOption): void;
  onCountChange(count: number): void;
  onGenerate(): void;
}

const clampDimension = (value: number | undefined): number =>
  Math.max(1, Math.min(8192, Math.floor(value || 1)));

const clampCount = (value: number | undefined): number =>
  Math.max(1, Math.min(10, Math.floor(value || 1)));

const LayoutSwitch: React.FC<{
  layout: ImageWorkbenchLayout;
  onChange(layout: ImageWorkbenchLayout): void;
}> = ({ layout, onChange }) => (
  <div className={styles.layoutSwitch} role='group' aria-label='工作台布局'>
    <Button
      size='small'
      type={layout === 'side' ? 'primary' : 'text'}
      icon={<LeftBar />}
      aria-pressed={layout === 'side'}
      onClick={() => onChange('side')}
    >
      侧边
    </Button>
    <Button
      size='small'
      type={layout === 'bottom' ? 'primary' : 'text'}
      icon={<BottomBar />}
      aria-pressed={layout === 'bottom'}
      onClick={() => onChange('bottom')}
    >
      底部
    </Button>
  </div>
);

const ComposerActions: React.FC<{
  compact?: boolean;
  onPastePrompt?(): void;
  onClearPrompt?(): void;
  onOpenPromptLibrary?(): void;
  onChooseReferences?(): void;
}> = ({ compact, onPastePrompt, onClearPrompt, onOpenPromptLibrary, onChooseReferences }) => (
  <div className={compact ? styles.compactActions : styles.actionRow}>
    {onPastePrompt ? (
      <Tooltip content='读取剪贴板'>
        <Button size='small' icon={<Clipboard />} onClick={onPastePrompt}>
          {compact ? null : '读取剪贴板'}
        </Button>
      </Tooltip>
    ) : null}
    {onClearPrompt ? (
      <Tooltip content='清空输入'>
        <Button size='small' icon={<Delete />} onClick={onClearPrompt}>
          {compact ? null : '清空'}
        </Button>
      </Tooltip>
    ) : null}
    {onOpenPromptLibrary ? (
      <Tooltip content='提示词库'>
        <Button size='small' icon={<BookOne />} onClick={onOpenPromptLibrary}>
          {compact ? null : '提示词库'}
        </Button>
      </Tooltip>
    ) : null}
    {onChooseReferences ? (
      <Tooltip content='从素材库选择'>
        <Button size='small' icon={<FolderOpen />} onClick={onChooseReferences}>
          {compact ? null : '我的素材'}
        </Button>
      </Tooltip>
    ) : null}
  </div>
);

const ReferenceStrip: React.FC<{
  references: readonly ImageWorkbenchReference[];
  compact?: boolean;
  uploadingCount: number;
  onRemove(referenceId: string): void;
}> = ({ references, compact, uploadingCount, onRemove }) => (
  <div
    className={`${styles.referenceStrip} ${compact ? styles.referenceStripCompact : ''}`}
    data-reference-count={references.length}
  >
    {references.map((reference) => (
      <div key={reference.id} className={styles.referenceItem}>
        <img src={reference.previewUrl} alt={reference.name} />
        <button
          type='button'
          className={styles.referenceRemove}
          aria-label={`移除参考图 ${reference.name}`}
          onClick={() => onRemove(reference.id)}
        >
          <Delete size={13} />
        </button>
        {compact ? null : <span className={styles.referenceName}>{reference.name}</span>}
      </div>
    ))}
    {Array.from({ length: uploadingCount }, (_, index) => (
      <div key={`uploading-${index}`} className={styles.referenceLoading} aria-label='正在添加参考图'>
        <Loading className={styles.spin} />
      </div>
    ))}
    {references.length === 0 && uploadingCount === 0 ? (
      <div className={styles.referenceEmpty}>暂无参考图</div>
    ) : null}
  </div>
);

interface SettingsFieldsProps {
  compact?: boolean;
  settings: ImageWorkbenchSettings;
  modelOptions: readonly ImageWorkbenchModelOption[];
  disabled?: boolean;
  onModelChange(model: ImageWorkbenchModelIdentity | null): void;
  onInterfaceModeChange(mode: ImageWorkbenchInterfaceMode): void;
  onQualityChange(quality: ImageWorkbenchQuality): void;
  onDimensionsChange(dimensions: { width: number | null; height: number | null }): void;
  onAspectRatioChange(option: ImageWorkbenchAspectRatioOption): void;
  onCountChange(count: number): void;
}

const SettingsFields: React.FC<SettingsFieldsProps> = ({
  compact,
  settings,
  modelOptions,
  disabled,
  onModelChange,
  onInterfaceModeChange,
  onQualityChange,
  onDimensionsChange,
  onAspectRatioChange,
  onCountChange,
}) => {
  const modelValue = settings.model ? imageWorkbenchModelKey(settings.model) : undefined;
  return (
    <div className={compact ? styles.compactSettings : styles.settingsStack}>
      <label className={styles.field}>
        <span>模型</span>
        <Select
          value={modelValue}
          placeholder='选择生图模型'
          disabled={disabled}
          allowClear
          onChange={(key) =>
            onModelChange(
              typeof key === 'string' ? parseImageWorkbenchModelKey(key, modelOptions) : null
            )
          }
        >
          {modelOptions.map((option) => (
            <Select.Option
              key={imageWorkbenchModelKey(option)}
              value={imageWorkbenchModelKey(option)}
              disabled={option.disabled}
            >
              <span className={styles.modelOption}>
                <span>{option.label}</span>
                {option.providerLabel ? <small>{option.providerLabel}</small> : null}
              </span>
            </Select.Option>
          ))}
        </Select>
      </label>

      <label className={styles.field}>
        <span>接口模式</span>
        <Radio.Group
          type='button'
          size='small'
          value={settings.interfaceMode}
          disabled={disabled}
          onChange={(value) => onInterfaceModeChange(value as ImageWorkbenchInterfaceMode)}
        >
          <Radio value='images'>Images</Radio>
          <Radio value='responses'>Responses</Radio>
        </Radio.Group>
      </label>

      {compact ? (
        <>
          <label className={styles.field}>
            <span>宽度</span>
            <InputNumber
              value={settings.width ?? undefined}
              min={1}
              max={8192}
              placeholder='自动'
              disabled={disabled || settings.width === null}
              onChange={(value) =>
                onDimensionsChange({ width: clampDimension(value), height: settings.height })
              }
            />
          </label>
          <label className={styles.field}>
            <span>高度</span>
            <InputNumber
              value={settings.height ?? undefined}
              min={1}
              max={8192}
              placeholder='自动'
              disabled={disabled || settings.height === null}
              onChange={(value) =>
                onDimensionsChange({ width: settings.width, height: clampDimension(value) })
              }
            />
          </label>
          <label className={styles.field}>
            <span>宽高比</span>
            <Select
              value={settings.aspectRatio}
              disabled={disabled}
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
          <label className={styles.field}>
            <span>质量</span>
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
          <label className={styles.field}>
            <span>数量</span>
            <InputNumber
              value={settings.count}
              min={1}
              max={10}
              disabled={disabled}
              onChange={(value) => onCountChange(clampCount(value))}
            />
          </label>
        </>
      ) : (
        <>
          <div className={styles.settingGroup}>
            <span className={styles.settingLabel}>质量</span>
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

          <div className={styles.settingGroup}>
            <span className={styles.settingLabel}>尺寸</span>
            <div className={styles.dimensionGrid}>
              <label>
                <span>W</span>
                <InputNumber
                  value={settings.width ?? undefined}
                  min={1}
                  max={8192}
                  placeholder='自动'
                  disabled={disabled || settings.width === null}
                  onChange={(value) =>
                    onDimensionsChange({ width: clampDimension(value), height: settings.height })
                  }
                />
              </label>
              <span className={styles.dimensionLink}>×</span>
              <label>
                <span>H</span>
                <InputNumber
                  value={settings.height ?? undefined}
                  min={1}
                  max={8192}
                  placeholder='自动'
                  disabled={disabled || settings.height === null}
                  onChange={(value) =>
                    onDimensionsChange({ width: settings.width, height: clampDimension(value) })
                  }
                />
              </label>
            </div>
          </div>

          <div className={styles.settingGroup}>
            <span className={styles.settingLabel}>宽高比</span>
            <div className={styles.aspectGrid}>
              {DEFAULT_IMAGE_WORKBENCH_ASPECT_RATIOS.map((option) => (
                <button
                  key={option.value}
                  type='button'
                  className={styles.aspectOption}
                  data-selected={settings.aspectRatio === option.value}
                  disabled={disabled || option.disabled}
                  onClick={() => onAspectRatioChange(option)}
                >
                  <span
                    className={styles.aspectShape}
                    style={
                      option.width && option.height
                        ? { aspectRatio: `${option.width} / ${option.height}` }
                        : undefined
                    }
                    aria-hidden='true'
                  />
                  <span>{option.label}</span>
                </button>
              ))}
            </div>
          </div>

          <div className={styles.settingGroup}>
            <span className={styles.settingLabel}>生成张数</span>
            <div className={styles.countGrid}>
              {[1, 2, 3, 4].map((count) => (
                <button
                  key={count}
                  type='button'
                  className={styles.optionPill}
                  data-selected={settings.count === count}
                  disabled={disabled}
                  onClick={() => onCountChange(count)}
                >
                  {count} 张
                </button>
              ))}
              <InputNumber
                value={settings.count}
                min={1}
                max={10}
                disabled={disabled}
                aria-label='自定义生成张数'
                onChange={(value) => onCountChange(clampCount(value))}
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
    onDimensionsChange,
    onAspectRatioChange,
    onCountChange,
    onGenerate,
  } = props;
  const canGenerate = !disabled && prompt.trim().length > 0 && settings.model !== null;
  const pendingLabel = task.state === 'queued' ? '排队中' : '生成中';
  const generateLabel = task.pendingCount > 0 ? `${task.pendingCount} 个${pendingLabel}` : '开始创作';
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
          <div className={styles.bottomPromptRow}>
            <Input.TextArea
              value={prompt}
              autoSize={{ minRows: 2, maxRows: 4 }}
              placeholder='描述你想生成的图片，可通过参考图锁定人物、风格或构图…'
              disabled={disabled}
              onChange={onPromptChange}
              onPressEnter={(event) => {
                if (!event.shiftKey && canGenerate) {
                  event.preventDefault();
                  onGenerate();
                }
              }}
            />
            <ComposerActions
              compact
              onPastePrompt={onPastePrompt}
              onClearPrompt={onClearPrompt}
              onOpenPromptLibrary={onOpenPromptLibrary}
              onChooseReferences={onChooseReferences}
            />
            {onUploadReferences ? (
              <Tooltip content={references.length ? `添加参考图，当前 ${references.length} 张` : '添加参考图'}>
                <Button icon={<Upload />} onClick={onUploadReferences}>
                  {references.length > 0 ? references.length : null}
                </Button>
              </Tooltip>
            ) : null}
            <Tooltip content='切换到侧边工作台'>
              <Button icon={<LeftBar />} onClick={() => onLayoutChange('side')} />
            </Tooltip>
            <Button
              type='primary'
              icon={generateIcon}
              disabled={!canGenerate}
              onClick={onGenerate}
            >
              {generateLabel}
            </Button>
          </div>
          <SettingsFields
            compact
            settings={settings}
            modelOptions={modelOptions}
            disabled={disabled}
            onModelChange={onModelChange}
            onInterfaceModeChange={onInterfaceModeChange}
            onQualityChange={onQualityChange}
            onDimensionsChange={onDimensionsChange}
            onAspectRatioChange={onAspectRatioChange}
            onCountChange={onCountChange}
          />
          {references.length > 0 || uploadingReferenceCount > 0 ? (
            <ReferenceStrip
              compact
              references={references}
              uploadingCount={uploadingReferenceCount}
              onRemove={onRemoveReference}
            />
          ) : null}
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
            <h1>生图工作台</h1>
            <small>生成设置</small>
          </span>
        </div>
        <LayoutSwitch layout={layout} onChange={onLayoutChange} />
      </header>

      <div className={styles.composerScroll}>
        <section className={styles.composerSection}>
          <div className={styles.sectionHeader}>
            <span>提示词</span>
            <SettingTwo aria-hidden='true' />
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
              placeholder='描述画面主体、风格、构图、光线和用途'
              disabled={disabled}
              onChange={onPromptChange}
            />
          </div>
        </section>

        <section className={styles.composerSection}>
          <div className={styles.sectionHeader}>
            <span>参考图</span>
            <Tag>{references.length}</Tag>
          </div>
          <div className={styles.sectionBody}>
            <div className={styles.actionRow}>
              {onPasteReferences ? (
                <Button size='small' icon={<Clipboard />} onClick={onPasteReferences}>
                  剪贴板
                </Button>
              ) : null}
              {onUploadReferences ? (
                <Button size='small' icon={<Upload />} onClick={onUploadReferences}>
                  上传
                </Button>
              ) : null}
              {onChooseReferences ? (
                <Button size='small' icon={<FolderOpen />} onClick={onChooseReferences}>
                  从素材库选择
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
            <span>生成参数</span>
          </div>
          <div className={styles.sectionBody}>
            <SettingsFields
              settings={settings}
              modelOptions={modelOptions}
              disabled={disabled}
              onModelChange={onModelChange}
              onInterfaceModeChange={onInterfaceModeChange}
              onQualityChange={onQualityChange}
              onDimensionsChange={onDimensionsChange}
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
          {task.pendingCount > 0 ? `继续提交（${task.pendingCount} 个${pendingLabel}）` : '开始生成'}
        </Button>
      </footer>
    </aside>
  );
};

export default ImageWorkbenchComposer;
