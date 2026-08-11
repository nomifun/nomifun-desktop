/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * MiniAppRunnerPage (`/mini-apps/:id`) — the full-page mini-app runtime.
 *
 * ONE column (spec D18): {@link MiniAppFrame} fills the body, and the toolbar
 * carries everything else. The frame is shared with the right-rail quick panel so
 * the two runtimes cannot drift (serve URL + sandbox + load watchdog).
 *
 * What the frame shows is always the PUBLISHED snapshot, while a conversation
 * edits the working copy on disk — so "the AI changed it" and "the app changed"
 * are two events, and this page has to make the gap legible: while a working copy
 * is newer than the snapshot the toolbar carries 「发布」 and one sentence saying
 * why. Without that users report 改了不生效.
 *
 * 「继续迭代」 leaves for an ORDINARY conversation ({@link useMiniAppIterate}):
 * this page hosts no chat of its own, which is exactly why it is a single column
 * again.
 *
 * Design spec: docs/specs/2026-08-10-miniapps-v3-unified-conversations.zh.md
 */
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useNavigate, useParams } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { Button, Result, Spin } from '@arco-design/web-react';
import { ApplicationOne, ArrowLeft, Browser, Delete, EditTwo, MagicWand, Refresh, Upload } from '@icon-park/react';
import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import type { IApiMiniApp } from '@/common/adapter/ipcBridge';
import { parseMiniAppId } from '@/common/types/ids';
import type { MiniAppId } from '@/common/types/ids';
import { useArcoMessage } from '@renderer/utils/ui/useArcoMessage';
import MiniAppFrame from './MiniAppFrame';
import { resolveMiniAppServeUrl } from './contract';
import { useMiniAppIterate } from './useMiniAppIterate';
import { useMiniAppMutations } from './useMiniAppMutations';

const TOOLBAR_ACTION_CLASS = [
  'grid h-32px w-32px place-items-center rounded-8px shrink-0 cursor-pointer',
  'text-[var(--color-text-2)] hover:bg-[var(--color-fill-2)] hover:text-[var(--color-text-1)]',
  'transition-colors',
].join(' ');

const TOOLBAR_DANGER_ACTION_CLASS = [
  'grid h-32px w-32px place-items-center rounded-8px shrink-0 cursor-pointer',
  'text-[var(--color-text-2)] hover:!text-danger-6 hover:!bg-[rgba(var(--danger-6),0.08)]',
  'transition-colors',
].join(' ');

interface ToolbarActionProps {
  label: string;
  danger?: boolean;
  onRun: () => void;
  children: React.ReactNode;
}

const ToolbarAction: React.FC<ToolbarActionProps> = ({ label, danger, onRun, children }) => (
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
    className={danger ? TOOLBAR_DANGER_ACTION_CLASS : TOOLBAR_ACTION_CLASS}
  >
    {children}
  </div>
);

