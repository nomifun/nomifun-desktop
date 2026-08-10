/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * MiniAppRunnerPage (`/mini-apps/:id`) — the full-page mini-app runtime.
 *
 * This is the landing surface of the left "Mini-Apps" tab: opening a card here
 * must be a self-contained way to USE the app, never a detour through the
 * conversation that built it. The runtime itself lives in {@link MiniAppFrame}
 * (serve URL + sandbox + load watchdog) so this page and the right-side quick
 * panel cannot drift apart; what stays here is the chrome — back, reload,
 * rename, delete, and a clearly secondary way back to the source conversation
 * for *further editing*.
 *
 * Metadata (name, icon, source conversation) comes from the authenticated detail
 * call, which is also what distinguishes "deleted" from "failed to load".
 *
 * Design spec: docs/specs/2026-08-09-miniapps.zh.md
 */
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useNavigate, useParams } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { Button, Result, Spin } from '@arco-design/web-react';
import { ApplicationOne, ArrowLeft, Delete, EditTwo, Refresh } from '@icon-park/react';
import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import type { IApiMiniApp } from '@/common/adapter/ipcBridge';
import { parseMiniAppId } from '@/common/types/ids';
import type { MiniAppId } from '@/common/types/ids';
import MiniAppFrame from './MiniAppFrame';
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

        <div className='ml-auto flex items-center gap-4px'>
          <ToolbarAction
            label={t('miniApps.actions.refresh')}
            onRun={() => setReloadToken((token) => token + 1)}
          >
            <Refresh theme='outline' size={16} strokeWidth={3} />
          </ToolbarAction>
          <ToolbarAction label={t('miniApps.actions.rename')} onRun={openRename}>
            <EditTwo theme='outline' size={16} strokeWidth={3} />
          </ToolbarAction>
          <ToolbarAction label={t('miniApps.actions.delete')} danger onRun={handleDelete}>
            <Delete theme='outline' size={16} strokeWidth={3} />
          </ToolbarAction>
        </div>
      </div>

      {/* Runtime — shared with the right-side quick panel; reload = remount. */}
      <div className='relative flex-1 min-h-0 w-full bg-[var(--color-bg-1)]'>
        <MiniAppFrame miniAppId={miniAppId} name={app.name} reloadToken={reloadToken} />
      </div>
    </div>
  );
};

export default MiniAppRunnerPage;
