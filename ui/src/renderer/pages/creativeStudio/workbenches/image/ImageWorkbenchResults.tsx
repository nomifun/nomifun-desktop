/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  Check,
  CloseSmall,
  Delete,
  Error,
  History,
  Loading,
  Pic,
  Refresh,
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
  onDeleteResult(resultId: string): void;
  onDeleteSelected(resultIds: string[]): void;
  onRetryResult?(resultId: string): void;
}

const TaskMeta: React.FC<{ result: ImageWorkbenchResult }> = ({ result }) => (
  <div className={styles.resultMeta}>
    <p title={result.prompt}>{result.prompt}</p>
    <div>
      <Tag>{result.modelLabel}</Tag>
      {result.createdAtLabel ? <Tag>{result.createdAtLabel}</Tag> : null}
      {result.durationLabel ? <Tag>{result.durationLabel}</Tag> : null}
    </div>
  </div>
);

const ResultVisual: React.FC<{
  result: ImageWorkbenchResult;
  onRetryResult?(resultId: string): void;
}> = ({ result, onRetryResult }) => {
  if (result.status === 'succeeded') {
    return (
      <div className={styles.successVisual}>
        <img src={result.imageUrl} alt={result.alt} />
        <span className={styles.resultBadge} data-tone='success'>
          <Check /> 成功
        </span>
        {result.width && result.height ? (
          <span className={styles.imageDimensions}>
            {result.width} × {result.height}
            {result.sizeLabel ? ` · ${result.sizeLabel}` : ''}
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
        {onRetryResult ? (
          <Button size='small' status='danger' icon={<Refresh />} onClick={() => onRetryResult(result.id)}>
            重试
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
}) => {
  const allSelected = results.length > 0 && results.every((result) => selectedResultIds.includes(result.id));
  const stateLabel =
    task.state === 'running'
      ? `${task.pendingCount} 个生成中`
      : task.state === 'failed'
        ? '最近任务失败'
        : task.state === 'succeeded'
          ? '任务已完成'
          : null;

  return (
    <section className={styles.resultsPanel} data-image-workbench-results data-result-count={results.length}>
      <header className={styles.resultsHeader}>
        <div className={styles.resultsTitle}>
          <History />
          <h2>全部结果</h2>
          <Tag>{results.length}</Tag>
          {stateLabel ? <Tag color={task.state === 'failed' ? 'red' : 'arcoblue'}>{stateLabel}</Tag> : null}
        </div>
        <div className={styles.resultsActions}>
          <Button
            size='small'
            icon={allSelected ? <CloseSmall /> : <Check />}
            disabled={results.length === 0}
            onClick={() => onSelectionChange(allSelected ? [] : results.map((result) => result.id))}
          >
            {allSelected ? '取消全选' : '全选'}
          </Button>
          <Button
            size='small'
            status='danger'
            icon={<Delete />}
            disabled={selectedResultIds.length === 0}
            onClick={() => onDeleteSelected([...selectedResultIds])}
          >
            删除{selectedResultIds.length > 0 ? ` ${selectedResultIds.length}` : ''}
          </Button>
        </div>
      </header>

      {results.length === 0 ? (
        <div className={styles.emptyResults} data-image-result-state='empty'>
          <span className={styles.emptyIcon}>
            <Pic size={38} />
          </span>
          <strong>还没有生成图片</strong>
          <p>在创作台输入提示词并选择模型，生成结果会出现在这里。</p>
        </div>
      ) : (
        <div className={styles.resultGrid}>
          {results.map((result) => {
            const selected = selectedResultIds.includes(result.id);
            return (
              <article
                key={result.id}
                className={styles.resultCard}
                data-image-result-state={result.status}
                data-selected={selected || undefined}
              >
                <div className={styles.resultSelection}>
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
                    aria-label={`删除结果 ${result.id}`}
                    onClick={() => onDeleteResult(result.id)}
                  />
                </div>
                <ResultVisual result={result} onRetryResult={onRetryResult} />
                <TaskMeta result={result} />
              </article>
            );
          })}
        </div>
      )}
    </section>
  );
};

export default ImageWorkbenchResults;
