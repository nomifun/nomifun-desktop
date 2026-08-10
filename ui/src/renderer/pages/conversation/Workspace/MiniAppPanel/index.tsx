/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * MiniAppPanel — 会话右栏常驻的小程序快捷入口
 *
 * A read-only companion to the `/mini-apps` library: it lists every solidified
 * mini-app, lets the user search them, and runs the one they pick *inside the
 * rail*, so a tool can be used without leaving the conversation it is being used
 * for. Selecting a card swaps the panel body for {@link MiniAppFrame} behind a
 * compact header (back / refresh / open full screen); "back" returns to the list
 * with the search query intact.
 *
 * Deliberately search-and-use only. Renaming and deleting stay in the left
 * "Mini-Apps" tab and on the full-page runner, so this file imports neither
 * `useMiniAppMutations` nor the update/delete bridge calls — a destructive action
 * one click deep in a 260px rail is not a trade worth making.
 *
 * Data: `ipcBridge.miniapps.list.invoke()` on mount plus an explicit refresh
 * control. The rail unmounts the inactive tab's body, so re-opening the panel
 * re-fetches — an app solidified earlier in this session shows up without a
 * restart. There is no mini-app WS event to subscribe to today.
 *
 * Rendered through the rail's existing `extraTabs` slot (registered once in
 * `useWorkspaceExtraTabs`), so it needs no changes to WorkspaceToolRail,
 * WorkspaceRailBody, WorkspacePanelHeader or ChatLayout. It follows the
 * ConversationTerminalPanel precedent: own data, own compact header row, own
 * loading/empty/error states, and a layout that survives the rail's 220px floor.
 *
 * Design spec: docs/specs/2026-08-09-miniapps.zh.md
 */
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { Empty, Spin } from '@arco-design/web-react';
import { ApplicationOne, ArrowLeft, FullScreen, Refresh, Right, Search } from '@icon-park/react';
import { ipcBridge } from '@/common';
import type { IApiMiniApp } from '@/common/adapter/ipcBridge';
import type { MiniAppId } from '@/common/types/ids';
import MiniAppFrame from '@/renderer/pages/miniApps/MiniAppFrame';
import { formatMiniAppRelativeTime } from '@/renderer/pages/miniApps/relativeTime';

// ─── Shared class lists ───────────────────────────────────────────────────────

/** Same primary affordance as the library card: "use it" must be on screen. */
const CARD_OPEN_BUTTON_CLASS = [
  'inline-flex shrink-0 items-center gap-3px h-22px px-8px rounded-8px cursor-pointer',
  'border border-solid border-[rgba(var(--primary-6),0.32)] bg-[rgba(var(--primary-6),0.10)]',
  'text-11px font-600 leading-none font-[inherit] text-primary-6',
  'transition-colors hover:bg-[rgba(var(--primary-6),0.18)]',
].join(' ');

const HEADER_ACTION_CLASS = [
  'grid h-24px w-24px place-items-center rounded-8px shrink-0 cursor-pointer',
  'text-[var(--color-text-2)] hover:bg-[var(--color-fill-2)] hover:text-[var(--color-text-1)]',
  'transition-colors',
].join(' ');

interface HeaderActionProps {
  label: string;
  onRun: () => void;
  children: React.ReactNode;
}

const HeaderAction: React.FC<HeaderActionProps> = ({ label, onRun, children }) => (
  <div
    role='button'
    tabIndex={0}
    title={label}
    aria-label={label}
    onClick={onRun}
    onKeyDown={(e) => {
      if (e.key === 'Enter' || e.key === ' ') {
        e.preventDefault();
        onRun();
      }
    }}
    className={HEADER_ACTION_CLASS}
  >
    {children}
  </div>
);

// ─── Card ─────────────────────────────────────────────────────────────────────

/**
 * The library card's visual language, minus the hover mutations and tightened
 * for a rail that can be 220px wide.
 */
