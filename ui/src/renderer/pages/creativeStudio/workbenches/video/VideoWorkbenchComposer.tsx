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
  Pic,
  Play,
  Plus,
  Right,
  SettingTwo,
  VideoTwo,
  Voice,
} from '@icon-park/react';
import { Button, Input } from '@arco-design/web-react';
import React from 'react';

import { normalizeVideoTaskCount } from './presentation';
import styles from './VideoWorkbench.module.css';
import type {
  VideoReferenceKind,
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

const referenceIcon = (kind: VideoReferenceKind): React.ReactNode => {
  if (kind === 'video') return <VideoTwo size={17} />;
  if (kind === 'audio') return <Voice size={17} />;
  return <Pic size={17} />;
};

const ReferenceItem: React.FC<{
  item: VideoWorkbenchReference;
  index: number;
  total: number;
  onRemove: () => void;
  onMove?: (direction: -1 | 1) => void;
}> = ({ item, index, total, onRemove, onMove }) => (
  <article className={styles.referenceItem} data-reference-kind={item.kind}>
    <div className={styles.referencePreview}>
      {item.previewUrl ? (
        <img src={item.previewUrl} alt={item.name} />
      ) : (
        <span aria-hidden='true'>{referenceIcon(item.kind)}</span>
      )}
    </div>
    <span className={styles.referenceName} title={item.name}>
      {item.name}
    </span>
    {onMove ? (
      <span className={styles.referenceOrder}>
        <button
          type='button'
          aria-label={`前移参考素材 ${item.name}`}
          disabled={index === 0}
          onClick={() => onMove(-1)}
        >
          <Left size={11} />
        </button>
        <button
          type='button'
          aria-label={`后移参考素材 ${item.name}`}
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
      aria-label={`移除参考素材 ${item.name}`}
      onClick={onRemove}
    >
      <CloseSmall size={13} />
    </button>
  </article>
);

const ReferenceStrip: React.FC<
  Pick<
    ComposerProps,
    'references' | 'onAddReferences' | 'onRemoveReference' | 'onMoveReference'
  >
> = ({ references, onAddReferences, onRemoveReference, onMoveReference }) => (
  <div className={styles.referenceSection}>
    <div className={styles.sectionHeading}>
      <span>参考素材</span>
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
        <span>{references.length ? '继续添加' : '添加图片、视频或音频'}</span>
      </button>
    </div>
  </div>
);

const LayoutSwitch: React.FC<{
  layout: VideoWorkbenchLayout;
  onChange: (layout: VideoWorkbenchLayout) => void;
}> = ({ layout, onChange }) => (
  <div className={styles.layoutSwitch} aria-label='工作台布局'>
    <button
      type='button'
      data-active={layout === 'side' || undefined}
      aria-pressed={layout === 'side'}
      onClick={() => onChange('side')}
    >
      <LayoutOne size={14} />
      <span>侧边</span>
    </button>
    <button
      type='button'
      data-active={layout === 'bottom' || undefined}
      aria-pressed={layout === 'bottom'}
      onClick={() => onChange('bottom')}
    >
      <LayoutTwo size={14} />
      <span>底部</span>
    </button>
  </div>
);

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

const SettingsGrid: React.FC<
  Pick<
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
  >
> = ({
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
}) => (
  <div className={styles.settingsGrid}>
    <div className={styles.modelControl}>
      <span>模型</span>
      <div>{modelSlot}</div>
    </div>
    <QuickSelect
      label='清晰度'
      value={resolution}
      options={resolutionOptions}
      onChange={onResolutionChange}
    />
    <QuickSelect label='尺寸' value={size} options={sizeOptions} onChange={onSizeChange} />
    <QuickSelect
      label='时长'
      value={duration}
      options={durationOptions}
      onChange={onDurationChange}
    />
    <label className={styles.quickControl}>
      <span>任务数量</span>
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

const VideoWorkbenchComposer: React.FC<ComposerProps> = ({
  layout,
  onLayoutChange,
  prompt,
  onPromptChange,
  onGenerate,
  generating = false,
  submitDisabled = false,
  references,
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
  const pendingCount = tasks.filter((task) => task.status === 'running').length;
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
    onAddReferences,
    onRemoveReference,
    onMoveReference,
  };

  if (layout === 'bottom') {
    return (
      <aside className={styles.bottomComposer} data-video-composer='bottom'>
        <div className={styles.bottomComposerSurface}>
          <div className={styles.bottomPromptRow}>
            <Input.TextArea
              value={prompt}
              onChange={onPromptChange}
              autoSize={{ minRows: 2, maxRows: 4 }}
              placeholder='描述镜头运动、主体动作、场景氛围和画面风格'
              aria-label='视频提示词'
              onPressEnter={(event) => {
                if (!event.shiftKey && !disabled && !generating) {
                  event.preventDefault();
                  onGenerate();
                }
              }}
            />
            <div className={styles.bottomPromptActions}>
              <Button
                aria-label='清空提示词'
                icon={<CloseSmall />}
                onClick={() => onPromptChange('')}
              />
              {onOpenPromptLibrary ? (
                <Button
                  aria-label='打开提示词库'
                  icon={<MagicWand />}
                  onClick={onOpenPromptLibrary}
                />
              ) : null}
              <Button
                aria-label='添加参考素材'
                icon={<FolderPlus />}
                onClick={onAddReferences}
              />
              <Button
                aria-label='打开高级参数'
                icon={<SettingTwo />}
                onClick={onOpenParameters}
              />
              <Button
                aria-label='切换到侧边工作台'
                icon={<LayoutOne />}
                onClick={() => onLayoutChange('side')}
              />
              <Button
                type='primary'
                loading={generating}
                disabled={disabled}
                icon={<Play />}
                onClick={onGenerate}
              >
                {pendingCount ? `${pendingCount} 个生成中` : '开始创作'}
              </Button>
            </div>
          </div>
          <SettingsGrid {...settings} />
          {references.length ? <ReferenceStrip {...referenceProps} /> : null}
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
            <strong>视频创作台</strong>
            <small>生成设置</small>
          </span>
        </div>
        <LayoutSwitch layout={layout} onChange={onLayoutChange} />
      </header>

      <div className={styles.sideComposerBody}>
        <section className={styles.promptSection}>
          <div className={styles.sectionHeading}>
            <span>提示词</span>
            <div>
              <button type='button' onClick={() => onPromptChange('')}>
                清空
              </button>
              {onOpenPromptLibrary ? (
                <button type='button' onClick={onOpenPromptLibrary}>
                  提示词库
                </button>
              ) : null}
            </div>
          </div>
          <Input.TextArea
            value={prompt}
            onChange={onPromptChange}
            rows={7}
            placeholder='描述镜头运动、主体动作、场景氛围和画面风格'
            aria-label='视频提示词'
          />
        </section>

        <ReferenceStrip {...referenceProps} />
        <SettingsGrid {...settings} />

        <button type='button' className={styles.parametersButton} onClick={onOpenParameters}>
          <SettingTwo size={15} />
          <span>更多生成参数</span>
          <Right size={13} />
        </button>
      </div>

      <footer className={styles.composerFooter}>
        <div>
          {pendingCount ? <span>{pendingCount} 个任务正在生成</span> : <span>准备就绪</span>}
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
          开始创作
        </Button>
      </footer>
    </aside>
  );
};

export default VideoWorkbenchComposer;
