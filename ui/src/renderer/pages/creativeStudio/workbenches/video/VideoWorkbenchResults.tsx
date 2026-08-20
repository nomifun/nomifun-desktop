/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  Check,
  CloseSmall,
  Copy,
  Delete,
  Download,
  Error,
  History,
  Loading,
  Plus,
  Refresh,
  VideoTwo,
} from '@icon-park/react';
import { Button, Checkbox, Progress, Tag } from '@arco-design/web-react';
import React from 'react';

import {
  clampVideoProgress,
  toggleAllVideoTasks,
  toggleVideoTaskSelection,
  videoResultsState,
} from './presentation';
import styles from './VideoWorkbench.module.css';
import type { VideoWorkbenchProps, VideoWorkbenchTask } from './types';

type ResultsProps = Pick<
  VideoWorkbenchProps,
  | 'tasks'
  | 'selectedTaskIds'
  | 'onSelectedTaskIdsChange'
  | 'onDeleteTasks'
  | 'onNewSession'
  | 'onLoadTask'
  | 'onRetryTask'
  | 'onInspectTask'
  | 'onCopyPrompt'
  | 'onDownloadTask'
>;

const taskStatusLabel = (task: VideoWorkbenchTask): string => {
  if (task.status === 'running') return '生成中';
  if (task.status === 'success') return '成功';
  return '失败';
};

const TaskMeta: React.FC<{
  task: VideoWorkbenchTask;
  onCopyPrompt?: (prompt: string) => void;
}> = ({ task, onCopyPrompt }) => (
  <div className={styles.taskMeta}>
    <div className={styles.taskPromptRow}>
      <p title={task.prompt}>{task.prompt}</p>
      {onCopyPrompt ? (
        <button
          type='button'
          aria-label={`复制任务 ${task.id} 的提示词`}
          onClick={() => onCopyPrompt(task.prompt)}
        >
          <Copy size={13} />
        </button>
      ) : null}
    </div>
    <div className={styles.taskTags}>
      <Tag>{task.createdAtLabel}</Tag>
      <Tag>{task.modelLabel}</Tag>
      <Tag>{task.sizeLabel}</Tag>
      <Tag>{task.resolutionLabel}</Tag>
      <Tag>{task.durationLabel}</Tag>
      <Tag>数量 {task.taskCount}</Tag>
    </div>
  </div>
);

const RunningVisual: React.FC<{ task: Extract<VideoWorkbenchTask, { status: 'running' }> }> = ({
  task,
}) => {
  const progress = clampVideoProgress(task.progress);
  return (
    <div className={styles.runningVisual}>
      <div className={styles.runningPattern} aria-hidden='true' />
      <div className={styles.runningCenter}>
        <Loading size={27} className={styles.spin} />
        <strong>{progress === null ? '生成中' : `正在创作 ${progress}%`}</strong>
        {task.elapsedLabel ? <span>{task.elapsedLabel}</span> : null}
      </div>
      {progress !== null ? (
        <div className={styles.progressBlock}>
          <span>
            <small>当前创作进度</small>
            <small>{progress}%</small>
          </span>
          <Progress percent={progress} size='small' showText={false} />
        </div>
      ) : null}
    </div>
  );
};

const SuccessVisual: React.FC<{
  task: Extract<VideoWorkbenchTask, { status: 'success' }>;
}> = ({ task }) => (
  <div className={styles.successVisual}>
    <video
      src={task.videoUrl}
      poster={task.posterUrl}
      controls
      preload='metadata'
      aria-label={`生成视频：${task.prompt}`}
    />
    <span className={styles.statusBadge} data-tone='success'>
      <Check size={11} />
      成功
    </span>
    {task.mediaMetaLabel ? <span className={styles.mediaMeta}>{task.mediaMetaLabel}</span> : null}
  </div>
);

const FailedVisual: React.FC<{
  task: Extract<VideoWorkbenchTask, { status: 'failed' }>;
}> = ({ task }) => (
  <div className={styles.failedVisual}>
    <Error size={30} />
    <strong>生成失败</strong>
    <span>{task.error}</span>
  </div>
);

const TaskVisual: React.FC<{ task: VideoWorkbenchTask }> = ({ task }) => {
  if (task.status === 'success') return <SuccessVisual task={task} />;
  if (task.status === 'failed') return <FailedVisual task={task} />;
  return <RunningVisual task={task} />;
};