const MiniAppQuickCard: React.FC<{ app: IApiMiniApp; onOpen: (app: IApiMiniApp) => void }> = ({
  app,
  onOpen,
}) => {
  const { t } = useTranslation();
  const icon = app.icon?.trim();

  return (
    <div
      className={[
        'group flex gap-10px overflow-hidden rounded-12px border border-solid p-10px',
        'border-[var(--color-border-2)] bg-[var(--color-bg-2)] box-border cursor-pointer',
        'transition-colors duration-160',
        'hover:border-[var(--color-border-3)] hover:bg-[var(--color-fill-2)]',
      ].join(' ')}
      onClick={() => onOpen(app)}
    >
      <span
        className='grid h-32px w-32px shrink-0 place-items-center rounded-10px bg-[var(--color-fill-2)] text-17px leading-none text-primary-6'
        aria-hidden='true'
      >
        {icon ? (
          icon
        ) : (
          <ApplicationOne theme='outline' size='17' fill='currentColor' className='block' style={{ lineHeight: 0 }} />
        )}
      </span>

      <div className='flex min-w-0 flex-1 flex-col gap-3px'>
        <div className='truncate text-13px font-600 leading-[1.35] text-[var(--color-text-1)]'>{app.name}</div>
        {app.description.trim() !== '' && (
          <div className='line-clamp-2 text-11px leading-16px text-[var(--color-text-3)]'>{app.description}</div>
        )}
        <div className='mt-auto flex items-center justify-between gap-6px pt-4px'>
          <span className='min-w-0 truncate text-11px leading-15px text-[var(--color-text-3)]'>
            {t('miniApps.list.updatedAt', { time: formatMiniAppRelativeTime(app.updated_at, t) })}
          </span>
          {/* The card body already opens it — this control must not fire twice. */}
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
            <Right theme='outline' size={11} fill='currentColor' className='block' style={{ lineHeight: 0 }} />
          </button>
        </div>
      </div>
    </div>
  );
};

// ─── Panel ────────────────────────────────────────────────────────────────────

