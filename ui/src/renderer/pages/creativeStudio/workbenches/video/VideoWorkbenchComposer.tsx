/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  CloseSmall,
  FolderPlus,
  LayoutOne,
  Left,
  MagicWand,
  Play,
  Right,
  SettingTwo,
} from '@icon-park/react';
import { Button, Input } from '@arco-design/web-react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import { WorkbenchAddReference, WorkbenchComposerHeader, WorkbenchReferenceCard, WorkbenchReferenceCount } from '../WorkbenchComposerControls';

import { normalizeVideoTaskCount } from './presentation';
import contentSiderStyles from '@/renderer/components/layout/ContentSider/ContentSider.module.css';
import composerStyles from '../WorkbenchComposer.module.css';
import styles from './VideoWorkbench.module.css';
import type {
  VideoWorkbenchChoice,
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
    <WorkbenchReferenceCard
      kind={item.kind}
      src={item.originalUrl ?? (item.kind === 'image' ? item.previewUrl : undefined)}
      posterSrc={item.previewUrl}
      name={item.name}
      removeLabel={t('creativeStudio.video.references.remove', { defaultValue: '移除参考素材 {{name}}', name: item.name })}
      onRemove={onRemove}
      orderControls={onMove ? (
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
    />
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
      <div className={`${styles.sectionHeading} ${composerStyles.sectionHeading}`}>
        <span>{t('creativeStudio.video.references.title', { defaultValue: '参考素材' })}</span>
        <WorkbenchReferenceCount count={references.length} />
      </div>
      <div className={composerStyles.referenceStrip}>
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
        <WorkbenchAddReference
          onClick={onAddReferences}
          label={references.length
            ? t('creativeStudio.workbenchComposer.addMore', { defaultValue: '继续添加' })
            : addReferenceLabel ?? t('creativeStudio.video.references.addDefault', { defaultValue: '添加图片、视频或音频' })}
        />
      </div>
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
        {compact ? <span>{t('creativeStudio.video.settings.model', { defaultValue: '模型' })}</span> : null}
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
      <aside className={`${styles.bottomComposer} ${composerStyles.root}`} data-video-composer='bottom'>
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
    <aside className={`${styles.sideComposer} ${composerStyles.root}`} data-video-composer='side'>
      <WorkbenchComposerHeader kind='video' layout={layout} onLayoutChange={onLayoutChange} />

      <div className={`${composerStyles.content} ${contentSiderStyles.scrollArea}`}>
        <section className={styles.promptSection}>
          <div className={`${styles.sectionHeading} ${composerStyles.sectionHeading}`}>
            <span>{t('creativeStudio.video.prompt.labelShort', { defaultValue: '提示词' })}</span>
            <div>
              <button type='button' className={composerStyles.toolButton} disabled={!prompt} onClick={() => onPromptChange('')}>
                {t('creativeStudio.video.actions.clear', { defaultValue: '清空' })}
              </button>
              {onOpenPromptLibrary ? (
                <button type='button' className={composerStyles.toolButton} onClick={onOpenPromptLibrary}>
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
            rows={5}
            placeholder={t('creativeStudio.video.prompt.placeholder', {
              defaultValue: '描述镜头运动、主体动作、场景氛围和画面风格',
            })}
            aria-label={t('creativeStudio.video.prompt.label', { defaultValue: '视频提示词' })}
          />
        </section>

        <ReferenceStrip {...referenceProps} />
        <SettingsGrid {...settings} />

        <button type='button' className={`${styles.parametersButton} ${composerStyles.toolButton}`} onClick={onOpenParameters}>
          <SettingTwo size={15} />
          <span>
            {t('creativeStudio.video.actions.moreParameters', {
              defaultValue: '更多生成参数',
            })}
          </span>
          <Right size={13} />
        </button>
      </div>

      <footer className={`${styles.composerFooter} ${composerStyles.footer}`}>
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
