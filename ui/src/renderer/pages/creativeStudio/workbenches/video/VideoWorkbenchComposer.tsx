/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  CloseSmall,
  FolderPlus,
  LayoutOne,
  LayoutTwo,
  Left,
  MagicWand,
  Play,
  Plus,
  Right,
  SettingTwo,
  VideoTwo,
} from '@icon-park/react';
import { Button, Input } from '@arco-design/web-react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import CreativeMediaPreview from '../../assets/components/CreativeMediaPreview';

import { normalizeVideoTaskCount } from './presentation';
import styles from './VideoWorkbench.module.css';
import type {
  VideoWorkbenchChoice,
  VideoWorkbenchLayout,
  VideoWorkbenchProps,
  VideoWorkbenchReference,
} from './types';

type ComposerProps = Pick<
  VideoWorkbenchProps,
  | 'layout'
  | 'onLayoutChange'
  | 'prompt'
  | 'onPromptChange'
  | 'onGenerate'
  | 'generating'
  | 'submitDisabled'
  | 'references'
  | 'addReferenceLabel'
  | 'onAddReferences'
  | 'onRemoveReference'
  | 'onMoveReference'
  | 'modelSlot'
  | 'resolution'
  | 'resolutionOptions'
  | 'onResolutionChange'
  | 'size'
  | 'sizeOptions'
  | 'onSizeChange'
  | 'duration'
  | 'durationOptions'
  | 'onDurationChange'
  | 'taskCount'
  | 'onTaskCountChange'
  | 'onOpenParameters'
  | 'onOpenPromptLibrary'
  | 'tasks'
>;

const ReferenceItem: React.FC<{
  item: VideoWorkbenchReference;
  index: number;
  total: number;
  onRemove: () => void;
  onMove?: (direction: -1 | 1) => void;
}> = ({ item, index, total, onRemove, onMove }) => {
  const { t } = useTranslation();
  return (
  <article className={styles.referenceItem} data-reference-kind={item.kind}>
    <div className={styles.referencePreview}>
      <CreativeMediaPreview
        kind={item.kind}
        src={item.originalUrl ?? (item.kind === 'image' ? item.previewUrl : undefined)}
        posterSrc={item.previewUrl}
        alt={item.name}
        className={styles.referenceMedia}
      />
    </div>
    <span className={styles.referenceName} title={item.name}>
      {item.name}
    </span>
    {onMove ? (
      <span className={styles.referenceOrder}>
        <button
          type='button'
          aria-label={t('creativeStudio.video.references.movePrevious', {
            defaultValue: '前移参考素材 {{name}}',
            name: item.name,
          })}
          disabled={index === 0}
          onClick={() => onMove(-1)}
        >
          <Left size={11} />
        </button>
        <button
          type='button'
          aria-label={t('creativeStudio.video.references.moveNext', {
            defaultValue: '后移参考素材 {{name}}',
            name: item.name,
          })}
          disabled={index === total - 1}
          onClick={() => onMove(1)}
        >
          <Right size={11} />
        </button>
      </span>
    ) : null}
    <button
      type='button'
      className={styles.referenceRemove}
      aria-label={t('creativeStudio.video.references.remove', {
        defaultValue: '移除参考素材 {{name}}',
        name: item.name,
      })}
      onClick={onRemove}
    >
      <CloseSmall size={13} />
    </button>
  </article>
  );
};

const ReferenceStrip: React.FC<
  Pick<
    ComposerProps,
    | 'references'
    | 'addReferenceLabel'
    | 'onAddReferences'
    | 'onRemoveReference'
    | 'onMoveReference'
  >
> = ({ references, addReferenceLabel, onAddReferences, onRemoveReference, onMoveReference }) => {
  const { t } = useTranslation();
  return (
  <div className={styles.referenceSection}>
    <div className={styles.sectionHeading}>
      <span>{t('creativeStudio.video.references.title', { defaultValue: '参考素材' })}</span>
      <span>{references.length}</span>
    </div>
    <div className={styles.referenceStrip}>
      {references.map((item, index) => (
        <ReferenceItem
          key={item.id}
          item={item}
          index={index}
          total={references.length}
          onRemove={() => onRemoveReference(item.id)}
          onMove={
            onMoveReference
              ? (direction) => onMoveReference(item.id, direction)
              : undefined
          }
        />
      ))}
      <button type='button' className={styles.addReference} onClick={onAddReferences}>
        <Plus size={15} />
        <span>
          {references.length
            ? t('creativeStudio.video.references.addMore', { defaultValue: '继续添加' })
            : addReferenceLabel ??
              t('creativeStudio.video.references.addDefault', {
                defaultValue: '添加图片、视频或音频',
              })}
        </span>
      </button>
    </div>
  </div>
  );
};

