/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  Check,
  CloseOne,
  CloseSmall,
  Delete,
  Error,
  History,
  Loading,
  Pic,
  Refresh,
  Time,
} from '@icon-park/react';
import { Button, Checkbox, Progress, Tag } from '@arco-design/web-react';
import React from 'react';
import {
  nextImageWorkbenchSelection,
  type ImageWorkbenchResult,
  type ImageWorkbenchTaskSummary,
} from './types';
import styles from './ImageWorkbench.module.css';

interface ImageWorkbenchResultsProps {
  results: readonly ImageWorkbenchResult[];
  selectedResultIds: readonly string[];
  task: ImageWorkbenchTaskSummary;
  onSelectionChange(resultIds: string[]): void;
  onDeleteResult?(resultId: string): void;
  onDeleteSelected?(resultIds: string[]): void;
  onRetryResult?(resultId: string): void;
  onLoadResult?(taskId: string): void;
  onCancelTask?(taskId: string): void;
  historyLoading?: boolean;
  historyError?: string;
  historyLoadingMore?: boolean;
  historyHasMore?: boolean;
  onLoadMoreResults?(): void;
}

const taskStateLabel = (task: ImageWorkbenchTaskSummary): string | null => {
  switch (task.state) {
    case 'queued':
      return `${task.pendingCount} 个排队中`;
    case 'running':
      return `${task.pendingCount} 个生成中`;
    case 'succeeded':
      return '任务已完成';
    case 'failed':
      return '最近任务失败';
    case 'canceled':
      return '最近任务已取消';
    default:
      return null;
  }
};

const TaskMeta: React.FC<{
  result: ImageWorkbenchResult;
  onLoadResult?(taskId: string): void;
}> = ({ result, onLoadResult }) => (
  <div className={styles.resultMeta}>
    <p title={result.prompt}>{result.prompt}</p>
    <div>
      <Tag title={`${result.model.providerId}/${result.model.model}`}>{result.modelLabel}</Tag>
      {result.createdAtLabel ? <Tag>{result.createdAtLabel}</Tag> : null}
      {result.durationLabel ? <Tag>{result.durationLabel}</Tag> : null}
      {onLoadResult ? (
        <Button size='mini' onClick={() => onLoadResult(result.taskId)}>
          载入
        </Button>
      ) : null}
    </div>
  </div>
);

const ResultVisual: React.FC<{
  result: ImageWorkbenchResult;
  onRetryResult?(resultId: string): void;
  onCancelTask?(taskId: string): void;
}> = ({ result, onRetryResult, onCancelTask }) => {
  if (result.status === 'succeeded') {
    const first = result.outputs[0];
    return (
      <div className={styles.successVisual}>
        <div className={styles.successGallery} data-image-output-count={result.outputs.length}>
          {result.outputs.map((output) => (
            <img key={output.assetId} src={output.imageUrl} alt={output.alt} />
          ))}
        </div>
        <span className={styles.resultBadge} data-tone='success'>
          <Check /> 成功 · {result.outputs.length} 张
        </span>
        {first?.width && first.height ? (
          <span className={styles.imageDimensions}>
            {first.width} × {first.height}
            {first.sizeLabel ? ` · ${first.sizeLabel}` : ''}
          </span>
        ) : null}
      </div>
    );
  }

  if (result.status === 'failed') {
    return (
      <div className={styles.failedVisual}>
        <Error size={30} />
        <strong>生成失败</strong>
        <span>{result.errorMessage}</span>
        {onRetryResult && result.retryable !== false ? (
          <Button size='small' status='danger' icon={<Refresh />} onClick={() => onRetryResult(result.id)}>
            重试
          </Button>
        ) : null}
      </div>
    );
  }

  if (result.status === 'canceled') {
    return (
      <div className={styles.canceledVisual}>
        <CloseOne size={30} />
        <strong>已取消</strong>
        <span>{result.message || '任务已取消，没有生成图片'}</span>
        {onRetryResult && result.retryable !== false ? (
          <Button size='small' icon={<Refresh />} onClick={() => onRetryResult(result.id)}>
            重新生成
          </Button>
        ) : null}
      </div>
    );
  }

  if (result.status === 'queued') {
    return (
      <div className={styles.queuedVisual}>
        <div className={styles.runningPattern} aria-hidden='true' />
        <Time size={26} />
        <strong>{result.statusLabel || '排队中'}</strong>
        <span>等待模型开始处理</span>
        {onCancelTask ? (
          <Button size='small' status='danger' onClick={() => onCancelTask(result.taskId)}>
            取消任务
          </Button>
        ) : null}
      </div>
    );
  }

  return (
    <div className={styles.runningVisual}>
      <div className={styles.runningPattern} aria-hidden='true' />
      <Loading size={26} className={styles.spin} />
      <strong>{result.statusLabel || '生成中'}</strong>
      {result.progress !== undefined ? (
        <Progress percent={Math.max(0, Math.min(100, result.progress))} size='small' showText />
      ) : (
        <span>正在等待模型返回结果</span>
      )}
      {onCancelTask ? (
        <Button size='small' status='danger' onClick={() => onCancelTask(result.taskId)}>
          取消任务
        </Button>
      ) : null}
    </div>
  );
};

