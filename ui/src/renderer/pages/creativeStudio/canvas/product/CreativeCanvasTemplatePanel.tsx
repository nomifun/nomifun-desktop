/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  CheckOne,
  Error,
  Loading,
  Pic,
  Play,
  Refresh,
  Right,
  Workbench,
} from '@icon-park/react';
import classNames from 'classnames';
import React, { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type {
  CreativeTemplateDefinitionV1,
  CreativeTemplateRunAggregateV1,
} from '../../templates/domain';
import type { CreativeTemplateRuntimeSnapshot } from '../../templates/runtime';
import styles from './CreativeCanvasTemplatePanel.module.css';

export interface CreativeCanvasTemplatePanelProps {
  templates: readonly CreativeTemplateDefinitionV1[];
  runtime: CreativeTemplateRuntimeSnapshot;
  loading: boolean;
  error: string | null;
  disabled?: boolean;
  insertingRunId?: string | null;
  onRetry(): void;
  onRun(template: CreativeTemplateDefinitionV1): void;
  onInsertResults(run: CreativeTemplateRunAggregateV1): void;
  onOpenCenter(): void;
  className?: string;
}

const iconProps = {
  theme: 'outline' as const,
  size: 15,
  fill: 'currentColor',
  strokeWidth: 2.5,
};

const ACTIVE_STATUSES = new Set<CreativeTemplateRunAggregateV1['record']['status']>([
  'requested',
  'queued',
  'running',
  'awaiting-review',
]);

const RUN_STATUS_KEYS: Record<
  CreativeTemplateRunAggregateV1['record']['status'],
  string
> = {
  requested: 'creativeStudio.canvas.templates.status.requested',
  queued: 'creativeStudio.canvas.templates.status.queued',
  running: 'creativeStudio.canvas.templates.status.running',
  'awaiting-review': 'creativeStudio.canvas.templates.status.awaitingReview',
  succeeded: 'creativeStudio.canvas.templates.status.succeeded',
  failed: 'creativeStudio.canvas.templates.status.failed',
  cancelled: 'creativeStudio.canvas.templates.status.cancelled',
};

const RUN_STATUS_FALLBACKS: Record<
  CreativeTemplateRunAggregateV1['record']['status'],
  string
> = {
  requested: '准备中',
  queued: '已排队',
  running: '运行中',
  'awaiting-review': '待审核',
  succeeded: '已完成',
  failed: '失败',
  cancelled: '已取消',
};

function latestRunByTemplate(
  runs: readonly CreativeTemplateRunAggregateV1[]
): ReadonlyMap<string, CreativeTemplateRunAggregateV1> {
  const latest = new Map<string, CreativeTemplateRunAggregateV1>();
  for (const run of runs) {
    const templateId = run.templateSnapshot.id;
    const current = latest.get(templateId);
    if (!current || run.request.requestedAt > current.request.requestedAt) {
      latest.set(templateId, run);
    }
  }
  return latest;
}

function CreativeTemplateRunStatus({ run }: { run: CreativeTemplateRunAggregateV1 }) {
  const { t } = useTranslation();
  const status = run.record.status;
  const icon = status === 'succeeded'
    ? <CheckOne {...iconProps} />
    : status === 'failed' || status === 'cancelled'
      ? <Error {...iconProps} />
      : <Loading className={styles.spinning} {...iconProps} />;
  return (
    <span className={styles.runStatus} data-status={status}>
      {icon}
      {t(RUN_STATUS_KEYS[status], {
        defaultValue: RUN_STATUS_FALLBACKS[status],
      })}
    </span>
  );
}

/** Compact canvas projection over the canonical global template repository and run store. */
const CreativeCanvasTemplatePanel: React.FC<CreativeCanvasTemplatePanelProps> = ({
  templates,
  runtime,
  loading,
  error,
  disabled = false,
  insertingRunId = null,
  onRetry,
  onRun,
  onInsertResults,
  onOpenCenter,
  className,
}) => {
  const { t } = useTranslation();
  const [search, setSearch] = useState('');
  const runsByTemplate = useMemo(() => latestRunByTemplate(runtime.runs), [runtime.runs]);
  const filtered = useMemo(() => {
    const query = search.trim().toLocaleLowerCase();
    if (!query) return templates;
    return templates.filter((template) => [
      template.metadata.name,
      template.metadata.description,
      template.metadata.category,
      ...template.metadata.tags,
    ].join('\n').toLocaleLowerCase().includes(query));
  }, [search, templates]);
  const activeCount = runtime.runs.filter((run) => ACTIVE_STATUSES.has(run.record.status)).length;

  return (
    <section
      className={classNames(styles.panel, className)}
      data-canvas-product-panel='templates'
      aria-label={t('creativeStudio.canvas.templates.label', {
        defaultValue: '画布模板',
      })}
    >
      <header className={styles.header}>
        <div>
          <h2>
            {t('creativeStudio.canvas.templates.title', {
              defaultValue: '模板',
            })}
          </h2>
          <p>
            {t('creativeStudio.canvas.templates.summary', {
              count: templates.length,
              defaultValue: `${templates.length} 个模板`,
            })}
            {activeCount > 0
              ? t('creativeStudio.canvas.templates.activeSummary', {
                  count: activeCount,
                  defaultValue: ` · ${activeCount} 个进行中`,
                })
              : ''}
          </p>
        </div>
        <button
          type='button'
          className={styles.iconButton}
          aria-label={t('creativeStudio.canvas.templates.openCenter', {
            defaultValue: '打开模板工作台',
          })}
          onClick={onOpenCenter}
        >
          <Right {...iconProps} />
        </button>
      </header>

      <div className={styles.searchRow}>
        <input
          type='search'
          value={search}
          placeholder={t('creativeStudio.canvas.templates.searchPlaceholder', {
            defaultValue: '搜索模板',
          })}
          aria-label={t('creativeStudio.canvas.templates.searchLabel', {
            defaultValue: '搜索模板',
          })}
          onChange={(event) => setSearch(event.currentTarget.value)}
        />
        <button
          type='button'
          className={styles.iconButton}
          aria-label={t('creativeStudio.canvas.templates.refresh', {
            defaultValue: '刷新模板',
          })}
          disabled={loading}
          onClick={onRetry}
        >
          <Refresh className={loading ? styles.spinning : undefined} {...iconProps} />
        </button>
      </div>

      {error || runtime.loadError ? (
        <div className={styles.error} role='alert'>
          <Error {...iconProps} />
          <span>{error ?? runtime.loadError}</span>
          <button type='button' onClick={onRetry}>
            {t('creativeStudio.canvas.templates.retry', {
              defaultValue: '重试',
            })}
          </button>
        </div>
      ) : loading && templates.length === 0 ? (
        <div className={styles.state} role='status'>
          <Loading className={styles.spinning} {...iconProps} />
          {t('creativeStudio.canvas.templates.loading', {
            defaultValue: '正在载入模板…',
          })}
        </div>
      ) : filtered.length === 0 ? (
        <div className={styles.state} role='status'>
          <Workbench {...iconProps} />
          <strong>
            {templates.length === 0
              ? t('creativeStudio.canvas.templates.empty', {
                  defaultValue: '暂无模板',
                })
              : t('creativeStudio.canvas.templates.noMatches', {
                  defaultValue: '没有匹配的模板',
                })}
          </strong>
          <span>
            {t('creativeStudio.canvas.templates.emptyDescription', {
              defaultValue: '可前往模板工作台创建和配置模板。',
            })}
          </span>
          <button type='button' onClick={onOpenCenter}>
            {t('creativeStudio.canvas.templates.openCenter', {
              defaultValue: '打开模板工作台',
            })}
          </button>
        </div>
      ) : (
        <div
          className={styles.list}
          role='list'
          aria-label={t('creativeStudio.canvas.templates.runnableLabel', {
            defaultValue: '可运行模板',
          })}
        >
          {filtered.map((template) => {
            const run = runsByTemplate.get(template.id) ?? null;
            const active = run ? ACTIVE_STATUSES.has(run.record.status) : false;
            const resultCount = run?.record.resultAssetIds.length ?? 0;
            const inserting = run?.request.id === insertingRunId;
            return (
              <article key={template.id} className={styles.card} role='listitem'>
                <div className={styles.cardHeading}>
                  <div className={styles.templateIcon}><Workbench {...iconProps} /></div>
                  <div className={styles.identity}>
                    <strong title={template.metadata.name}>{template.metadata.name}</strong>
                    <span>
                      {template.metadata.category ||
                        t('creativeStudio.canvas.templates.uncategorized', {
                          defaultValue: '未分类',
                        })}{' '}
                      ·{' '}
                      {template.output.kind === 'multi-image-series'
                        ? t('creativeStudio.canvas.templates.series', {
                            count: template.output.targetCount,
                            defaultValue: '{{count}} 张系列',
                          })
                        : t('creativeStudio.canvas.templates.singleImage', {
                            defaultValue: '单图',
                          })}
                    </span>
                  </div>
                </div>
                {template.metadata.description ? (
                  <p className={styles.description}>{template.metadata.description}</p>
                ) : null}
                {run ? (
                  <div className={styles.runRow}>
                    <CreativeTemplateRunStatus run={run} />
                    {resultCount > 0 ? (
                      <span>
                        {t('creativeStudio.canvas.templates.resultCount', {
                          count: resultCount,
                          defaultValue: `${resultCount} 项真实结果`,
                        })}
                      </span>
                    ) : null}
                  </div>
                ) : null}
                <div className={styles.actions}>
                  <button
                    type='button'
                    disabled={disabled || active}
                    onClick={() => onRun(template)}
                  >
                    <Play {...iconProps} />
                    {active
                      ? t('creativeStudio.canvas.templates.runningAction', {
                          defaultValue: '正在运行',
                        })
                      : run
                        ? t('creativeStudio.canvas.templates.rerun', {
                            defaultValue: '再次运行',
                          })
                        : t('creativeStudio.canvas.templates.run', {
                            defaultValue: '运行',
                          })}
                  </button>
                  {run && run.record.status === 'succeeded' && resultCount > 0 ? (
                    <button
                      type='button'
                      disabled={disabled || inserting}
                      onClick={() => onInsertResults(run)}
                    >
                      {inserting ? <Loading className={styles.spinning} {...iconProps} /> : <Pic {...iconProps} />}
                      {inserting
                        ? t('creativeStudio.canvas.templates.inserting', {
                            defaultValue: '正在插入',
                          })
                        : t('creativeStudio.canvas.templates.insertResults', {
                            defaultValue: '插入结果',
                          })}
                    </button>
                  ) : run?.record.status === 'awaiting-review' ? (
                    <button type='button' onClick={onOpenCenter}>
                      {t('creativeStudio.canvas.templates.review', {
                        defaultValue: '去审核',
                      })}
                    </button>
                  ) : null}
                </div>
              </article>
            );
          })}
        </div>
      )}
    </section>
  );
};

export default CreativeCanvasTemplatePanel;
