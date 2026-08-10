/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * MiniAppsListPage (`/mini-apps`) — the solidified mini-app library.
 *
 * This is the *official* place a mini-app gets used, and the card grid has to say
 * so: every card carries a visible "open and use" control next to its metadata,
 * on top of the whole-card click, so nobody has to guess. Opening jumps to the
 * full-page runner at `/mini-apps/:id`, which loads the stored HTML straight from
 * the backend — nothing is regenerated, and no detour through the conversation
 * that built it is ever required.
 *
 * Rename and delete live here (hover actions) and on the runner; the read-only
 * right-side quick panel deliberately has neither. Creation does not live here
 * either: it starts as a conversation, so both the header CTA and the empty state
 * hand off to the start page with mini-app mode pre-armed.
 *
 * Design spec: docs/specs/2026-08-09-miniapps.zh.md
 */
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { Button, Result, Spin } from '@arco-design/web-react';
import { ApplicationOne, Delete, EditTwo, Plus, Right, Search } from '@icon-park/react';
import { ipcBridge } from '@/common';
import type { IApiMiniApp } from '@/common/adapter/ipcBridge';
import { useLayoutContext } from '@renderer/hooks/context/LayoutContext';
import { HUB_PAGE_TITLE_CLASS } from '@/renderer/components/layout/HubPageShell';
import { formatMiniAppRelativeTime } from './relativeTime';
import { useMiniAppMutations } from './useMiniAppMutations';

/** Start page with mini-app composer mode pre-armed (HashRouter query). */
const CREATE_ROUTE = '/guid?miniapp=1';

// ─── Mini-app card ────────────────────────────────────────────────────────────

/**
 * Always-visible primary affordance. The whole card is clickable too, but a bare
 * clickable card asks the user to guess: this library is where a mini-app gets
 * *used*, so the "use it" verb has to be on screen without hovering.
 */
const CARD_OPEN_BUTTON_CLASS = [
  'inline-flex shrink-0 items-center gap-3px h-24px px-9px rounded-8px cursor-pointer',
  'border border-solid border-[rgba(var(--primary-6),0.32)] bg-[rgba(var(--primary-6),0.10)]',
  'text-12px font-600 leading-none font-[inherit] text-primary-6',
  'transition-colors hover:bg-[rgba(var(--primary-6),0.18)]',
].join(' ');

interface MiniAppCardProps {
  app: IApiMiniApp;
  onOpen: (app: IApiMiniApp) => void;
  onRename: (app: IApiMiniApp) => void;
  onDelete: (app: IApiMiniApp) => void;
}

