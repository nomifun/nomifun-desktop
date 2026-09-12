/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * AutoWorkPanel — the admin/overview body of the "自动执行" (AutoWork) panel under
 * the requirements platform's ExtensionsPage. A direct port of the legacy
 * `autowork/TagSessionTab`, showing which sessions are bound to which tags for
 * automatic requirement execution.
 *
 * It loads tags + their session bindings and renders one row per tag with:
 * done/total counts, a bound-session count (with a >1-active conflict warning),
 * an expandable per-binding list (run-state dot + state tag + Unbind), and a
 * paused badge + Resume action.
 *
 * REMOVED relative to TagSessionTab: the per-tag webhook `Select` column and its
 * `handleWebhookChange` / `ipcBridge.webhook.*` data loading. Webhook (notify)
 * binding now lives in the separate NotifyPanel / RoutingRuleList, so this panel
 * no longer touches webhooks, tag settings, or the `autowork.tagSessions.webhook*`
 * i18n keys.
 *
 * This component is the panel body only — no page header and no list/kanban nav
 * buttons (those live in the ExtensionsPage / RequirementsLayout shell).
 *
 * Messages go through `useArcoMessage` (render `{ctx}`); clickable affordances
 * are Arco `Button`s; theme tokens only.
 */
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, Table, Tag, Tooltip } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import { isHandledAuthExpiredHttpError } from '@/common/adapter/httpBridge';
import type { ITagBinding, ITagBindings, ITagSummary } from '@/common/adapter/ipcBridge';
import { shortSessionId } from '@renderer/utils/ui/shortId';
import { useArcoMessage } from '@renderer/utils/ui/useArcoMessage';

type TagRowData = Pick<ITagSummary, 'tag' | 'done' | 'total'> & {
  paused?: boolean;
  paused_reason?: string | null;
  bindings: ITagBindings['bindings'];
};