const LayoutSwitch: React.FC<{
  layout: VideoWorkbenchLayout;
  onChange: (layout: VideoWorkbenchLayout) => void;
}> = ({ layout, onChange }) => {
  const { t } = useTranslation();
  return (
  <div
    className={styles.layoutSwitch}
    aria-label={t('creativeStudio.video.layout.label', { defaultValue: '工作台布局' })}
  >
    <button
      type='button'
      data-active={layout === 'side' || undefined}
      aria-pressed={layout === 'side'}
      onClick={() => onChange('side')}
    >
      <LayoutOne size={14} />
      <span>{t('creativeStudio.video.layout.side', { defaultValue: '侧边' })}</span>
    </button>
    <button
      type='button'
      data-active={layout === 'bottom' || undefined}
      aria-pressed={layout === 'bottom'}
      onClick={() => onChange('bottom')}
    >
      <LayoutTwo size={14} />
      <span>{t('creativeStudio.video.layout.bottom', { defaultValue: '底部' })}</span>
    </button>
  </div>
  );
};

const QuickSelect: React.FC<{
  label: string;
  value: string;
  options: readonly VideoWorkbenchChoice[];
  onChange: (value: string) => void;
}> = ({ label, value, options, onChange }) => (
  <label className={styles.quickControl}>
    <span>{label}</span>
    <select value={value} onChange={(event) => onChange(event.target.value)}>
      {options.map((option) => (
        <option key={option.value} value={option.value}>
          {option.label}
        </option>
      ))}
    </select>
  </label>
);

type SettingsGridProps = Pick<
  ComposerProps,
  | 'modelSlot'
  | 'resolution'
  | 'resolutionOptions'
  | 'onResolutionChange'
  | 'size'
  | 'sizeOptions'
  | 'onSizeChange'
  | 'duration'
  | 'durationOptions'
  | 'onDurationChange'
  | 'taskCount'
  | 'onTaskCountChange'
> & {
  compact?: boolean;
};

const SettingsGrid: React.FC<SettingsGridProps> = ({
  compact = false,
  modelSlot,
  resolution,
  resolutionOptions,
  onResolutionChange,
  size,
  sizeOptions,
  onSizeChange,
  duration,
  durationOptions,
  onDurationChange,
  taskCount,
  onTaskCountChange,
}) => {
  const { t } = useTranslation();
  return (
    <div
      className={`${styles.settingsGrid} ${compact ? styles.compactSettingsGrid : ''}`}
    >
      <div className={styles.modelControl}>
        <span>{t('creativeStudio.video.settings.model', { defaultValue: '模型' })}</span>
        <div>{modelSlot}</div>
      </div>
      <QuickSelect
        label={t('creativeStudio.video.settings.resolution', { defaultValue: '分辨率' })}
        value={resolution}
        options={resolutionOptions}
        onChange={onResolutionChange}
      />
      <QuickSelect
        label={t('creativeStudio.video.settings.aspectRatio', { defaultValue: '宽高比' })}
        value={size}
        options={sizeOptions}
        onChange={onSizeChange}
      />
      <QuickSelect
        label={t('creativeStudio.video.settings.duration', { defaultValue: '时长' })}
        value={duration}
        options={durationOptions}
        onChange={onDurationChange}
      />
      <label className={styles.quickControl}>
        <span>{t('creativeStudio.video.settings.taskCount', { defaultValue: '任务数量' })}</span>
        <input
          type='number'
          min={1}
          max={6}
          value={taskCount}
          onChange={(event) =>
            onTaskCountChange(normalizeVideoTaskCount(Number(event.target.value)))
          }
        />
      </label>
    </div>
  );
};