const MiniAppCard: React.FC<MiniAppCardProps> = ({ app, onOpen, onRename, onDelete }) => {
  const { t } = useTranslation();
  const icon = app.icon?.trim();

  return (
    <div
      className={[
        'group relative flex gap-12px overflow-hidden rounded-14px border border-solid p-14px',
        'border-[var(--color-border-2)] bg-[var(--color-bg-2)] box-border cursor-pointer',
        'transition-all duration-160',
        'hover:border-[var(--color-border-3)] hover:shadow-[0_12px_30px_rgba(0,0,0,0.12)] hover:-translate-y-2px',
      ].join(' ')}
      onClick={() => onOpen(app)}
    >
      {/* Icon tile — the stored emoji, or the domain glyph when none was given */}
      <span
        className='grid h-44px w-44px shrink-0 place-items-center rounded-12px bg-[var(--color-fill-2)] text-22px leading-none text-primary-6'
        aria-hidden='true'
      >
        {icon ? (
          icon
        ) : (
          <ApplicationOne theme='outline' size='22' fill='currentColor' className='block' style={{ lineHeight: 0 }} />
        )}
      </span>

      <div className='flex min-w-0 flex-1 flex-col gap-4px'>
        <div className='truncate text-15px font-600 leading-[1.35] text-[var(--color-text-1)]'>
          {app.name}
        </div>
        {app.description.trim() !== '' && (
          <div className='line-clamp-2 text-12px leading-17px text-[var(--color-text-3)]'>
            {app.description}
          </div>
        )}
        <div className='mt-auto flex items-center justify-between gap-8px pt-6px'>
          <span className='min-w-0 truncate text-11px leading-15px text-[var(--color-text-3)]'>
            {t('miniApps.list.updatedAt', { time: formatMiniAppRelativeTime(app.updated_at, t) })}
          </span>
          {/* The card's own click already opens it — this control must not fire twice. */}
          <button
            type='button'
            className={CARD_OPEN_BUTTON_CLASS}
            title={t('miniApps.actions.open')}
            onClick={(e) => {
              e.stopPropagation();
              onOpen(app);
            }}
          >
            <span>{t('miniApps.actions.open')}</span>
            <Right theme='outline' size={12} fill='currentColor' className='block' style={{ lineHeight: 0 }} />
          </button>
        </div>
      </div>

      {/* Hover actions */}
      <div
        className={[
          'absolute top-10px right-10px flex gap-6px',
          'pointer-events-none opacity-0 transition-opacity duration-150',
          'group-hover:pointer-events-auto group-hover:opacity-100',
        ].join(' ')}
        onClick={(e) => e.stopPropagation()}
      >
        {(
          [
            {
              key: 'rename',
              icon: <EditTwo theme='outline' size={14} strokeWidth={3} />,
              label: t('miniApps.actions.rename'),
              run: () => onRename(app),
            },
            {
              key: 'delete',
              icon: <Delete theme='outline' size={14} strokeWidth={3} />,
              label: t('miniApps.actions.delete'),
              run: () => onDelete(app),
              danger: true,
            },
          ] satisfies { key: string; icon: React.ReactNode; label: string; run: () => void; danger?: boolean }[]
        ).map((action) => (
          <div
            key={action.key}
            role='button'
            tabIndex={0}
            title={action.label}
            aria-label={action.label}
            onClick={action.run}
            onKeyDown={(e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault();
                action.run();
              }
            }}
            className={[
              'grid h-26px w-26px place-items-center rounded-8px cursor-pointer',
              'border border-solid border-[var(--color-border-2)] bg-[var(--color-bg-2)]',
              action.danger
                ? 'text-[var(--color-text-3)] hover:!border-[rgba(var(--danger-6),0.4)] hover:!text-danger-6 hover:!bg-[rgba(var(--danger-6),0.08)]'
                : 'text-[var(--color-text-3)] hover:border-[var(--color-border-3)] hover:text-[var(--color-text-1)] hover:bg-[var(--color-fill-2)]',
              'transition-colors',
            ].join(' ')}
          >
            {action.icon}
          </div>
        ))}
      </div>
    </div>
  );
};

// ─── Main page ────────────────────────────────────────────────────────────────

