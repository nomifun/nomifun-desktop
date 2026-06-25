/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Left, Right, Refresh, Loading } from '@icon-park/react';

export interface WebviewHostProps {
  /** URL to display */
  url: string;
  /** Unique key for session persistence */
  id?: string;
  /** Whether to show the navigation bar (back/forward/refresh/URL) */
  showNavBar?: boolean;
  /**
   * Cache/session partition. Kept in the public API for compatibility with
   * the previous Electron `<webview>` implementation, but ignored under the
   * iframe runtime — there is no portable equivalent across Tauri / WebUI.
   */
  partition?: string;
  /** Extra class names for root container */
  className?: string;
  /** Extra styles for root container */
  style?: React.CSSProperties;
  /** Called when the page finishes loading */
  onDidFinishLoad?: () => void;
  /** Called when the page fails to load */
  onDidFailLoad?: (errorCode: number, errorDescription: string) => void;
}

const MIN_ZOOM_FACTOR = 0.75;
const MAX_ZOOM_FACTOR = 1.5;

/**
 * Shared embedded-browser host component.
 *
 * Renders a sandboxed `<iframe>` so it works under both Tauri and the WebUI
 * browser runtime (the previous Electron `<webview>` tag exists in neither).
 *
 * Features:
 * - Self-managed history stacks (back / forward) by swapping `iframe.src`
 *   — note that cross-origin iframes don't expose their internal history,
 *   so this only tracks navigations driven through this component.
 * - Loading indicator via iframe `onLoad` / `onError`.
 * - Optional navigation bar (hidden by default for embedded use).
 * - Best-effort CSS zoom for star-office localhost previews. This is a no-op
 *   for true cross-origin pages but is harmless and preserves the existing
 *   toolbar UX.
 */