const MiniAppPanel: React.FC = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();

  const [apps, setApps] = useState<IApiMiniApp[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState('');
  /** Which mini-app the rail is currently running; null = the searchable list. */
  const [runningId, setRunningId] = useState<MiniAppId | null>(null);
  /** Bumping this remounts the frame, which is the only honest "reload". */
  const [reloadToken, setReloadToken] = useState(0);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      setApps(await ipcBridge.miniapps.list.invoke());
      setError(null);
    } catch (e) {
      console.error('[miniapps] failed to load the library for the rail panel', e);
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  // The rail unmounts an inactive tab's body, so this doubles as "refresh when
  // the panel is shown again".
  useEffect(() => {
    void refresh();
  }, [refresh]);

  const displayed = useMemo(() => {
    const q = searchQuery.trim().toLowerCase();
    if (!q) return apps;
    return apps.filter(
      (app) => app.name.toLowerCase().includes(q) || app.description.toLowerCase().includes(q)
    );
  }, [apps, searchQuery]);

  // Derived, not stored: a mini-app deleted from the left tab simply drops out of
  // the next listing and the panel falls back to the list instead of running a
  // frame for something that no longer exists.
  const runningApp = useMemo(
    () => (runningId ? apps.find((app) => app.miniapp_id === runningId) ?? null : null),
    [apps, runningId]
  );

  const openApp = useCallback((app: IApiMiniApp) => setRunningId(app.miniapp_id), []);
  /** Back keeps `searchQuery` untouched, so the list reopens where it was left. */
  const backToList = useCallback(() => setRunningId(null), []);
  const openFullPage = useCallback(() => {
    if (runningApp) navigate(`/mini-apps/${runningApp.miniapp_id}`);
  }, [navigate, runningApp]);

  // ─── Running one mini-app ───────────────────────────────────────────────────

  if (runningApp) {
    const icon = runningApp.icon?.trim();
    return (
      <div className='flex size-full flex-col overflow-hidden'>
        <div className='shrink-0 flex items-center gap-6px px-8px py-6px border-b border-b-solid border-b-[var(--color-border-2)]'>
          {/* Its own label, not the generic `actions.back`: inside a conversation
              rail "返回" could just as easily mean the conversation. */}
          <HeaderAction label={t('miniApps.panel.backToList')} onRun={backToList}>
            <ArrowLeft theme='outline' size={15} strokeWidth={3} />
          </HeaderAction>

          <span
            className='grid h-22px w-22px shrink-0 place-items-center rounded-6px text-13px leading-none text-primary-6 bg-[rgba(var(--primary-6),0.12)]'
            aria-hidden='true'
          >
            {icon ? (
              icon
            ) : (
              <ApplicationOne theme='outline' size='13' fill='currentColor' className='block' style={{ lineHeight: 0 }} />
            )}
          </span>

          <span className='min-w-0 flex-1 truncate text-12px font-600 text-[var(--color-text-1)]'>
            {runningApp.name}
          </span>

          <HeaderAction
            label={t('miniApps.actions.refresh')}
            onRun={() => setReloadToken((token) => token + 1)}
          >
            <Refresh theme='outline' size={14} strokeWidth={3} />
          </HeaderAction>
          <HeaderAction label={t('miniApps.panel.openFullPage')} onRun={openFullPage}>
            <FullScreen theme='outline' size={14} strokeWidth={3} />
          </HeaderAction>
        </div>

        {/* Sizing lives here: the frame is size-full. */}
        <div className='relative flex-1 min-h-0 w-full bg-[var(--color-bg-1)]'>
          <MiniAppFrame miniAppId={runningApp.miniapp_id} name={runningApp.name} reloadToken={reloadToken} />
        </div>
      </div>
    );
  }

  // ─── The searchable list ────────────────────────────────────────────────────

  return (
    <div className='flex size-full flex-col gap-8px overflow-hidden p-10px box-border'>
      <div className='shrink-0 flex items-center gap-6px'>
        <div className='flex min-w-0 flex-1 items-center gap-8px rounded-10px border border-solid border-[var(--color-border-3)] bg-[var(--color-fill-2)] px-10px py-6px'>
          <Search theme='outline' size={14} className='flex-none text-[var(--color-text-3)]' />
          <input
            className='w-full border-none bg-transparent text-12px font-[inherit] text-[var(--color-text-1)] outline-none placeholder:text-[var(--color-text-3)]'
            placeholder={t('miniApps.list.searchPlaceholder')}
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
          />
        </div>
        <HeaderAction label={t('miniApps.actions.refresh')} onRun={() => void refresh()}>
          <Refresh theme='outline' size={14} strokeWidth={3} />
        </HeaderAction>
      </div>

      {error && (
        // `break-words`: the raw message is a backend string of unknown length and
        // the rail floors at 220px, so an unbroken URL would otherwise widen the
        // strip past the panel.
        <div className='shrink-0 break-words rounded-8px bg-danger-1 px-9px py-7px text-11px leading-16px text-danger-6'>
          {t('miniApps.errors.loadListFailed')}
          {`: ${error}`}
        </div>
      )}

      {loading && apps.length === 0 ? (
        <div className='flex flex-1 items-center justify-center'>
          <Spin />
        </div>
      ) : apps.length === 0 ? (
        // Only a library we successfully read as empty earns the empty state. On a
        // failed fetch we do not know what the user has, and the strip above
        // already said so — inviting them to go create their first mini-app on top
        // of a load error would be a lie about their own data.
        error ? null : (
          <div className='flex flex-1 flex-col items-center justify-center gap-8px px-6px text-center'>
            <Empty description={t('miniApps.empty.title')} />
            <span className='text-11px leading-16px text-[var(--color-text-3)]'>
              {t('miniApps.empty.description')}
            </span>
          </div>
        )
      ) : (
        <div className='min-h-0 flex-1 overflow-y-auto'>
          {/* One column in a narrow rail, more if the user widens it. */}
          <div
            className='grid gap-8px'
            style={{ gridTemplateColumns: 'repeat(auto-fill, minmax(min(280px, 100%), 1fr))' }}
          >
            {displayed.map((app) => (
              <MiniAppQuickCard key={app.miniapp_id} app={app} onOpen={openApp} />
            ))}
          </div>

          {displayed.length === 0 && (
            <div className='py-28px text-center text-12px text-[var(--color-text-3)]'>
              {t('miniApps.list.filterEmpty')}
            </div>
          )}
        </div>
      )}
    </div>
  );
};

export default MiniAppPanel;