const TaskActions: React.FC<{
  task: VideoWorkbenchTask;
  onLoadTask?: (taskId: string) => void;
  onRetryTask?: (taskId: string) => void;
  onInspectTask?: (taskId: string) => void;
  onDownloadTask?: (taskId: string) => void;
}> = ({ task, onLoadTask, onRetryTask, onInspectTask, onDownloadTask }) => {
  if (!onLoadTask && !onRetryTask && !onInspectTask && !onDownloadTask) return null;
  return (
    <div className={styles.taskActions}>
      <div>
        {onLoadTask ? (
          <Button size='mini' onClick={() => onLoadTask(task.id)}>
            载入
          </Button>
        ) : null}
        {task.status === 'failed' && onInspectTask ? (
          <Button size='mini' onClick={() => onInspectTask(task.id)}>
            详情
          </Button>
        ) : null}
      </div>
      <div>
        {task.status === 'failed' && onRetryTask ? (
          <Button
            size='mini'
            status='danger'
            icon={<Refresh />}
            onClick={() => onRetryTask(task.id)}
          >
            重试
          </Button>
        ) : null}
        {task.status === 'success' && onDownloadTask ? (
          <Button
            size='mini'
            icon={<Download />}
            onClick={() => onDownloadTask(task.id)}
          >
            下载
          </Button>
        ) : null}
      </div>
    </div>
  );
};

const VideoWorkbenchResults: React.FC<ResultsProps> = ({
  tasks,
  selectedTaskIds,
  onSelectedTaskIdsChange,
  onDeleteTasks,
  onNewSession,
  onLoadTask,
  onRetryTask,
  onInspectTask,
  onCopyPrompt,
  onDownloadTask,
}) => {
  const taskIds = tasks.map((task) => task.id);
  const visibleSelectedIds = selectedTaskIds.filter((id) => taskIds.includes(id));
  const allSelected = tasks.length > 0 && tasks.every((task) => selectedTaskIds.includes(task.id));
  const pendingCount = tasks.filter((task) => task.status === 'running').length;

  return (
    <section
      className={styles.resultsPanel}
      data-video-workbench-results
      data-results-state={videoResultsState(tasks)}
      data-result-count={tasks.length}
    >
      <header className={styles.resultsHeader}>
        <div className={styles.resultsTitle}>
          <History size={17} />
          <h2>全部成果</h2>
          <Tag>{tasks.length}</Tag>
          {pendingCount ? <Tag color='arcoblue'>{pendingCount} 个生成中</Tag> : null}
        </div>
        <div className={styles.resultsActions}>
          {onNewSession ? (
            <Button size='small' icon={<Plus />} onClick={onNewSession}>
              新建
            </Button>
          ) : null}
          <Button
            size='small'
            icon={allSelected ? <CloseSmall /> : <Check />}
            disabled={tasks.length === 0}
            onClick={() =>
              onSelectedTaskIdsChange(toggleAllVideoTasks(taskIds, selectedTaskIds))
            }
          >
            {allSelected ? '取消全选' : '全选'}
          </Button>
          <Button
            size='small'
            status='danger'
            icon={<Delete />}
            disabled={visibleSelectedIds.length === 0}
            onClick={() => onDeleteTasks(visibleSelectedIds)}
          >
            删除{visibleSelectedIds.length ? ` ${visibleSelectedIds.length}` : ''}
          </Button>
        </div>
      </header>

      {tasks.length === 0 ? (
        <div className={styles.emptyResults} data-video-result-state='empty'>
          <span className={styles.emptyIcon}>
            <VideoTwo size={40} />
          </span>
          <strong>还没有生成视频</strong>
          <p>输入提示词并选择视频模型，生成成果会出现在这里。</p>
        </div>
      ) : (
        <div className={styles.resultGrid}>
          {tasks.map((task) => {
            const selected = selectedTaskIds.includes(task.id);
            return (
              <article
                key={task.id}
                className={styles.resultCard}
                data-video-result-state={task.status}
                data-selected={selected || undefined}
              >
                <div className={styles.cardOverlayActions}>
                  <Checkbox
                    checked={selected}
                    aria-label={`选择任务 ${task.id}`}
                    onChange={(checked) =>
                      onSelectedTaskIdsChange(
                        toggleVideoTaskSelection(selectedTaskIds, task.id, checked)
                      )
                    }
                  />
                  <Button
                    size='mini'
                    type='text'
                    status='danger'
                    icon={<Delete />}
                    aria-label={`删除任务 ${task.id}`}
                    onClick={() => onDeleteTasks([task.id])}
                  />
                </div>
                <span className={styles.cardStatus} data-status={task.status}>
                  {taskStatusLabel(task)}
                </span>
                <TaskVisual task={task} />
                <TaskMeta task={task} onCopyPrompt={onCopyPrompt} />
                <TaskActions
                  task={task}
                  onLoadTask={onLoadTask}
                  onRetryTask={onRetryTask}
                  onInspectTask={onInspectTask}
                  onDownloadTask={onDownloadTask}
                />
              </article>
            );
          })}
        </div>
      )}
    </section>
  );
};

export default VideoWorkbenchResults;