const MiniAppRunnerPage: React.FC = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { id: rawId } = useParams<{ id: string }>();
  const [message, messageHolder] = useArcoMessage();

  // A malformed path segment is a not-found, not a crash.
  const miniAppId = useMemo<MiniAppId | null>(() => {
    if (rawId == null) return null;
    try {
      return parseMiniAppId(rawId);
    } catch {
      return null;
    }
  }, [rawId]);

  const [app, setApp] = useState<IApiMiniApp | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  /** Bumping this remounts the iframe, which is the only honest "reload". */
  const [reloadToken, setReloadToken] = useState(0);
  const [publishing, setPublishing] = useState(false);

  const load = useCallback(async () => {
    if (!miniAppId) {
      setApp(null);
      setLoading(false);
      return;
    }
    setLoading(true);
    try {
      setApp(await ipcBridge.miniapps.get.invoke({ miniapp_id: miniAppId }));
      setError(null);
    } catch (e) {
      // A deleted mini-app is a 404 from the detail route, and `httpBridge`
      // surfaces every non-2xx as a throw — it never resolves to null. Without
      // this branch a gone mini-app would show the retryable "load failed" card
      // instead of the honest "does not exist" one.
      if (isBackendHttpError(e) && e.status === 404) {
        setApp(null);
        setError(null);
        return;
      }
      console.error('[miniapps] failed to load the mini-app', e);
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [miniAppId]);

  useEffect(() => {
    void load();
  }, [load]);

  const goBack = useCallback(() => navigate('/mini-apps'), [navigate]);

  /**
   * 「刷新」 — remount the iframe AND re-read the record.
   *
   * Reloading the frame alone was the trap: the user iterated in another
   * conversation, came back, and the one control they reached for reloaded an
   * iframe that still serves the old snapshot — reinforcing 改了不生效. Refresh has
   * to be able to surface the publish state, not just repaint.
   */
  const refresh = useCallback(() => {
    setReloadToken((token) => token + 1);
    void load();
  }, [load]);

  const openInBrowser = useCallback(() => {
    if (!miniAppId) return;
    void ipcBridge.shell.openExternal.invoke(resolveMiniAppServeUrl(miniAppId));
  }, [miniAppId]);

  // ─── Iterate + publish ──────────────────────────────────────────────────────

  const { iterate, starting: iterating } = useMiniAppIterate();
  const startIterating = useCallback(() => {
    if (app) void iterate(app);
  }, [app, iterate]);

  const publishingRef = useRef(false);
  const publish = useCallback(async () => {
    if (!miniAppId || publishingRef.current) return;
    publishingRef.current = true;
    setPublishing(true);
    try {
      const published = await ipcBridge.miniapps.publish.invoke({ miniapp_id: miniAppId });
      setApp(published);
      // The served document just changed underneath a live iframe, so the frame
      // has to remount or the user would still be looking at the old app.
      setReloadToken((token) => token + 1);
      message.success(t('miniApps.publish.success'));
    } catch (e) {
      const detail = e instanceof Error ? e.message : String(e);
      message.error(t('miniApps.publish.failed', { message: detail }));
    } finally {
      publishingRef.current = false;
      setPublishing(false);
    }
  }, [miniAppId, message, t]);

  // ─── Rename + delete (shared with the library grid) ─────────────────────────

  const {
    node: mutationsNode,
    openRename: renameMiniApp,
    confirmDelete: deleteMiniApp,
  } = useMiniAppMutations({ onRenamed: setApp, onDeleted: goBack });
  const openRename = useCallback(() => {
    if (app) renameMiniApp(app);
  }, [app, renameMiniApp]);
  const handleDelete = useCallback(() => {
    if (app) deleteMiniApp(app);
  }, [app, deleteMiniApp]);

  // ─── Render ─────────────────────────────────────────────────────────────────

  if (loading) {
    return (
      <div className='size-full flex items-center justify-center'>
        <Spin />
      </div>
    );
  }

  if (error) {
    return (
      <div className='size-full flex items-center justify-center px-16px'>
        {mutationsNode}
        <Result
          status='error'
          title={t('miniApps.runner.loadError')}
          subTitle={error}
          extra={
            <div className='flex items-center justify-center gap-10px'>
              <Button onClick={goBack}>{t('miniApps.actions.back')}</Button>
              <Button type='primary' onClick={() => void load()}>
                {t('miniApps.actions.refresh')}
              </Button>
            </div>
          }
        />
      </div>
    );
  }

  if (!app || !miniAppId) {
    return (
      <div className='size-full flex items-center justify-center px-16px'>
        {mutationsNode}
        <Result
          status='warning'
          title={t('miniApps.runner.notFound')}
          extra={<Button onClick={goBack}>{t('miniApps.actions.back')}</Button>}
        />
      </div>
    );
  }

  const icon = app.icon?.trim();

  return (
    <div className='size-full flex flex-col overflow-hidden bg-[var(--color-bg-1)]'>
      {mutationsNode}
      {messageHolder}

      {/* Toolbar */}
      <div className='shrink-0 flex items-center gap-10px px-16px h-52px bg-[var(--color-bg-2)] border-b border-b-solid border-b-[var(--color-border-2)]'>
        <ToolbarAction label={t('miniApps.actions.back')} onRun={goBack}>
          <ArrowLeft theme='outline' size={18} strokeWidth={3} />
        </ToolbarAction>

        <span
          className='flex items-center justify-center w-28px h-28px rd-8px shrink-0 text-16px leading-none text-primary-6 bg-[rgba(var(--primary-6),0.12)]'
          aria-hidden='true'
        >
          {icon ? (
            icon
          ) : (
            <ApplicationOne theme='outline' size={16} fill='currentColor' className='block' style={{ lineHeight: 0 }} />
          )}
        </span>

        <span className='min-w-0 truncate text-15px font-700 text-[var(--color-text-1)]'>{app.name}</span>

        <div className='ml-auto flex items-center gap-6px'>
          {app.has_unpublished_changes && (
            <Button
              size='mini'
              type='primary'
              loading={publishing}
              icon={<Upload theme='outline' size='14' strokeWidth={3} />}
              onClick={() => void publish()}
            >
              {t('miniApps.publish.action')}
            </Button>
          )}
          {/* Labelled, not another 32px glyph: it is the only control here that
              leaves the page, and the only non-obvious one. */}
          <Button
            size='mini'
            loading={iterating}
            icon={<MagicWand theme='outline' size='14' strokeWidth={3} />}
            onClick={startIterating}
          >
            {t('miniApps.iterate.toggle')}
          </Button>
          <ToolbarAction label={t('miniApps.actions.refresh')} onRun={refresh}>
            <Refresh theme='outline' size={16} strokeWidth={3} />
          </ToolbarAction>
          <ToolbarAction label={t('miniApps.actions.openInBrowser')} onRun={openInBrowser}>
            <Browser theme='outline' size={16} strokeWidth={3} />
          </ToolbarAction>
          <ToolbarAction label={t('miniApps.actions.rename')} onRun={openRename}>
            <EditTwo theme='outline' size={16} strokeWidth={3} />
          </ToolbarAction>
          <ToolbarAction label={t('miniApps.actions.delete')} danger onRun={handleDelete}>
            <Delete theme='outline' size={16} strokeWidth={3} />
          </ToolbarAction>
        </div>
      </div>

      {/* One line about where the user's change is, or is not, yet. */}
      {app.has_unpublished_changes && (
        <div
          role='status'
          className='shrink-0 flex items-center gap-8px px-16px py-6px bg-[rgba(var(--warning-6),0.08)] border-b border-b-solid border-b-[var(--color-border-2)]'
        >
          <span className='shrink-0 text-12px font-600 text-warning-6'>{t('miniApps.publish.pending')}</span>
          <span className='min-w-0 text-12px leading-18px text-[var(--color-text-2)]'>
            {t('miniApps.publish.explain')}
          </span>
        </div>
      )}

      {/* Body — the published snapshot, filling the page. A flex child that may
          shrink AND has a resolved height: a percentage-height iframe under an
          auto-height ancestor collapses to 0px, which looks exactly like the
          blank render this layout exists to avoid. */}
      <div className='relative flex-1 min-h-0 w-full overflow-hidden bg-[var(--color-bg-1)]'>
        <MiniAppFrame miniAppId={miniAppId} name={app.name} reloadToken={reloadToken} />
      </div>
    </div>
  );
};

export default MiniAppRunnerPage;