const VideoWorkbenchComposer: React.FC<ComposerProps> = ({
  layout,
  onLayoutChange,
  prompt,
  onPromptChange,
  onGenerate,
  generating = false,
  submitDisabled = false,
  references,
  addReferenceLabel,
  onAddReferences,
  onRemoveReference,
  onMoveReference,
  modelSlot,
  resolution,
  resolutionOptions,
  onResolutionChange,
  size,
  sizeOptions,
  onSizeChange,
  duration,
  durationOptions,
  onDurationChange,
  taskCount,
  onTaskCountChange,
  onOpenParameters,
  onOpenPromptLibrary,
  tasks,
}) => {
  const { t } = useTranslation();
  const pendingCount = tasks.filter(
    (task) => task.status === 'queued' || task.status === 'running'
  ).length;
  const disabled = submitDisabled || prompt.trim().length === 0;
  const settings = {
    modelSlot,
    resolution,
    resolutionOptions,
    onResolutionChange,
    size,
    sizeOptions,
    onSizeChange,
    duration,
    durationOptions,
    onDurationChange,
    taskCount,
    onTaskCountChange,
  };
  const referenceProps = {
    references,
    addReferenceLabel,
    onAddReferences,
    onRemoveReference,
    onMoveReference,
  };

  if (layout === 'bottom') {
    return (
      <aside className={styles.bottomComposer} data-video-composer='bottom'>
        <div className={styles.bottomComposerSurface}>
          <div className={styles.bottomComposerBody}>
            <div className={styles.bottomPromptPane}>
              <Input.TextArea
                value={prompt}
                onChange={onPromptChange}
                autoSize={{ minRows: 4, maxRows: 6 }}
                placeholder={t('creativeStudio.video.prompt.placeholder', {
                  defaultValue: '描述镜头运动、主体动作、场景氛围和画面风格',
                })}
                aria-label={t('creativeStudio.video.prompt.label', {
                  defaultValue: '视频提示词',
                })}
                onPressEnter={(event) => {
                  if (!event.shiftKey && !disabled && !generating) {
                    event.preventDefault();
                    onGenerate();
                  }
                }}
              />
              {references.length ? <ReferenceStrip {...referenceProps} /> : null}
              <div className={styles.bottomActionRow}>
                <Button
                  className={styles.bottomGenerateButton}
                  type='primary'
                  loading={generating}
                  disabled={disabled}
                  icon={<Play />}
                  onClick={onGenerate}
                >
                  {pendingCount
                    ? t('creativeStudio.video.generate.pending', {
                        defaultValue: '{{taskCount}} 个处理中',
                        taskCount: pendingCount,
                      })
                    : t('creativeStudio.video.generate.start', {
                        defaultValue: '开始创作',
                      })}
                </Button>
                <div className={styles.bottomTools}>
                  <Button
                    aria-label={t('creativeStudio.video.actions.clearPrompt', {
                      defaultValue: '清空提示词',
                    })}
                    icon={<CloseSmall />}
                    onClick={() => onPromptChange('')}
                  />
                  {onOpenPromptLibrary ? (
                    <Button
                      aria-label={t('creativeStudio.video.actions.openPromptLibrary', {
                        defaultValue: '打开提示词库',
                      })}
                      icon={<MagicWand />}
                      onClick={onOpenPromptLibrary}
                    />
                  ) : null}
                  <Button
                    aria-label={t('creativeStudio.video.actions.addReference', {
                      defaultValue: '添加参考素材',
                    })}
                    icon={<FolderPlus />}
                    onClick={onAddReferences}
                  />
                  <Button
                    aria-label={t('creativeStudio.video.actions.openParameters', {
                      defaultValue: '打开高级参数',
                    })}
                    icon={<SettingTwo />}
                    onClick={onOpenParameters}
                  />
                  <Button
                    aria-label={t('creativeStudio.video.layout.switchToSide', {
                      defaultValue: '切换到侧边工作台',
                    })}
                    icon={<LayoutOne />}
                    onClick={() => onLayoutChange('side')}
                  />
                </div>
              </div>
            </div>
            <SettingsGrid {...settings} compact />
          </div>
        </div>
      </aside>
    );
  }

  return (
    <aside className={styles.sideComposer} data-video-composer='side'>
      <header className={styles.composerHeader}>
        <div>
          <VideoTwo size={20} />
          <span>
            <strong>
              {t('creativeStudio.video.header.title', { defaultValue: '视频创作台' })}
            </strong>
            <small>
              {t('creativeStudio.video.header.settings', { defaultValue: '生成设置' })}
            </small>
          </span>
        </div>
        <LayoutSwitch layout={layout} onChange={onLayoutChange} />
      </header>

      <div className={styles.sideComposerBody}>
        <section className={styles.promptSection}>
          <div className={styles.sectionHeading}>
            <span>{t('creativeStudio.video.prompt.labelShort', { defaultValue: '提示词' })}</span>
            <div>
              <button type='button' onClick={() => onPromptChange('')}>
                {t('creativeStudio.video.actions.clear', { defaultValue: '清空' })}
              </button>
              {onOpenPromptLibrary ? (
                <button type='button' onClick={onOpenPromptLibrary}>
                  {t('creativeStudio.video.actions.promptLibrary', {
                    defaultValue: '提示词库',
                  })}
                </button>
              ) : null}
            </div>
          </div>
          <Input.TextArea
            value={prompt}
            onChange={onPromptChange}
            rows={7}
            placeholder={t('creativeStudio.video.prompt.placeholder', {
              defaultValue: '描述镜头运动、主体动作、场景氛围和画面风格',
            })}
            aria-label={t('creativeStudio.video.prompt.label', { defaultValue: '视频提示词' })}
          />
        </section>

        <ReferenceStrip {...referenceProps} />
        <SettingsGrid {...settings} />

        <button type='button' className={styles.parametersButton} onClick={onOpenParameters}>
          <SettingTwo size={15} />
          <span>
            {t('creativeStudio.video.actions.moreParameters', {
              defaultValue: '更多生成参数',
            })}
          </span>
          <Right size={13} />
        </button>
      </div>

      <footer className={styles.composerFooter}>
        <div>
          {pendingCount ? (
            <span>
              {t('creativeStudio.video.generate.pendingTasks', {
                defaultValue: '{{taskCount}} 个任务正在处理',
                taskCount: pendingCount,
              })}
            </span>
          ) : (
            <span>{t('creativeStudio.video.generate.ready', { defaultValue: '准备就绪' })}</span>
          )}
        </div>
        <Button
          type='primary'
          size='large'
          long
          loading={generating}
          disabled={disabled}
          icon={<Play />}
          onClick={onGenerate}
        >
          {t('creativeStudio.video.generate.start', { defaultValue: '开始创作' })}
        </Button>
      </footer>
    </aside>
  );
};

export default VideoWorkbenchComposer;
