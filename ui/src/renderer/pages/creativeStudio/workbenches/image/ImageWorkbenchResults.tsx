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
import { useTranslation } from 'react-i18next';
import CopyIconButton from '@/renderer/components/base/CopyIconButton';
import { CreativeAssetUnavailable } from '../../assets/components/CreativeAssetUnavailable';
import CreativeMediaPreview from '../../assets/components/CreativeMediaPreview';
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
  onCancelTask?(taskId: string): void;
  historyLoading?: boolean;
  historyError?: string;
  historyLoadingMore?: boolean;
  historyHasMore?: boolean;
  onLoadMoreResults?(): void;
}

const taskStateLabel = (
  t: ReturnType<typeof useTranslation>['t'],
  task: ImageWorkbenchTaskSummary
): string | null => {
  switch (task.state) {
    case 'queued':
      return t('creativeStudio.image.results.queuedCount', {
        defaultValue: '{{taskCount}} 个排队中',
        taskCount: task.pendingCount,
      });
    case 'running':
      return t('creativeStudio.image.results.runningCount', {
        defaultValue: '{{taskCount}} 个生成中',
        taskCount: task.pendingCount,
      });
    case 'succeeded':
      return t('creativeStudio.image.results.taskSucceeded', { defaultValue: '任务已完成' });
    case 'failed':
      return t('creativeStudio.image.results.taskFailed', { defaultValue: '最近任务失败' });
    case 'canceled':
      return t('creativeStudio.image.results.taskCanceled', { defaultValue: '最近任务已取消' });
    default:
      return null;
  }
};

const TaskMeta: React.FC<{
  result: ImageWorkbenchResult;
}> = ({ result }) => {
  const { t } = useTranslation();
  return (
    <div className={styles.resultMeta}>
      {result.hasDeletedInputs ? <p role='status'>{t('creativeStudio.assets.deletedReference', { defaultValue: '引用素材已删除，请重新选择后再生成。' })}</p> : null}
      <div className={styles.resultPromptRow}>
        <p className={styles.resultPrompt} title={result.prompt}>{result.prompt}</p>
        {result.prompt ? (
          <CopyIconButton
            text={result.prompt}
            tooltip={t('creativeStudio.image.results.copyPrompt', { defaultValue: '复制提示词' })}
            successMessage={t('creativeStudio.image.results.promptCopied', {
              defaultValue: '提示词已复制',
            })}
            size={13}
            className={styles.promptCopy}
          />
        ) : null}
      </div>
      <div className={styles.resultTags}>
        <Tag title={`${result.model.providerId}/${result.model.model}`}>{result.modelLabel}</Tag>
        {result.createdAtLabel ? <Tag>{result.createdAtLabel}</Tag> : null}
        {result.durationLabel ? <Tag>{result.durationLabel}</Tag> : null}
      </div>
    </div>
  );
};