const AutoWorkPanel: React.FC = () => {
  const { t } = useTranslation();
  const [message, ctx] = useArcoMessage();
  const [tags, setTags] = useState<ITagSummary[]>([]);
  const [bindings, setBindings] = useState<ITagBindings[]>([]);
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState(false);
  const [acting, setActing] = useState(false);
  const mounted = useRef(false);
  const latestRequest = useRef(0);
  const actionPending = useRef(false);

  const loadData = useCallback(async () => {
    if (!mounted.current) return;
    const request = ++latestRequest.current;
    setLoading(true);
    setLoadError(false);
    try {
      // Tags + bindings are the whole of this panel now that the webhook picker
      // has moved out — load them together.
      const [tagList, bindingList] = await Promise.all([
        ipcBridge.requirements.tags.invoke(),
        ipcBridge.requirements.tagBindings.invoke(),
      ]);
      if (request !== latestRequest.current) return;
      setTags(tagList);
      setBindings(bindingList);
    } catch (e) {
      if (request !== latestRequest.current) return;
      setLoadError(true);
      if (isHandledAuthExpiredHttpError(e)) return;
      message.error(String(e));
    } finally {
      if (request === latestRequest.current) setLoading(false);
    }
  }, [message]);

  useEffect(() => {
    mounted.current = true;
    const unsubs = [
      ipcBridge.requirements.onTagPaused.on(() => void loadData()),
      ipcBridge.requirements.onAutoWork.on(() => void loadData()),
      ipcBridge.requirements.onCreated.on(() => void loadData()),
      ipcBridge.requirements.onUpdated.on(() => void loadData()),
      ipcBridge.requirements.onStatusChanged.on(() => void loadData()),
      ipcBridge.requirements.onDeleted.on(() => void loadData()),
      ipcBridge.conversation.reconnected.on(() => void loadData()),
    ];
    void loadData();
    return () => {
      mounted.current = false;
      latestRequest.current++;
      unsubs.forEach((u) => u());
    };
  }, [loadData]);

  const runAction = async (action: () => Promise<unknown>, successKey: string, errorKey?: string) => {
    if (!mounted.current || actionPending.current) return;
    actionPending.current = true;
    setActing(true);
    try {
      await action();
      if (!mounted.current) return;
      message.success(t(successKey));
      void loadData();
    } catch (e) {
      if (!mounted.current || isHandledAuthExpiredHttpError(e)) return;
      message.error(errorKey ? t(errorKey, { error: String(e) }) : String(e));
    } finally {
      actionPending.current = false;
      if (mounted.current) setActing(false);
    }
  };

  // Preserve the binding's typed target and the backend's active-session guard.
  const handleUnbind = (binding: ITagBinding) => runAction(
    () => ipcBridge.requirements.setAutoWork.invoke({
      kind: binding.kind, target_id: binding.target_id, enabled: false, from_admin: true,
    }),
    'autowork.tagSessions.unbindOk', 'autowork.tagSessions.unbindError'
  );
  const handleResume = (tag: string) => runAction(
    () => ipcBridge.requirements.resumeTag.invoke({ tag, requeue_failed: true }),
    'autowork.tagSessions.resumeSuccess'
  );

  const bindingsByTag = new Map(bindings.map((group) => [group.tag, group.bindings]));
  const tableData: TagRowData[] = tags.map((tg) => ({ ...tg, bindings: bindingsByTag.get(tg.tag) ?? [] }));
  const knownTags = new Set(tags.map((tg) => tg.tag));
  // The tags endpoint omits tags without requirements. Keep their bindings
  // manageable; that endpoint supplies no pause metadata for these rows.
  for (const group of bindings) {
    if (!knownTags.has(group.tag)) tableData.push({ tag: group.tag, done: 0, total: 0, bindings: group.bindings });
  }

  const runStateColor = (state: string): string => {
    switch (state) {
      case 'active':
        return 'rgb(var(--success-6))';
      case 'idle':
        return 'rgb(var(--warning-6))';
      default:
        return 'rgb(var(--gray-4))';
    }
  };

  const pausedReasonLabel = (reason?: string | null): string => {
    switch (reason) {
      case 'requirement_failed':
        return t('autowork.tagSessions.pausedReasons.requirementFailed');
      case 'user_interrupted':
        return t('autowork.tagSessions.pausedReasons.userInterrupted');
      default:
        return reason ?? '';
    }
  };

  // Conversation and terminal bindings use stable UUIDs. Keep rows scannable
  // with the shared UUID suffix while retaining the full id on hover.
  const bindingIdLabel = (binding: ITagBinding): string => shortSessionId(binding.target_id);

  const columns = [
    {
      title: t('autowork.tagSessions.tag'),
      dataIndex: 'tag',
      width: 240,
      render: (v: string, row: TagRowData) => (
        <div className='flex flex-wrap items-center gap-6px'>
          <Tag>{v}</Tag>
          {row.paused ? (
            <>
              <Tag size='small' color='red'>
                {t('autowork.tagSessions.pausedBadge', {
                  reason: pausedReasonLabel(row.paused_reason),
                })}
              </Tag>
              <Button size='mini' type='primary' disabled={acting} onClick={() => void handleResume(row.tag)}>
                {t('autowork.tagSessions.resume')}
              </Button>
            </>
          ) : null}
        </div>
      ),
    },
    {
      title: t('autowork.tagSessions.counts'),
      width: 120,
      render: (_: unknown, row: TagRowData) => (
        <span className='text-t-secondary text-12px'>
          {t('autowork.tagSessions.countsFmt', { done: row.done, total: row.total })}
        </span>
      ),
    },
    {
      title: t('autowork.tagSessions.boundCount'),
      width: 120,
      render: (_: unknown, row: TagRowData) => {
        const activeCount = row.bindings.filter((b) => b.run_state === 'active').length;
        return (
          <div className='flex items-center gap-4px'>
            <span className='text-t-primary'>{row.bindings.length}</span>
            {activeCount > 1 && (
              <Tag size='small' color='orangered'>
                {t('autowork.tagSessions.conflictWarning')}
              </Tag>
            )}
          </div>
        );
      },
    },
  ];

  const expandedRowRender = (row: TagRowData) => {
    if (row.bindings.length === 0) {
      return <span className='text-t-tertiary text-12px'>{t('autowork.tagSessions.noBindings')}</span>;
    }

    return (
      <div className='flex flex-col gap-8px py-4px'>
        {row.bindings.map((binding) => {
          const isActive = binding.run_state === 'active';
          return (
            <div key={`${binding.kind}-${binding.target_id}`} className='flex items-center gap-12px'>
              {/* Run state indicator dot */}
              <span
                className='inline-block w-8px h-8px rd-full shrink-0'
                style={{ backgroundColor: runStateColor(binding.run_state) }}
              />
              {/* Name + compact stable id label. */}
              <div className='flex flex-col min-w-0'>
                <span className='text-t-primary text-13px truncate'>{binding.name}</span>
                <span className='text-t-tertiary text-11px' title={String(binding.target_id)}>
                  {bindingIdLabel(binding)}
                </span>
              </div>
              {/* Run state text */}
              <Tag size='small' color={isActive ? 'green' : binding.run_state === 'idle' ? 'orange' : 'gray'}>
                {t(`autowork.runState.${binding.run_state}`)}
              </Tag>
              {/* Unbind button */}
              <Tooltip
                content={isActive ? t('autowork.tagSessions.activeStopInSession') : undefined}
                disabled={!isActive}
              >
                <Button
                  size='mini'
                  status='warning'
                  disabled={isActive || acting}
                  onClick={() => void handleUnbind(binding)}
                >
                  {t('autowork.tagSessions.unbind')}
                </Button>
              </Tooltip>
            </div>
          );
        })}
      </div>
    );
  };

  return (
    <>
      {ctx}
      {loadError && (
        <div className='mb-12px text-t-secondary' role='alert'>
          {t('requirements.loadError')}
          <Button className='ml-8px' onClick={() => void loadData()}>{t('requirements.retry')}</Button>
        </div>
      )}
      <Table
        rowKey='tag'
        loading={loading}
        columns={columns}
        data={tableData}
        border={{ wrapper: true, cell: false }}
        pagination={false}
        expandedRowRender={expandedRowRender}
        noDataElement={<span className='text-t-tertiary'>{t('autowork.tagSessions.empty')}</span>}
      />
    </>
  );
};

export default AutoWorkPanel;
