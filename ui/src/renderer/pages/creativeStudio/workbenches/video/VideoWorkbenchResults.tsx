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
import { useTranslation } from 'react-i18next';
import { CreativeAssetUnavailable } from '../../assets/components/CreativeAssetUnavailable';
import CreativeVideoMedia from '../../assets/components/CreativeVideoMedia';

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

const taskStatusLabel = (
  t: ReturnType<typeof useTranslation>['t'],
  task: VideoWorkbenchTask
): string => {
  if (task.status === 'queued') return t('creativeStudio.video.task.queued', { defaultValue: '排队中' });
  if (task.status === 'running') return t('creativeStudio.video.task.running', { defaultValue: '生成中' });
  if (task.status === 'succeeded') return t('creativeStudio.video.task.succeeded', { defaultValue: '成功' });
  if (task.status === 'failed') return t('creativeStudio.video.task.failed', { defaultValue: '失败' });
  return t('creativeStudio.video.task.canceled', { defaultValue: '已取消' });
};

const TaskMeta: React.FC<{
  task: VideoWorkbenchTask;
  onCopyPrompt?: (prompt: string) => void;
}> = ({ task, onCopyPrompt }) => {
  const { t } = useTranslation();
  return (
  <div className={styles.taskMeta}>
    {task.hasDeletedInputs ? <p role='status'>{t('creativeStudio.assets.deletedReference', { defaultValue: '引用素材已删除，请重新选择后再生成。' })}</p> : null}
    <div className={styles.taskPromptRow}>
      <p title={task.prompt}>{task.prompt}</p>
      {onCopyPrompt ? (
        <button
          type='button'
          aria-label={t('creativeStudio.video.results.copyPrompt', {
            defaultValue: '复制任务 {{id}} 的提示词',
            id: task.id,
          })}
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
      <Tag>
        {t('creativeStudio.video.results.taskCount', {
          defaultValue: '数量 {{taskCount}}',
          taskCount: task.taskCount,
        })}
      </Tag>
    </div>
  </div>
  );
};

const RunningVisual: React.FC<{ task: Extract<VideoWorkbenchTask, { status: 'running' }> }> = ({
  task,
}) => {
  const { t } = useTranslation();
  const progress = clampVideoProgress(task.progress);
  return (
    <div className={styles.runningVisual}>
      <div className={styles.runningPattern} aria-hidden='true' />
      <div className={styles.runningCenter}>
        <Loading size={27} className={styles.spin} />
        <strong>
          {progress === null
            ? t('creativeStudio.video.task.running', { defaultValue: '生成中' })
            : t('creativeStudio.video.results.creatingProgress', {
                defaultValue: '正在创作 {{progress}}%',
                progress,
              })}
        </strong>
        {task.elapsedLabel ? <span>{task.elapsedLabel}</span> : null}
      </div>
      {progress !== null ? (
        <div className={styles.progressBlock}>
          <span>
            <small>
              {t('creativeStudio.video.results.currentProgress', {
                defaultValue: '当前创作进度',
              })}
            </small>
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
}> = ({ task }) => {
  const { t } = useTranslation();
  return (
  <div className={styles.queuedVisual}>
    <div className={styles.runningPattern} aria-hidden='true' />
    <div className={styles.runningCenter}>
      <Time size={27} />
      <strong>
        {task.statusLabel ||
          t('creativeStudio.video.task.queued', { defaultValue: '排队中' })}
      </strong>
      <span>
        {t('creativeStudio.video.results.waitingForModel', {
          defaultValue: '等待模型开始处理',
        })}
      </span>
    </div>
  </div>
  );
};

const SuccessVisual: React.FC<{
  task: Extract<VideoWorkbenchTask, { status: 'succeeded' }>;
}> = ({ task }) => {
  const { t } = useTranslation();
  return (
  <div className={styles.successVisual}>
    {task.availability && task.availability !== 'available' ? <CreativeAssetUnavailable status={task.availability} /> : <CreativeVideoMedia
      src={task.videoUrl}
      poster={task.posterUrl}
      controls
      playsInline
      preload='metadata'
      aria-label={t('creativeStudio.video.results.generatedVideo', {
        defaultValue: '生成视频：{{prompt}}',
        prompt: task.prompt,
      })}
    />}
    <span className={styles.statusBadge} data-tone='success'>
      <Check size={11} />
      {t('creativeStudio.video.task.succeeded', { defaultValue: '成功' })}
    </span>
    {task.mediaMetaLabel ? <span className={styles.mediaMeta}>{task.mediaMetaLabel}</span> : null}
  </div>
  );
};

const FailedVisual: React.FC<{
  task: Extract<VideoWorkbenchTask, { status: 'failed' }>;
}> = ({ task }) => {
  const { t } = useTranslation();
  return (
  <div className={styles.failedVisual}>
    <Error size={30} />
    <strong>
      {t('creativeStudio.video.results.generationFailed', { defaultValue: '生成失败' })}
    </strong>
    <span>{task.error}</span>
  </div>
  );
};

const CanceledVisual: React.FC<{
  task: Extract<VideoWorkbenchTask, { status: 'canceled' }>;
}> = ({ task }) => {
  const { t } = useTranslation();
  return (
  <div className={styles.canceledVisual}>
    <CloseOne size={30} />
    <strong>{t('creativeStudio.video.task.canceled', { defaultValue: '已取消' })}</strong>
    <span>
      {task.message ||
        t('creativeStudio.video.results.canceledDescription', {
          defaultValue: '任务已取消，没有生成视频',
        })}
    </span>
  </div>
  );
};

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
  const { t } = useTranslation();
  if (!onLoadTask && !onRetryTask && !onCancelTask && !onInspectTask && !onDownloadTask) return null;
  return (
    <div className={styles.taskActions}>
      <div>
        {onLoadTask ? (
          <Button size='mini' onClick={() => onLoadTask(task.id)}>
            {t('creativeStudio.video.actions.load', { defaultValue: '载入' })}
          </Button>
        ) : null}
        {(task.status === 'failed' || task.status === 'canceled') && onInspectTask ? (
          <Button size='mini' onClick={() => onInspectTask(task.id)}>
            {t('creativeStudio.video.actions.details', { defaultValue: '详情' })}
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
            {t('creativeStudio.video.actions.retry', { defaultValue: '重试' })}
          </Button>
        ) : null}
        {(task.status === 'queued' || task.status === 'running') && onCancelTask ? (
          <Button
            size='mini'
            status='danger'
            onClick={() => onCancelTask(task.id)}
          >
            {t('creativeStudio.video.actions.cancel', { defaultValue: '取消' })}
          </Button>
        ) : null}
        {task.status === 'succeeded' && onDownloadTask ? (
          <Button
            size='mini'
            icon={<Download />}
            disabled={Boolean(task.availability && task.availability !== 'available')}
            onClick={() => onDownloadTask(task.id)}
          >
            {t('creativeStudio.video.actions.download', { defaultValue: '下载' })}
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
  const { t } = useTranslation();
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
          <h2>{t('creativeStudio.video.results.title', { defaultValue: '全部成果' })}</h2>
          <Tag>
            {t('creativeStudio.video.results.loadedCount', {
              defaultValue: '已加载 {{resultCount}}',
              resultCount: tasks.length,
            })}
          </Tag>
          {pendingCount ? (
            <Tag color='arcoblue'>
              {t('creativeStudio.video.results.pendingCount', {
                defaultValue: '{{taskCount}} 个处理中',
                taskCount: pendingCount,
              })}
            </Tag>
          ) : null}
        </div>
        <div className={styles.resultsActions}>
          {onNewSession ? (
            <Button size='small' icon={<Plus />} onClick={onNewSession}>
              {t('creativeStudio.video.actions.new', { defaultValue: '新建' })}
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
            {allSelected
              ? t('creativeStudio.video.results.clearAll', { defaultValue: '取消全选' })
              : t('creativeStudio.video.results.selectAll', { defaultValue: '全选' })}
          </Button> : null}
          {deletionEnabled ? <Button
            size='small'
            status='danger'
            icon={<Delete />}
            disabled={visibleSelectedIds.length === 0}
            onClick={() => onDeleteTasks?.(visibleSelectedIds)}
          >
            {t('creativeStudio.video.results.removeSelected', {
              defaultValue: '移除{{suffix}}',
              suffix: visibleSelectedIds.length ? ` ${visibleSelectedIds.length}` : '',
            })}
          </Button> : null}
        </div>
      </header>

      {tasks.length === 0 ? (
        <div className={styles.emptyResults} data-video-result-state='empty'>
          <span className={styles.emptyIcon}>{historyLoading ? <Loading size={40} className={styles.spin} /> : historyError ? <Error size={40} /> : <VideoTwo size={40} />}</span>
          <strong>
            {historyLoading
              ? t('creativeStudio.video.results.historyLoading', {
                  defaultValue: '正在恢复生成历史',
                })
              : historyError
                ? t('creativeStudio.video.results.historyFailed', {
                    defaultValue: '生成历史加载失败',
                  })
                : t('creativeStudio.video.results.emptyTitle', {
                    defaultValue: '还没有生成视频',
                  })}
          </strong>
          <p>
            {historyLoading
              ? t('creativeStudio.video.results.historyLoadingDescription', {
                  defaultValue: '正在读取当前工作台的真实任务与结果。',
                })
              : historyError ??
                t('creativeStudio.video.results.emptyDescription', {
                  defaultValue: '输入提示词并选择视频模型，生成成果会出现在这里。',
                })}
          </p>
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
                    aria-label={t('creativeStudio.video.results.selectTask', {
                      defaultValue: '选择任务 {{id}}',
                      id: task.id,
                    })}
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
                    aria-label={t('creativeStudio.video.results.removeFromHistory', {
                      defaultValue: '从历史移除 {{id}}',
                      id: task.id,
                    })}
                    onClick={() => onDeleteTasks?.([task.id])}
                  />
                </div> : null}
                <span className={styles.cardStatus} data-status={task.status}>
                  {taskStatusLabel(t, task)}
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
            {historyLoadingMore
              ? t('creativeStudio.video.results.loadingMore', { defaultValue: '正在加载…' })
              : t('creativeStudio.video.results.loadMore', { defaultValue: '加载更多历史' })}
          </Button>
        </div>
      ) : null}
    </section>
  );
};

export default VideoWorkbenchResults;
