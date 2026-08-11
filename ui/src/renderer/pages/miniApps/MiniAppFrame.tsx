/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * MiniAppFrame — the one and only runtime for a *solidified* mini-app.
 *
 * Every surface that lets a user actually USE a mini-app mounts this component:
 * the full-page runner at `/mini-apps/:id` and the right-side quick panel. The
 * component owns three things those surfaces must never re-implement:
 *
 * 1. the serve URL ({@link resolveMiniAppServeUrl}) — the stored HTML streams
 *    from the backend's embed-whitelisted route, it never rides the JSON API;
 * 2. the sandbox ({@link MINI_APP_IFRAME_SANDBOX}) — the shared grant list
 *    withholds same-origin access, so AI-generated script runs with an opaque
 *    origin;
 * 3. a load watchdog. After {@link MINI_APP_LOAD_WATCHDOG_MS} without a single
 *    `load` event we overlay a slim, dismissible hint offering a retry and an
 *    escape hatch to the system browser — deliberately an overlay and not a
 *    full-screen error, because a slow-but-fine app must stay usable.
 *
 * What the watchdog can and cannot see: it only catches a frame that never
 * commits a document at all (backend down, a request that hangs). It does NOT
 * catch a *blank* frame — a document refused by the embed policy still fires
 * `load` on the iframe element, and the sandbox withholds same-origin access, so
 * this side can never inspect what rendered. Do not "fix" that by leaving the
 * timer armed past `load`: it would fire on every healthy app. The honest guard
 * against blank renders is the backend's embed whitelist, not this timer.
 *
 * Reload contract: **`reloadToken`**, not a ref handle. The parent bumps the
 * number and the iframe remounts, which is the only honest reload for a
 * cross-origin sandboxed document (`contentWindow.location.reload()` is not
 * reachable). Retrying from the watchdog hint bumps a private counter folded
 * into the same mount key, so both paths share one code path.
 *
 * Design spec: docs/specs/2026-08-09-miniapps.zh.md
 */
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Browser, Close, Refresh } from '@icon-park/react';
import { ipcBridge } from '@/common';
import type { MiniAppId } from '@/common/types/ids';
import { MINI_APP_IFRAME_SANDBOX, resolveMiniAppServeUrl } from './contract';

/**
 * Grace period before the frame admits something may be wrong. Generous on
 * purpose: a first paint behind a CDN-heavy app can take seconds, and crying
 * wolf on a working app is worse than a late hint.
 */
export const MINI_APP_LOAD_WATCHDOG_MS = 6000;

export interface MiniAppFrameProps {
  /** Solidified mini-app to run. */
  miniAppId: MiniAppId;
  /** Accessible name for the frame — usually the mini-app's display name. */
  name: string;
  /** Extra classes for the positioned container (sizing lives with the parent). */
  className?: string;
  /**
   * Bump to remount the frame. This is the component's whole imperative API:
   * there is no ref handle, so every consumer reloads the same way.
   */
  reloadToken?: number;
}

const HINT_BAR_CLASS = [
  'absolute left-12px right-12px bottom-12px z-10',
  'flex items-center gap-8px box-border rounded-10px px-12px py-8px',
  'border border-solid border-[var(--color-border-2)] bg-[var(--color-bg-2)]',
  'shadow-[0_8px_24px_rgba(0,0,0,0.14)]',
].join(' ');

const HINT_ACTION_CLASS = [
  'inline-flex shrink-0 items-center gap-4px h-24px px-8px rounded-8px cursor-pointer',
  'border border-solid border-[var(--color-border-3)] bg-[var(--color-bg-2)]',
  'text-12px leading-none font-[inherit] text-[var(--color-text-2)]',
  'transition-colors hover:border-[var(--color-border-4)] hover:bg-[var(--color-fill-2)] hover:text-[var(--color-text-1)]',
].join(' ');