const WebviewHost: React.FC<WebviewHostProps> = ({
  url,
  id: _id,
  showNavBar = false,
  partition: _partition,
  className,
  style,
  onDidFinishLoad,
  onDidFailLoad,
}) => {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const contentRef = useRef<HTMLDivElement | null>(null);
  const iframeRef = useRef<HTMLIFrameElement | null>(null);

  // Navigation state
  const [currentUrl, setCurrentUrl] = useState(url);
  const [inputUrl, setInputUrl] = useState(url);
  const [isLoading, setIsLoading] = useState(true);
  const [zoomFactor, setZoomFactor] = useState(1);

  // Self-managed history stacks
  const historyBackRef = useRef<string[]>([]);
  const historyForwardRef = useRef<string[]>([]);
  const [canGoBack, setCanGoBack] = useState(false);
  const [canGoForward, setCanGoForward] = useState(false);

  const isStarOfficeUrl = useCallback((targetUrl: string): boolean => {
    try {
      const parsed = new URL(targetUrl);
      const host = parsed.hostname.toLowerCase();
      const localHost = host === '127.0.0.1' || host === 'localhost';
      const knownPort = ['18791', '18888', '19000'].includes(parsed.port);
      return localHost && knownPort;
    } catch {
      return false;
    }
  }, []);

  const isStarOffice = isStarOfficeUrl(currentUrl);

  // Reset when props.url changes
  useEffect(() => {
    historyBackRef.current = [];
    historyForwardRef.current = [];
    setCanGoBack(false);
    setCanGoForward(false);
    setCurrentUrl(url);
    setInputUrl(url);
    setIsLoading(true);
    setZoomFactor(1);
  }, [url]);

  // Apply best-effort CSS zoom to the iframe element. For cross-origin
  // documents the browser won't actually scale the inner document via this,
  // but for same-origin (localhost star-office) it works through CSS `zoom`.
  useEffect(() => {
    const iframeEl = iframeRef.current;
    if (!iframeEl) return;
    const factor = isStarOffice ? zoomFactor : 1;
    // `zoom` is non-standard but widely supported and the only CSS knob that
    // affects iframe content sizing without breaking layout boxes.
    (iframeEl.style as CSSStyleDeclaration & { zoom?: string }).zoom = String(factor);
  }, [isStarOffice, zoomFactor]);

  // Navigate to new URL (add to history)
  const navigateToWithHistory = useCallback(
    (targetUrl: string) => {
      if (!targetUrl || targetUrl === currentUrl) return;

      if (currentUrl) {
        historyBackRef.current.push(currentUrl);
      }
      historyForwardRef.current = [];

      setCurrentUrl(targetUrl);
      setInputUrl(targetUrl);
      setCanGoBack(historyBackRef.current.length > 0);
      setCanGoForward(false);
      setIsLoading(true);
    },
    [currentUrl]
  );

  // Iframe load / error handlers
  const handleIframeLoad = useCallback(() => {
    setIsLoading(false);
    onDidFinishLoad?.();
  }, [onDidFinishLoad]);

  const handleIframeError = useCallback(() => {
    setIsLoading(false);
    // The iframe `onError` event carries no error code/description, so we
    // surface a generic failure to keep the previous callback signature.
    onDidFailLoad?.(-1, 'iframe failed to load');
  }, [onDidFailLoad]);

  const handleZoomReset = useCallback(() => {
    if (!isStarOffice) return;
    setZoomFactor(1);
  }, [isStarOffice]);

  const handleZoomFit = useCallback(() => {
    const iframeEl = iframeRef.current;
    const contentEl = contentRef.current;
    if (!isStarOffice || !iframeEl || !contentEl) return;
    // Same-origin only: try to read the inner document width to fit.
    try {
      const innerDoc = iframeEl.contentDocument;
      const innerWin = iframeEl.contentWindow;
      if (!innerDoc || !innerWin) return;
      const stage = innerDoc.getElementById('main-stage');
      const body = innerDoc.body;
      const docEl = innerDoc.documentElement;
      const stageWidth = Math.max(
        stage?.scrollWidth || 0,
        body?.scrollWidth || 0,
        docEl?.scrollWidth || 0,
        innerWin.innerWidth || 0
      );
      if (!stageWidth) return;
      const next = Number((contentEl.clientWidth / stageWidth).toFixed(2));
      setZoomFactor(Math.max(MIN_ZOOM_FACTOR, Math.min(MAX_ZOOM_FACTOR, next)));
    } catch {
      // Cross-origin or detached document — silently ignore.
    }
  }, [isStarOffice]);

  const handleOuterWheelZoom = useCallback(
    (event: React.WheelEvent<HTMLDivElement>) => {
      if (!isStarOffice) return;
      if (!(event.ctrlKey || event.metaKey)) return;
      event.preventDefault();
      const step = event.deltaY < 0 ? 0.08 : -0.08;
      setZoomFactor((prev) => {
        const next = Number((prev + step).toFixed(2));
        return Math.max(MIN_ZOOM_FACTOR, Math.min(MAX_ZOOM_FACTOR, next));
      });
    },
    [isStarOffice]
  );

  // Back
  const handleGoBack = useCallback(() => {
    if (historyBackRef.current.length === 0) return;
    const prevUrl = historyBackRef.current.pop()!;
    historyForwardRef.current.push(currentUrl);
    setCanGoBack(historyBackRef.current.length > 0);
    setCanGoForward(true);
    setCurrentUrl(prevUrl);
    setInputUrl(prevUrl);
    setIsLoading(true);
  }, [currentUrl]);

  // Forward
  const handleGoForward = useCallback(() => {
    if (historyForwardRef.current.length === 0) return;
    const nextUrl = historyForwardRef.current.pop()!;
    historyBackRef.current.push(currentUrl);
    setCanGoBack(true);
    setCanGoForward(historyForwardRef.current.length > 0);
    setCurrentUrl(nextUrl);
    setInputUrl(nextUrl);
    setIsLoading(true);
  }, [currentUrl]);

  // Refresh
  const handleRefresh = useCallback(() => {
    const iframeEl = iframeRef.current;
    if (!iframeEl) return;
    setIsLoading(true);
    // Reassigning src forces a reload across origins (contentWindow.location
    // would throw on cross-origin frames).
    iframeEl.src = currentUrl;
  }, [currentUrl]);

  // URL bar submit
  const handleUrlSubmit = useCallback(
    (e: React.FormEvent) => {
      e.preventDefault();
      let targetUrl = inputUrl.trim();
      if (!targetUrl) return;
      if (!/^https?:\/\//i.test(targetUrl)) {
        targetUrl = 'https://' + targetUrl;
      }
      navigateToWithHistory(targetUrl);
    },
    [inputUrl, navigateToWithHistory]
  );

  const handleUrlKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLInputElement>) => {
      if (e.key === 'Escape') {
        setInputUrl(currentUrl);
        (e.target as HTMLInputElement).blur();
      }
    },
    [currentUrl]
  );

  return (
    <div ref={containerRef} className={`h-full w-full flex flex-col ${className ?? ''}`} style={style}>
      {showNavBar && (
        <style>
          {`
            .nomi-url-viewer-toolbar {
              --viewer-border: var(--color-border-2);
              --viewer-border-hover: var(--color-border-3);
              --viewer-bg: var(--color-bg-3);
              --viewer-bg-hover: var(--color-fill-2);
              --viewer-text: var(--color-text-2);
              --viewer-text-muted: var(--color-text-3);
            }
            .nomi-url-viewer-toolbar .toolbar-btn {
              -webkit-appearance: none;
              appearance: none;
              display: inline-flex;
              align-items: center;
              justify-content: center;
              height: 30px;
              min-width: 30px;
              padding: 0 10px;
              border-radius: 10px;
              border: 1px solid var(--viewer-border);
              background: var(--viewer-bg);
              color: var(--viewer-text);
              line-height: 1;
              font-size: 12px;
              transition: all 150ms ease;
              cursor: pointer;
            }
            .nomi-url-viewer-toolbar .toolbar-btn.icon-btn {
              width: 30px;
              min-width: 30px;
              padding: 0;
            }
            .nomi-url-viewer-toolbar .toolbar-btn:hover:not(:disabled) {
              background: var(--viewer-bg-hover);
              border-color: var(--viewer-border-hover);
            }
            .nomi-url-viewer-toolbar .toolbar-btn:active:not(:disabled) {
              transform: translateY(0.5px);
            }
            .nomi-url-viewer-toolbar .toolbar-btn:focus-visible {
              outline: none;
              border-color: rgb(var(--primary-6));
              box-shadow: 0 0 0 2px rgba(var(--primary-6), 0.12);
            }
            .nomi-url-viewer-toolbar .toolbar-btn:disabled {
              opacity: 0.55;
              cursor: not-allowed;
              color: var(--viewer-text-muted);
              background: var(--color-bg-2);
            }
            .nomi-url-viewer-toolbar .toolbar-chip {
              display: inline-flex;
              align-items: center;
              justify-content: center;
              height: 30px;
              min-width: 48px;
              padding: 0 10px;
              border-radius: 10px;
              border: 1px solid var(--viewer-border);
              background: var(--color-bg-2);
              color: var(--viewer-text-muted);
              font-size: 11px;
              line-height: 1;
            }
            .nomi-url-viewer-toolbar .toolbar-input {
              -webkit-appearance: none;
              appearance: none;
              width: 100%;
              height: 30px;
              padding: 0 12px;
              border-radius: 10px;
              border: 1px solid var(--viewer-border);
              background: var(--viewer-bg);
              color: var(--color-text-1);
              font-size: 12px;
              line-height: 30px;
              transition: all 150ms ease;
            }
            .nomi-url-viewer-toolbar .toolbar-input:hover {
              border-color: var(--viewer-border-hover);
            }
            .nomi-url-viewer-toolbar .toolbar-input:focus {
              outline: none;
              border-color: rgb(var(--primary-6));
              box-shadow: 0 0 0 2px rgba(var(--primary-6), 0.12);
            }
          `}
        </style>
      )}
      {/* Navigation bar (optional) */}
      {showNavBar && (
        <div className='nomi-url-viewer-toolbar flex items-center gap-6px h-40px px-10px bg-bg-2 border-b border-border-1 flex-shrink-0'>
          <button onClick={handleGoBack} disabled={!canGoBack} className='toolbar-btn icon-btn' title='Back'>
            <Left theme='outline' size={16} />
          </button>
          <button onClick={handleGoForward} disabled={!canGoForward} className='toolbar-btn icon-btn' title='Forward'>
            <Right theme='outline' size={16} />
          </button>
          <button onClick={handleRefresh} className='toolbar-btn icon-btn' title='Refresh'>
            {isLoading ? (
              <Loading theme='outline' size={16} className='animate-spin' />
            ) : (
              <Refresh theme='outline' size={16} />
            )}
          </button>
          {isStarOffice && (
            <div className='flex items-center gap-6px ml-2px'>
              <button onClick={handleZoomReset} className='toolbar-btn' title='Reset zoom'>
                100%
              </button>
              <button onClick={handleZoomFit} className='toolbar-btn' title='Fit'>
                Fit
              </button>
              <span className='toolbar-chip'>{Math.round(zoomFactor * 100)}%</span>
            </div>
          )}
          <form onSubmit={handleUrlSubmit} className='flex-1 ml-2px'>
            <input
              type='text'
              value={inputUrl}
              onChange={(e) => setInputUrl(e.target.value)}
              onKeyDown={handleUrlKeyDown}
              onFocus={(e) => e.target.select()}
              className='toolbar-input'
              placeholder='Enter URL...'
            />
          </form>
        </div>
      )}

      {/* Loading indicator (when no nav bar) */}
      {!showNavBar && isLoading && (
        <div className='absolute inset-0 flex items-center justify-center text-t-secondary text-14px z-10 pointer-events-none'>
          <span className='animate-pulse'>Loading…</span>
        </div>
      )}

      {/* Iframe content area */}
      <div
        ref={contentRef}
        className='flex-1 overflow-hidden relative'
        style={{ minHeight: 0 }}
        onWheel={handleOuterWheelZoom}
      >
        <iframe
          ref={iframeRef}
          src={currentUrl}
          // Permissive sandbox: allow scripts + same-origin so localhost
          // star-office and extension pages work, plus forms/popups for
          // typical external https sites loaded by URLViewer.
          sandbox='allow-scripts allow-same-origin allow-forms allow-popups allow-popups-to-escape-sandbox'
          referrerPolicy='no-referrer-when-downgrade'
          onLoad={handleIframeLoad}
          onError={handleIframeError}
          className='w-full h-full border-0 absolute left-0 top-0'
          style={{
            opacity: !showNavBar && isLoading ? 0 : 1,
            transition: 'opacity 150ms ease-in',
          }}
          title='Embedded content'
        />
      </div>
    </div>
  );
};

export default WebviewHost;