const ResultVisual: React.FC<{
  result: ImageWorkbenchResult;
  onRetryResult?(resultId: string): void;
  onCancelTask?(taskId: string): void;
}> = ({ result, onRetryResult, onCancelTask }) => {
  const { t } = useTranslation();
  if (result.status === 'succeeded') {
    const first = result.outputs[0];
    return (
      <div className={styles.successVisual}>
        <div className={styles.successGallery} data-image-output-count={result.outputs.length}>
          {result.outputs.map((output) => (
            output.availability && output.availability !== 'available'
              ? <CreativeAssetUnavailable key={output.assetId} status={output.availability} />
              : <CreativeMediaPreview
                  key={output.assetId}
                  kind='image'
                  src={output.imageUrl}
                  alt={output.alt}
                  className={styles.resultMedia}
                />
          ))}
        </div>
        <span className={styles.resultBadge} data-tone='success'>
          <Check />{' '}
          {t('creativeStudio.image.results.successCount', {
            defaultValue: '成功 · {{imageCount}} 张',
            imageCount: result.outputs.length,
          })}
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
    const copyText = result.errorDetail
      ? `${result.errorMessage}\n${result.errorDetail}`
      : result.errorMessage;
    return (
      <div className={styles.failedVisual}>
        <Error size={30} />
        <strong>{t('creativeStudio.image.results.generationFailed', { defaultValue: '生成失败' })}</strong>
        <div className={styles.failureMessageRow}>
          <span className={styles.failureMessage} title={result.errorMessage}>
            {result.errorMessage}
          </span>
          <CopyIconButton
            text={copyText}
            tooltip={t('creativeStudio.image.results.copyFullError', {
              defaultValue: '复制完整报错信息',
            })}
            successMessage={t('creativeStudio.image.results.errorCopied', {
              defaultValue: '报错信息已复制',
            })}
            size={14}
            className={styles.failureCopy}
          />
        </div>
        {onRetryResult && result.retryable !== false ? (
          <Button size='small' status='danger' icon={<Refresh />} onClick={() => onRetryResult(result.id)}>
            {t('creativeStudio.image.actions.retry', { defaultValue: '重试' })}
          </Button>
        ) : null}
      </div>
    );
  }

  if (result.status === 'canceled') {
    return (
      <div className={styles.canceledVisual}>
        <CloseOne size={30} />
        <strong>{t('creativeStudio.image.results.canceled', { defaultValue: '已取消' })}</strong>
        <span>
          {result.message ||
            t('creativeStudio.image.results.canceledDescription', {
              defaultValue: '任务已取消，没有生成图片',
            })}
        </span>
        {onRetryResult && result.retryable !== false ? (
          <Button size='small' icon={<Refresh />} onClick={() => onRetryResult(result.id)}>
            {t('creativeStudio.image.actions.regenerate', { defaultValue: '重新生成' })}
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
        <strong>
          {result.statusLabel ||
            t('creativeStudio.image.task.queued', { defaultValue: '排队中' })}
        </strong>
        <span>
          {t('creativeStudio.image.results.waitingForModel', {
            defaultValue: '等待模型开始处理',
          })}
        </span>
        {onCancelTask ? (
          <Button size='small' status='danger' onClick={() => onCancelTask(result.taskId)}>
            {t('creativeStudio.image.actions.cancelTask', { defaultValue: '取消任务' })}
          </Button>
        ) : null}
      </div>
    );
  }

  return (
    <div className={styles.runningVisual}>
      <div className={styles.runningPattern} aria-hidden='true' />
      <Loading size={26} className={styles.spin} />
      <strong>
        {result.statusLabel ||
          t('creativeStudio.image.task.running', { defaultValue: '生成中' })}
      </strong>
      {result.progress !== undefined ? (
        <Progress percent={Math.max(0, Math.min(100, result.progress))} size='small' showText />
      ) : (
        <span>
          {t('creativeStudio.image.results.waitingForResult', {
            defaultValue: '正在等待模型返回结果',
          })}
        </span>
      )}
      {onCancelTask ? (
        <Button size='small' status='danger' onClick={() => onCancelTask(result.taskId)}>
          {t('creativeStudio.image.actions.cancelTask', { defaultValue: '取消任务' })}
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
  onCancelTask,
  historyLoading,
  historyError,
  historyLoadingMore,
  historyHasMore,
  onLoadMoreResults,
}) => {
  const { t } = useTranslation();
  const deletionEnabled = Boolean(onDeleteResult && onDeleteSelected);
  const deletableResults = results.filter((result) => result.deletable);
  const deletableIds = deletableResults.map((result) => result.id);
  const selectedDeletableIds = selectedResultIds.filter((id) => deletableIds.includes(id));
  const allSelected =
    deletableResults.length > 0 &&
    deletableResults.every((result) => selectedResultIds.includes(result.id));
  const stateLabel = taskStateLabel(t, task);
  const stateTone =
    task.state === 'failed' ? 'red' : task.state === 'canceled' ? 'gray' : 'arcoblue';

  return (
    <section className={styles.resultsPanel} data-image-workbench-results data-result-count={results.length}>
      <header className={styles.resultsHeader}>
        <div className={styles.resultsTitle}>
          <History />
          <h2>{t('creativeStudio.image.results.title', { defaultValue: '全部结果' })}</h2>
          <Tag>
            {t('creativeStudio.image.results.loadedCount', {
              defaultValue: '已加载 {{resultCount}}',
              resultCount: results.length,
            })}
          </Tag>
          {stateLabel ? <Tag color={stateTone}>{stateLabel}</Tag> : null}
        </div>
        {deletionEnabled ? <div className={styles.resultsActions}>
          <Button
            size='small'
            icon={allSelected ? <CloseSmall /> : <Check />}
            disabled={deletableResults.length === 0}
            onClick={() => onSelectionChange(allSelected ? [] : deletableIds)}
          >
            {allSelected
              ? t('creativeStudio.image.results.clearAll', { defaultValue: '取消全选' })
              : t('creativeStudio.image.results.selectAll', { defaultValue: '全选' })}
          </Button>
          <Button
            size='small'
            status='danger'
            icon={<Delete />}
            disabled={selectedDeletableIds.length === 0}
            onClick={() => onDeleteSelected?.(selectedDeletableIds)}
          >
            {t('creativeStudio.image.results.removeSelected', {
              defaultValue: '移除{{suffix}}',
              suffix:
                selectedDeletableIds.length > 0 ? ` ${selectedDeletableIds.length}` : '',
            })}
          </Button>
        </div> : null}
      </header>

      {results.length === 0 ? (
        <div className={styles.emptyResults} data-image-result-state='empty'>
          <span className={styles.emptyIcon}>{historyLoading ? <Loading size={38} className={styles.spin} /> : historyError ? <Error size={38} /> : <Pic size={38} />}</span>
          <strong>
            {historyLoading
              ? t('creativeStudio.image.results.historyLoading', {
                  defaultValue: '正在恢复生成历史',
                })
              : historyError
                ? t('creativeStudio.image.results.historyFailed', {
                    defaultValue: '生成历史加载失败',
                  })
                : t('creativeStudio.image.results.emptyTitle', {
                    defaultValue: '还没有生成图片',
                  })}
          </strong>
          <p>
            {historyLoading
              ? t('creativeStudio.image.results.historyLoadingDescription', {
                  defaultValue: '正在读取当前工作台的真实任务与结果。',
                })
              : historyError ??
                t('creativeStudio.image.results.emptyDescription', {
                  defaultValue: '在创作台输入提示词并选择模型，生成结果会出现在这里。',
                })}
          </p>
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
                    aria-label={t('creativeStudio.image.results.selectResult', {
                      defaultValue: '选择结果 {{id}}',
                      id: result.id,
                    })}
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
                    aria-label={t('creativeStudio.image.results.removeFromHistory', {
                      defaultValue: '从历史移除 {{id}}',
                      id: result.id,
                    })}
                    onClick={() => onDeleteResult?.(result.id)}
                  />
                </div> : null}
                <ResultVisual
                  result={result}
                  onRetryResult={onRetryResult}
                  onCancelTask={onCancelTask}
                />
                <TaskMeta result={result} />
              </article>
            );
          })}
        </div>
      )}
      {historyHasMore && onLoadMoreResults ? (
        <div className={styles.historyFooter}>
          <Button loading={historyLoadingMore} onClick={onLoadMoreResults}>
            {historyLoadingMore
              ? t('creativeStudio.image.results.loadingMore', { defaultValue: '正在加载…' })
              : t('creativeStudio.image.results.loadMore', { defaultValue: '加载更多历史' })}
          </Button>
        </div>
      ) : null}
    </section>
  );
};

export default ImageWorkbenchResults;