const HINT_DISMISS_CLASS = [
  'grid shrink-0 h-24px w-24px place-items-center rounded-8px cursor-pointer',
  'border-0 bg-transparent',
  'text-[var(--color-text-3)] transition-colors hover:bg-[var(--color-fill-2)] hover:text-[var(--color-text-1)]',
].join(' ');

/**
 * Runs one solidified mini-app in a sandboxed iframe, with a load watchdog.
 */
const MiniAppFrame: React.FC<MiniAppFrameProps> = ({ miniAppId, name, className, reloadToken = 0 }) => {
  const { t } = useTranslation();
  /** Watchdog "retry" remounts too, so both reload paths share one mount key. */
  const [retryToken, setRetryToken] = useState(0);
  const [stalled, setStalled] = useState(false);
  const [dismissed, setDismissed] = useState(false);
  const watchdogRef = useRef<number | null>(null);

  const mountKey = `${miniAppId}:${reloadToken}:${retryToken}`;

  const clearWatchdog = useCallback(() => {
    if (watchdogRef.current != null) {
      window.clearTimeout(watchdogRef.current);
      watchdogRef.current = null;
    }
  }, []);

  // One timer per mount. Re-armed on every remount (external reload, retry, or a
  // different mini-app) and cleared on unmount, so a stale timer can never light
  // up the hint bar over a frame that already loaded.
  useEffect(() => {
    setStalled(false);
    setDismissed(false);
    clearWatchdog();
    watchdogRef.current = window.setTimeout(() => {
      watchdogRef.current = null;
      setStalled(true);
    }, MINI_APP_LOAD_WATCHDOG_MS);
    return clearWatchdog;
  }, [mountKey, clearWatchdog]);

  // The `load` event fires on the iframe *element*, in this document, whatever
  // the frame's own origin is — so it stays observable through the sandbox.
  const handleLoad = useCallback(() => {
    clearWatchdog();
    setStalled(false);
  }, [clearWatchdog]);

  const handleRetry = useCallback(() => setRetryToken((token) => token + 1), []);

  const openInBrowser = useCallback(() => {
    void ipcBridge.shell.openExternal.invoke(resolveMiniAppServeUrl(miniAppId)).catch((error) => {
      console.error('[miniapps] failed to open the mini-app in the browser', error);
    });
  }, [miniAppId]);

  const showHint = stalled && !dismissed;

  return (
    <div className={`relative size-full overflow-hidden ${className ?? ''}`}>
      <iframe
        key={mountKey}
        src={resolveMiniAppServeUrl(miniAppId)}
        sandbox={MINI_APP_IFRAME_SANDBOX}
        title={name}
        onLoad={handleLoad}
        className='block size-full border-0'
      />

      {showHint && (
        <div className={HINT_BAR_CLASS} role='status'>
          <span className='min-w-0 flex-1 text-12px leading-17px text-[var(--color-text-2)]'>
            {t('miniApps.frame.stalledHint')}
          </span>
          <button type='button' className={HINT_ACTION_CLASS} onClick={handleRetry}>
            <Refresh theme='outline' size={12} fill='currentColor' className='block' style={{ lineHeight: 0 }} />
            <span>{t('miniApps.actions.retry')}</span>
          </button>
          <button type='button' className={HINT_ACTION_CLASS} onClick={openInBrowser}>
            <Browser theme='outline' size={12} fill='currentColor' className='block' style={{ lineHeight: 0 }} />
            <span>{t('miniApps.actions.openInBrowser')}</span>
          </button>
          <button
            type='button'
            className={HINT_DISMISS_CLASS}
            title={t('miniApps.actions.dismissHint')}
            aria-label={t('miniApps.actions.dismissHint')}
            onClick={() => setDismissed(true)}
          >
            <Close theme='outline' size={12} fill='currentColor' className='block' style={{ lineHeight: 0 }} />
          </button>
        </div>
      )}
    </div>
  );
};

export default MiniAppFrame;
