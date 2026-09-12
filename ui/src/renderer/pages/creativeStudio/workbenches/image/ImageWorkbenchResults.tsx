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
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import CopyIconButton from '@/renderer/components/base/CopyIconButton';
import {
  CreativeDetailFacts,
  CreativeDetailLayout,
  CreativeDetailModal,
  CreativeDetailSection,
  CreativeDetailText,
} from '../../components/CreativeDetailLayout';
import { CreativeAssetUnavailable } from '../../assets/components/CreativeAssetUnavailable';
import {
  nextImageWorkbenchSelection,
  type ImageWorkbenchResult,
  type ImageWorkbenchTaskSummary,
} from './types';
import styles from './ImageWorkbench.module.css';

const RESULT_CARD_MIN_WIDTH = 184;
const RESULT_CARD_GAP = 6;

export const imageWorkbenchResultColumnCount = (availableWidth: number): number => {
  if (!Number.isFinite(availableWidth) || availableWidth <= 0) return 1;
  return Math.max(
    1,
    Math.floor((availableWidth + RESULT_CARD_GAP) / (RESULT_CARD_MIN_WIDTH + RESULT_CARD_GAP))
  );
};

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

const ResultVisual: React.FC<{
  result: ImageWorkbenchResult;
  onRetryResult?(resultId: string): void;
  onCancelTask?(taskId: string): void;
}> = ({ result, onRetryResult, onCancelTask }) => {
  const { t } = useTranslation();
  if (result.status === 'succeeded') {
    return (
      <div className={styles.successVisual}>
        <div className={styles.successGallery} data-image-output-count={result.outputs.length}>
          {result.outputs.length === 0 ? (
            <div className={styles.unavailableOutput}>
              <CreativeAssetUnavailable status='unavailable' />
            </div>
          ) : null}
          {result.outputs.map((output) => (
            output.availability && output.availability !== 'available'
              ? (
                <div
                  key={output.assetId}
                  className={styles.unavailableOutput}
                  style={output.width && output.height
                    ? { aspectRatio: `${output.width} / ${output.height}` }
                    : undefined}
                >
                  <CreativeAssetUnavailable status={output.availability} />
                </div>
              )
              : (
                <img
                  key={output.assetId}
                  src={output.imageUrl}
                  alt={output.alt}
                  className={styles.resultMedia}
                  width={output.width}
                  height={output.height}
                  loading='lazy'
                  draggable={false}
                />
              )
          ))}
        </div>
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

const resultStatusLabel = (
  t: ReturnType<typeof useTranslation>['t'],
  result: ImageWorkbenchResult
): string => {
  switch (result.status) {
    case 'queued':
      return result.statusLabel ?? t('creativeStudio.image.task.queued', { defaultValue: '排队中' });
    case 'running':
      return result.statusLabel ?? t('creativeStudio.image.task.running', { defaultValue: '生成中' });
    case 'succeeded':
      return t('creativeStudio.image.results.detailSucceeded', { defaultValue: '生成成功' });
    case 'failed':
      return t('creativeStudio.image.results.generationFailed', { defaultValue: '生成失败' });
    case 'canceled':
      return t('creativeStudio.image.results.canceled', { defaultValue: '已取消' });
  }
};

const ResultDetailsVisual: React.FC<{ result: ImageWorkbenchResult }> = ({ result }) => {
  const { t } = useTranslation();
  if (result.status === 'succeeded' && result.outputs.length > 0) {
    return (
      <div className={styles.detailGallery}>
        {result.outputs.map((output) => (
          output.availability && output.availability !== 'available'
            ? (
              <div
                key={output.assetId}
                className={styles.detailUnavailable}
                style={output.width && output.height
                  ? { aspectRatio: `${output.width} / ${output.height}` }
                  : undefined}
              >
                <CreativeAssetUnavailable status={output.availability} />
              </div>
            )
            : (
              <img
                key={output.assetId}
                src={output.imageUrl}
                alt={output.alt}
                width={output.width}
                height={output.height}
                loading='eager'
                draggable={false}
              />
            )
        ))}
      </div>
    );
  }

  const icon = result.status === 'failed'
    ? <Error size={38} />
    : result.status === 'canceled'
      ? <CloseOne size={38} />
      : result.status === 'queued'
        ? <Time size={36} />
        : result.status === 'running'
          ? <Loading size={36} className={styles.spin} />
          : <Pic size={38} />;

  return (
    <div className={styles.detailStateVisual} data-result-detail-state={result.status}>
      {icon}
      <strong>{resultStatusLabel(t, result)}</strong>
    </div>
  );
};

export const ImageResultDetails: React.FC<{ result: ImageWorkbenchResult }> = ({ result }) => {
  const { t } = useTranslation();
  const status = resultStatusLabel(t, result);
  return (
    <CreativeDetailLayout
      data-image-result-details={result.status}
      visual={<ResultDetailsVisual result={result} />}
      badges={(
        <>
          <Tag color={result.status === 'failed' ? 'red' : result.status === 'succeeded' ? 'green' : 'gray'}>
            {status}
          </Tag>
          <Tag>{result.modelLabel}</Tag>
        </>
      )}
    >
      <CreativeDetailSection
        label={t('creativeStudio.image.results.detailPrompt', { defaultValue: '完整提示词' })}
        action={result.prompt ? (
          <CopyIconButton
            text={result.prompt}
            tooltip={t('creativeStudio.image.results.copyPrompt', {
              defaultValue: '复制提示词',
            })}
            successMessage={t('creativeStudio.image.results.promptCopied', {
              defaultValue: '提示词已复制',
            })}
            size={14}
          />
        ) : null}
      >
        <CreativeDetailText>{result.prompt || '—'}</CreativeDetailText>
      </CreativeDetailSection>

      <CreativeDetailFacts>
        <div>
          <dt>{t('creativeStudio.image.results.detailStatus', { defaultValue: '状态' })}</dt>
          <dd>{status}</dd>
        </div>
        <div>
          <dt>{t('creativeStudio.image.results.detailProvider', { defaultValue: '提供商' })}</dt>
          <dd>{result.model.providerId}</dd>
        </div>
        <div>
          <dt>{t('creativeStudio.image.results.detailModel', { defaultValue: '模型' })}</dt>
          <dd>{result.model.model}</dd>
        </div>
        {result.createdAtLabel ? (
          <div>
            <dt>{t('creativeStudio.image.results.detailCreatedAt', { defaultValue: '生成时间' })}</dt>
            <dd>{result.createdAtLabel}</dd>
          </div>
        ) : null}
        {result.durationLabel ? (
          <div>
            <dt>{t('creativeStudio.image.results.detailDuration', { defaultValue: '耗时' })}</dt>
            <dd>{result.durationLabel}</dd>
          </div>
        ) : null}
        {result.status === 'succeeded' ? (
          <div>
            <dt>{t('creativeStudio.image.results.detailOutputCount', { defaultValue: '作品数量' })}</dt>
            <dd>{result.outputs.length}</dd>
          </div>
        ) : null}
      </CreativeDetailFacts>

      {result.status === 'succeeded' && result.outputs.length > 0 ? (
        <CreativeDetailSection label={t('creativeStudio.image.results.detailFiles', { defaultValue: '图片信息' })}>
          <div className={styles.detailFiles}>
            {result.outputs.map((output, index) => (
              <div key={output.assetId}>
                <span>
                  {t('creativeStudio.image.results.detailImageIndex', {
                    defaultValue: '图片 {{index}}',
                    index: index + 1,
                  })}
                </span>
                <strong>
                  {output.width && output.height ? `${output.width} × ${output.height}` : '—'}
                  {output.sizeLabel ? ` · ${output.sizeLabel}` : ''}
                </strong>
              </div>
            ))}
          </div>
        </CreativeDetailSection>
      ) : null}

      {result.hasDeletedInputs ? (
        <p className={styles.detailNotice} role='status'>
          {t('creativeStudio.assets.deletedReference', { defaultValue: '引用素材已删除，请重新选择后再生成。' })}
        </p>
      ) : null}

      {result.status === 'failed' ? (
        <CreativeDetailSection label={t('creativeStudio.image.results.detailError', { defaultValue: '错误详情' })}>
          <CreativeDetailText error>
            {result.errorDetail
              ? `${result.errorMessage}\n${result.errorDetail}`
              : result.errorMessage}
          </CreativeDetailText>
        </CreativeDetailSection>
      ) : null}
    </CreativeDetailLayout>
  );
};

const estimatedResultHeight = (result: ImageWorkbenchResult): number => {
  if (result.status !== 'succeeded' || result.outputs.length === 0) {
    return RESULT_CARD_MIN_WIDTH * 0.86;
  }
  return result.outputs.reduce((height, output) => {
    const ratio = output.width && output.height ? output.height / output.width : 1;
    return height + RESULT_CARD_MIN_WIDTH * ratio;
  }, Math.max(0, result.outputs.length - 1) * 2);
};

const distributeResults = (
  results: readonly ImageWorkbenchResult[],
  columnCount: number
): ImageWorkbenchResult[][] => {
  const columns = Array.from({ length: Math.max(1, columnCount) }, () => [] as ImageWorkbenchResult[]);
  const heights = columns.map(() => 0);
  for (const result of results) {
    const columnIndex = heights.indexOf(Math.min(...heights));
    columns[columnIndex].push(result);
    heights[columnIndex] += estimatedResultHeight(result) + RESULT_CARD_GAP;
  }
  return columns;
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
  const masonryRef = useRef<HTMLDivElement | null>(null);
  const [columnCount, setColumnCount] = useState(1);
  const [openedResultId, setOpenedResultId] = useState<string | null>(null);
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
  const openedResult = results.find((result) => result.id === openedResultId) ?? null;
  const columns = useMemo(
    () => distributeResults(results, columnCount),
    [columnCount, results]
  );

  useEffect(() => {
    const container = masonryRef.current;
    if (!container) return undefined;
    const updateColumnCount = (): void => {
      const availableWidth = container.getBoundingClientRect().width;
      const nextCount = imageWorkbenchResultColumnCount(availableWidth);
      setColumnCount((current) => current === nextCount ? current : nextCount);
    };
    updateColumnCount();
    if (typeof ResizeObserver === 'undefined') {
      window.addEventListener('resize', updateColumnCount);
      return () => window.removeEventListener('resize', updateColumnCount);
    }
    const observer = new ResizeObserver(updateColumnCount);
    observer.observe(container);
    return () => observer.disconnect();
  }, []);

  return (
    <section className={styles.resultsPanel} data-image-workbench-results data-result-count={results.length}>
      <header className={styles.resultsHeader}>
        <div className={styles.resultsTitle}>
          <History size={15} />
          <h2>{t('creativeStudio.image.results.title', { defaultValue: '全部结果' })}</h2>
          <Tag size='small' bordered={false}>
            {t('creativeStudio.image.results.loadedCount', {
              defaultValue: '已加载 {{resultCount}}',
              resultCount: results.length,
            })}
          </Tag>
          {stateLabel ? (
            <Tag size='small' bordered={false} color={stateTone}>
              {stateLabel}
            </Tag>
          ) : null}
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
          <div
            ref={masonryRef}
            className={styles.resultMasonry}
            data-masonry-columns={columnCount}
          >
            {columns.map((column, columnIndex) => (
              <div className={styles.resultColumn} key={columnIndex}>
                {column.map((result) => {
                  const selected = Boolean(result.deletable && selectedResultIds.includes(result.id));
                  const openDetails = (): void => setOpenedResultId(result.id);
                  return (
                    <article
                      key={result.id}
                      className={styles.resultCard}
                      data-image-result-state={result.status}
                      data-provider-id={result.model.providerId}
                      data-model={result.model.model}
                      data-selected={selected || undefined}
                      tabIndex={0}
                      aria-label={t('creativeStudio.image.results.openDetails', {
                        defaultValue: '查看作品详情',
                      })}
                      onClick={(event) => {
                        const target = event.target as HTMLElement;
                        if (target.closest('button, label, input, [role="checkbox"]')) return;
                        openDetails();
                      }}
                      onKeyDown={(event) => {
                        if (event.target !== event.currentTarget) return;
                        if (event.key === 'Enter' || event.key === ' ') {
                          event.preventDefault();
                          openDetails();
                        }
                      }}
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
                    </article>
                  );
                })}
              </div>
            ))}
          </div>
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
      <CreativeDetailModal
        visible={openedResult !== null}
        title={t('creativeStudio.image.results.detailTitle', { defaultValue: '作品详情' })}
        onClose={() => setOpenedResultId(null)}
      >
        {openedResult ? <ImageResultDetails result={openedResult} /> : null}
      </CreativeDetailModal>
    </section>
  );
};

export default ImageWorkbenchResults;