const MiniAppsListPage: React.FC = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const layout = useLayoutContext();
  const isMobile = layout?.isMobile ?? false;

  const [apps, setApps] = useState<IApiMiniApp[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState('');

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      setApps(await ipcBridge.miniapps.list.invoke());
      setError(null);
    } catch (e) {
      console.error('[miniapps] failed to load the library', e);
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Rename + delete are shared with the runner page (same copy, same dialogs);
  // only the follow-up differs — here both mutations just reload the grid.
  const reload = useCallback(() => {
    void refresh();
  }, [refresh]);
  const mutations = useMiniAppMutations({ onRenamed: reload, onDeleted: reload });

  const displayed = useMemo(() => {
    const q = searchQuery.trim().toLowerCase();
    if (!q) return apps;
    return apps.filter(
      (app) => app.name.toLowerCase().includes(q) || app.description.toLowerCase().includes(q)
    );
  }, [apps, searchQuery]);

  const openApp = useCallback(
    (app: IApiMiniApp) => navigate(`/mini-apps/${app.miniapp_id}`),
    [navigate]
  );
  const goCreate = useCallback(() => navigate(CREATE_ROUTE), [navigate]);

  // ─── Render ─────────────────────────────────────────────────────────────────

  return (
    <div
      className={[
        'size-full box-border overflow-y-auto',
        isMobile ? 'px-16px py-14px' : 'px-12px py-24px md:px-40px md:py-32px',
      ].join(' ')}
    >
      {mutations.node}
      <div className='mx-auto flex w-full max-w-1180px box-border flex-col gap-16px'>
        {/* Header */}
        <div className='flex w-full flex-wrap items-start justify-between gap-x-20px gap-y-12px'>
          <div className='min-w-0'>
            <h1 className={`${HUB_PAGE_TITLE_CLASS} mb-3px`}>{t('miniApps.title')}</h1>
            <p className='m-0 max-w-560px text-13px leading-19px text-[var(--color-text-3)]'>
              {t('miniApps.subtitle')}
            </p>
          </div>

          {!error && (apps.length > 0 || loading) && (
            <div className='flex items-center gap-10px'>
              <div className='flex w-200px items-center gap-8px rounded-10px border border-solid border-[var(--color-border-3)] bg-[var(--color-fill-2)] px-12px py-8px'>
                <Search theme='outline' size={14} className='flex-none text-[var(--color-text-3)]' />
                <input
                  className='w-full border-none bg-transparent text-13px font-[inherit] text-[var(--color-text-1)] outline-none placeholder:text-[var(--color-text-3)]'
                  placeholder={t('miniApps.list.searchPlaceholder')}
                  value={searchQuery}
                  onChange={(e) => setSearchQuery(e.target.value)}
                />
              </div>
              <Button type='primary' className='shrink-0' onClick={goCreate}>
                <span className='inline-flex items-center gap-6px'>
                  <Plus theme='outline' size='15' fill='currentColor' className='block' style={{ lineHeight: 0 }} />
                  {t('miniApps.actions.create')}
                </span>
              </Button>
            </div>
          )}
        </div>

        {/* Body states */}
        {error ? (
          <Result
            status='error'
            title={t('miniApps.errors.loadListFailed')}
            subTitle={error}
            extra={<Button onClick={() => void refresh()}>{t('miniApps.actions.refresh')}</Button>}
          />
        ) : loading ? (
          <div className='flex justify-center py-56px'>
            <Spin />
          </div>
        ) : apps.length === 0 ? (
          <div className='flex flex-col items-center justify-center gap-14px px-24px py-64px text-center'>
            <span className='flex h-72px w-72px items-center justify-center rounded-full bg-[var(--color-fill-2)] text-primary-6'>
              <ApplicationOne theme='outline' size='32' fill='currentColor' className='block' style={{ lineHeight: 0 }} />
            </span>
            <div className='flex flex-col gap-4px'>
              <span className='text-15px font-600 text-[var(--color-text-1)]'>
                {t('miniApps.empty.title')}
              </span>
              <span className='max-w-[460px] text-13px text-[var(--color-text-3)]'>
                {t('miniApps.empty.description')}
              </span>
            </div>
            <Button type='primary' onClick={goCreate}>
              <span className='inline-flex items-center gap-6px'>
                <Plus theme='outline' size='15' fill='currentColor' className='block' style={{ lineHeight: 0 }} />
                {t('miniApps.empty.cta')}
              </span>
            </Button>
          </div>
        ) : (
          <>
            <div
              className='grid gap-14px'
              style={{ gridTemplateColumns: 'repeat(auto-fill, minmax(min(280px, 100%), 1fr))' }}
            >
              {displayed.map((app) => (
                <MiniAppCard
                  key={app.miniapp_id}
                  app={app}
                  onOpen={openApp}
                  onRename={mutations.openRename}
                  onDelete={mutations.confirmDelete}
                />
              ))}
            </div>

            {displayed.length === 0 && (
              <div className='py-40px text-center text-13px text-[var(--color-text-3)]'>
                {t('miniApps.list.filterEmpty')}
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
};

export default MiniAppsListPage;