const ImageWorkbenchResults: React.FC<ImageWorkbenchResultsProps> = ({
  results,
  selectedResultIds,
  task,
  onSelectionChange,
  onDeleteResult,
  onDeleteSelected,
  onRetryResult,
  onLoadResult,
  onCancelTask,
  historyLoading,
  historyError,
  historyLoadingMore,
  historyHasMore,
  onLoadMoreResults,
}) => {
  const deletionEnabled = Boolean(onDeleteResult && onDeleteSelected);
  const deletableResults = results.filter((result) => result.deletable);
  const deletableIds = deletableResults.map((result) => result.id);
  const selectedDeletableIds = selectedResultIds.filter((id) => deletableIds.includes(id));
  const allSelected =
    deletableResults.length > 0 &&
    deletableResults.every((result) => selectedResultIds.includes(result.id));
  const stateLabel = taskStateLabel(task);
  const stateTone =
    task.state === 'failed' ? 'red' : task.state === 'canceled' ? 'gray' : 'arcoblue';

  return (
    <section className={styles.resultsPanel} data-image-workbench-results data-result-count={results.length}>
      <header className={styles.resultsHeader}>
        <div className={styles.resultsTitle}>
          <History />
          <h2>全部结果</h2>
          <Tag>已加载 {results.length}</Tag>
          {stateLabel ? <Tag color={stateTone}>{stateLabel}</Tag> : null}
        </div>
        {deletionEnabled ? <div className={styles.resultsActions}>
          <Button
            size='small'
            icon={allSelected ? <CloseSmall /> : <Check />}
            disabled={deletableResults.length === 0}
            onClick={() => onSelectionChange(allSelected ? [] : deletableIds)}
          >
            {allSelected ? '取消全选' : '全选'}
          </Button>
          <Button
            size='small'
            status='danger'
            icon={<Delete />}
            disabled={selectedDeletableIds.length === 0}
            onClick={() => onDeleteSelected?.(selectedDeletableIds)}
          >
            移除{selectedDeletableIds.length > 0 ? ` ${selectedDeletableIds.length}` : ''}
          </Button>
        </div> : null}
      </header>

      {results.length === 0 ? (
        <div className={styles.emptyResults} data-image-result-state='empty'>
          <span className={styles.emptyIcon}>{historyLoading ? <Loading size={38} className={styles.spin} /> : historyError ? <Error size={38} /> : <Pic size={38} />}</span>
          <strong>{historyLoading ? '正在恢复生成历史' : historyError ? '生成历史加载失败' : '还没有生成图片'}</strong>
          <p>{historyLoading ? '正在读取当前项目的真实任务与结果。' : historyError ?? '在创作台输入提示词并选择模型，生成结果会出现在这里。'}</p>
        </div>
      ) : (
        <div className={styles.resultGrid}>
          {results.map((result) => {
            const selected = Boolean(result.deletable && selectedResultIds.includes(result.id));
            return (
              <article
                key={result.id}
                className={styles.resultCard}
                data-image-result-state={result.status}
                data-provider-id={result.model.providerId}
                data-model={result.model.model}
                data-selected={selected || undefined}
              >
                {deletionEnabled && result.deletable ? <div className={styles.resultSelection}>
                  <Checkbox
                    checked={selected}
                    aria-label={`选择结果 ${result.id}`}
                    onChange={(checked) =>
                      onSelectionChange(
                        nextImageWorkbenchSelection(selectedResultIds, result.id, checked)
                      )
                    }
                  />
                  <Button
                    size='mini'
                    type='text'
                    status='danger'
                    icon={<Delete />}
                    aria-label={`从历史移除 ${result.id}`}
                    onClick={() => onDeleteResult?.(result.id)}
                  />
                </div> : null}
                <ResultVisual
                  result={result}
                  onRetryResult={onRetryResult}
                  onCancelTask={onCancelTask}
                />
                <TaskMeta result={result} onLoadResult={onLoadResult} />
              </article>
            );
          })}
        </div>
      )}
      {historyHasMore && onLoadMoreResults ? (
        <div className={styles.historyFooter}>
          <Button loading={historyLoadingMore} onClick={onLoadMoreResults}>
            {historyLoadingMore ? '正在加载…' : '加载更多历史'}
          </Button>
        </div>
      ) : null}
    </section>
  );
};

export default ImageWorkbenchResults;
