/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  Check,
  CloseOne,
  CloseSmall,
  Copy,
  Delete,
  Download,
  Error,
  History,
  Loading,
  Plus,
  Refresh,
  Time,
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
  | 'onCancelTask'
  | 'onInspectTask'
  | 'onCopyPrompt'
  | 'onDownloadTask'
  | 'historyLoading'
  | 'historyError'
  | 'historyLoadingMore'
  | 'historyHasMore'
  | 'onLoadMoreTasks'
>;

const taskStatusLabel = (task: VideoWorkbenchTask): string => {
  if (task.status === 'queued') return '排队中';
  if (task.status === 'running') return '生成中';
  if (task.status === 'succeeded') return '成功';
  if (task.status === 'failed') return '失败';
  return '已取消';
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
      <Tag title={`${task.model.providerId}/${task.model.model}`}>{task.modelLabel}</Tag>
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

const QueuedVisual: React.FC<{
  task: Extract<VideoWorkbenchTask, { status: 'queued' }>;
}> = ({ task }) => (
  <div className={styles.queuedVisual}>
    <div className={styles.runningPattern} aria-hidden='true' />
    <div className={styles.runningCenter}>
      <Time size={27} />
      <strong>{task.statusLabel || '排队中'}</strong>
      <span>等待模型开始处理</span>
    </div>
  </div>
);

const SuccessVisual: React.FC<{
  task: Extract<VideoWorkbenchTask, { status: 'succeeded' }>;
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

const CanceledVisual: React.FC<{
  task: Extract<VideoWorkbenchTask, { status: 'canceled' }>;
}> = ({ task }) => (
  <div className={styles.canceledVisual}>
    <CloseOne size={30} />
    <strong>已取消</strong>
    <span>{task.message || '任务已取消，没有生成视频'}</span>
  </div>
);

const TaskVisual: React.FC<{ task: VideoWorkbenchTask }> = ({ task }) => {
  if (task.status === 'queued') return <QueuedVisual task={task} />;
  if (task.status === 'succeeded') return <SuccessVisual task={task} />;
  if (task.status === 'failed') return <FailedVisual task={task} />;
  if (task.status === 'canceled') return <CanceledVisual task={task} />;
  return <RunningVisual task={task} />;
};

const TaskActions: React.FC<{
  task: VideoWorkbenchTask;
  onLoadTask?: (taskId: string) => void;
  onRetryTask?: (taskId: string) => void;
  onCancelTask?: (taskId: string) => void;
  onInspectTask?: (taskId: string) => void;
  onDownloadTask?: (taskId: string) => void;
}> = ({ task, onLoadTask, onRetryTask, onCancelTask, onInspectTask, onDownloadTask }) => {
  if (!onLoadTask && !onRetryTask && !onCancelTask && !onInspectTask && !onDownloadTask) return null;
  return (
    <div className={styles.taskActions}>
      <div>
        {onLoadTask ? (
          <Button size='mini' onClick={() => onLoadTask(task.id)}>
            载入
          </Button>
        ) : null}
        {(task.status === 'failed' || task.status === 'canceled') && onInspectTask ? (
          <Button size='mini' onClick={() => onInspectTask(task.id)}>
            详情
          </Button>
        ) : null}
      </div>
      <div>
        {(task.status === 'failed' || task.status === 'canceled') && onRetryTask && task.retryable !== false ? (
          <Button
            size='mini'
            status='danger'
            icon={<Refresh />}
            onClick={() => onRetryTask(task.id)}
          >
            重试
          </Button>
        ) : null}
        {(task.status === 'queued' || task.status === 'running') && onCancelTask ? (
          <Button
            size='mini'
            status='danger'
            onClick={() => onCancelTask(task.id)}
          >
            取消
          </Button>
        ) : null}
        {task.status === 'succeeded' && onDownloadTask ? (
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
  onCancelTask,
  onInspectTask,
  onCopyPrompt,
  onDownloadTask,
  historyLoading,
  historyError,
  historyLoadingMore,
  historyHasMore,
  onLoadMoreTasks,
}) => {
  const deletionEnabled = Boolean(onDeleteTasks);
  const taskIds = tasks.filter((task) => task.deletable).map((task) => task.id);
  const visibleSelectedIds = selectedTaskIds.filter((id) => taskIds.includes(id));
  const allSelected = taskIds.length > 0 && taskIds.every((id) => selectedTaskIds.includes(id));
  const pendingCount = tasks.filter(
    (task) => task.status === 'queued' || task.status === 'running'
  ).length;

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
          <Tag>已加载 {tasks.length}</Tag>
          {pendingCount ? <Tag color='arcoblue'>{pendingCount} 个处理中</Tag> : null}
        </div>
        <div className={styles.resultsActions}>
          {onNewSession ? (
            <Button size='small' icon={<Plus />} onClick={onNewSession}>
              新建
            </Button>
          ) : null}
          {deletionEnabled ? <Button
            size='small'
            icon={allSelected ? <CloseSmall /> : <Check />}
            disabled={taskIds.length === 0}
            onClick={() =>
              onSelectedTaskIdsChange(toggleAllVideoTasks(taskIds, selectedTaskIds))
            }
          >
            {allSelected ? '取消全选' : '全选'}
          </Button> : null}
          {deletionEnabled ? <Button
            size='small'
            status='danger'
            icon={<Delete />}
            disabled={visibleSelectedIds.length === 0}
            onClick={() => onDeleteTasks?.(visibleSelectedIds)}
          >
            移除{visibleSelectedIds.length ? ` ${visibleSelectedIds.length}` : ''}
          </Button> : null}
        </div>
      </header>

      {tasks.length === 0 ? (
        <div className={styles.emptyResults} data-video-result-state='empty'>
          <span className={styles.emptyIcon}>{historyLoading ? <Loading size={40} className={styles.spin} /> : historyError ? <Error size={40} /> : <VideoTwo size={40} />}</span>
          <strong>{historyLoading ? '正在恢复生成历史' : historyError ? '生成历史加载失败' : '还没有生成视频'}</strong>
          <p>{historyLoading ? '正在读取当前项目的真实任务与结果。' : historyError ?? '输入提示词并选择视频模型，生成成果会出现在这里。'}</p>
        </div>
      ) : (
        <div className={styles.resultGrid}>
          {tasks.map((task) => {
            const selected = Boolean(task.deletable && selectedTaskIds.includes(task.id));
            return (
              <article
                key={task.id}
                className={styles.resultCard}
                data-video-result-state={task.status}
                data-provider-id={task.model.providerId}
                data-model={task.model.model}
                data-selected={selected || undefined}
              >
                {deletionEnabled && task.deletable ? <div className={styles.cardOverlayActions}>
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
                    aria-label={`从历史移除 ${task.id}`}
                    onClick={() => onDeleteTasks?.([task.id])}
                  />
                </div> : null}
                <span className={styles.cardStatus} data-status={task.status}>
                  {taskStatusLabel(task)}
                </span>
                <TaskVisual task={task} />
                <TaskMeta task={task} onCopyPrompt={onCopyPrompt} />
                <TaskActions
                  task={task}
                  onLoadTask={onLoadTask}
                  onRetryTask={onRetryTask}
                  onCancelTask={onCancelTask}
                  onInspectTask={onInspectTask}
                  onDownloadTask={onDownloadTask}
                />
              </article>
            );
          })}
        </div>
      )}
      {historyHasMore && onLoadMoreTasks ? (
        <div className={styles.historyFooter}>
          <Button loading={historyLoadingMore} onClick={onLoadMoreTasks}>
            {historyLoadingMore ? '正在加载…' : '加载更多历史'}
          </Button>
        </div>
      ) : null}
    </section>
  );
};

export default VideoWorkbenchResults;
