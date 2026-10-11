/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import { getBaseUrl, isBackendHttpError } from '@/common/adapter/httpBridge';
import type { OfficePreviewDocType, OfficePreviewError, OfficePreviewStatus } from '@/common/types/office/preview';
import { uuidv7 } from '@/common/utils/uuidv7';
import { Button, Spin } from '@arco-design/web-react';
import React, { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

const POLL_INTERVAL_MS = 500;
const FRAME_READY_TIMEOUT_MS = 30_000;
const IFRAME_TITLE: Record<OfficePreviewDocType, string> = {
  ppt: 'PowerPoint Preview', word: 'Word Preview', excel: 'Excel Preview',
};

const ERROR_I18N_KEYS: Record<string, string> = {
  ENGINE_UNAVAILABLE: 'preview.office.errors.componentUnavailable',
  FORMAT_UNSUPPORTED: 'preview.office.errors.unsupportedFormat',
  FILE_CHANGED: 'preview.office.errors.fileUnstable',
  FILE_UNAVAILABLE: 'preview.office.errors.fileUnavailable',
  DOCUMENT_INVALID: 'preview.office.errors.documentInvalid',
  PREPARATION_TIMEOUT: 'preview.office.errors.preparationTimeout',
  TIMEOUT: 'preview.office.errors.preparationTimeout',
  RESOURCE_LIMIT: 'preview.office.errors.resourceLimit',
  CACHE_BUSY: 'preview.office.errors.cacheBusy',
  RENDER_FAILED: 'preview.office.errors.renderFailed',
  RENDER_TIMEOUT: 'preview.office.errors.renderTimeout',
  CLEANUP_FAILED: 'preview.office.errors.cleanupFailed',
  OUTPUT_INVALID: 'preview.office.errors.invalidArtifact',
  CANCELLED: 'preview.office.errors.cancelled',
  OFFICE_PREVIEW_INVALID_RESPONSE: 'preview.office.errors.invalidArtifact',
  OFFICE_PREVIEW_DISPLAY_FAILED: 'preview.office.errors.displayFailed',
  OFFICE_PREVIEW_CONNECTION_FAILED: 'preview.office.errors.connectionFailed',
  FILE_NOT_FOUND: 'preview.office.errors.fileNotFound',
  NOT_FOUND: 'preview.office.errors.requestUnavailable',
  FORBIDDEN: 'preview.office.errors.permissionDenied',
  UNAUTHORIZED: 'preview.office.errors.permissionDenied',
  BAD_REQUEST: 'preview.office.errors.invalidRequest',
  PATH_OUTSIDE_SANDBOX: 'preview.office.errors.outsideSandbox',
};

interface Frame {
  requestId: string;
  capability: string;
  nonce: string;
  url: string;
  identity: string;
}
interface Request {
  id: string;
  cancelled: boolean;
  committed: boolean;
  poll?: ReturnType<typeof setTimeout>;
}
interface OfficeArtifactViewerProps {
  docType: OfficePreviewDocType;
  file_path?: string;
  workspace?: string;
  revision?: number;
}

/** Only a canonical root capability URL returned by our backend is embeddable. */
export function resolveOfficeArtifactUrl(url: string, capability: string, baseUrl = getBaseUrl()): string {
  if (!/^[0-9a-f]{64}$/.test(capability) || url !== `/api/office-preview-artifacts/${capability}/`) {
    throw new Error('Invalid Office preview capability URL');
  }
  return `${baseUrl.replace(/\/+$/, '')}${url}`;
}

/** Sandboxed frames have an opaque origin; authenticate their source and lease nonce. */
function isOfficeArtifactMessage(event: MessageEvent, source: Window | null, frame: Pick<Frame, 'requestId' | 'capability' | 'nonce'>, type: string): boolean {
  if (!source || event.source !== source || event.origin !== 'null' || !event.data || typeof event.data !== 'object') return false;
  const data = event.data as Record<string, unknown>;
  return data.type === type && data.request_id === frame.requestId
    && data.capability === frame.capability && data.nonce === frame.nonce;
}

export function isOfficeArtifactReadyMessage(event: MessageEvent, source: Window | null, frame: Pick<Frame, 'requestId' | 'capability' | 'nonce'>): boolean {
  return isOfficeArtifactMessage(event, source, frame, 'nomifun-office-preview-ready');
}

/** First paint can wait for child rAF while the app/tab is hidden. Charge only visible time. */
export function scheduleOfficeFrameDeadline(callback: () => void, durationMs = FRAME_READY_TIMEOUT_MS): () => void {
  let remaining = durationMs;
  let started: number | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let stopped = false;
  const pause = () => {
    if (timer) clearTimeout(timer);
    timer = undefined;
    if (started !== null) remaining = Math.max(0, remaining - (performance.now() - started));
    started = null;
  };
  const visibility = () => {
    pause();
    if (stopped || document.visibilityState !== 'visible') return;
    started = performance.now();
    timer = setTimeout(() => {
      if (document.visibilityState !== 'visible') { visibility(); return; }
      stopped = true;
      document.removeEventListener('visibilitychange', visibility);
      callback();
    }, remaining);
  };
  document.addEventListener('visibilitychange', visibility);
  visibility();
  return () => { stopped = true; pause(); document.removeEventListener('visibilitychange', visibility); };
}

const release = (requestId: string) => {
  void ipcBridge.officePreview.stop.invoke({ request_id: requestId }).catch(() => {
    // Backend leases also expire. A detached/unreachable renderer cannot block cleanup.
  });
};

const OfficeArtifactViewer: React.FC<OfficeArtifactViewerProps> = ({ docType, file_path, workspace, revision = 0 }) => {
  const { t } = useTranslation();
  const identity = JSON.stringify([docType, file_path, workspace]);
  const [phase, setPhase] = useState<'queued' | 'rendering' | 'displaying' | 'ready' | 'error'>('queued');
  const [error, setError] = useState<OfficePreviewError | null>(null);
  const [retry, setRetry] = useState(0);
  const [displayed, setDisplayed] = useState<Frame | null>(null);
  const [candidate, setCandidate] = useState<Frame | null>(null);
  const displayedRef = useRef<Frame | null>(null);
  const activeRequest = useRef<Request | null>(null);
  const frames = useRef(new Map<string, HTMLIFrameElement>());
  const positions = useRef(new Map<string, { x: number; y: number; sheet?: number }>());

  useEffect(() => {
    if (displayedRef.current?.identity !== identity) {
      if (displayedRef.current) release(displayedRef.current.requestId);
      displayedRef.current = null;
      setDisplayed(null);
    }
    setCandidate(null);
    setError(null);
    setPhase('queued');
    if (!file_path) {
      setError({ code: 'FILE_NOT_FOUND', message: 'File path is missing', retryable: false });
      setPhase('error');
      return;
    }

    const request: Request = { id: uuidv7(), cancelled: false, committed: false };
    activeRequest.current = request;
    const fail = (nextError: OfficePreviewError) => {
      if (request.cancelled || activeRequest.current !== request) return;
      setCandidate(null);
      setError(nextError);
      setPhase('error');
      release(request.id);
    };
    const observe = (result: OfficePreviewStatus) => {
      if (request.cancelled || activeRequest.current !== request) {
        // stop may have raced a not-yet-arrived start request; release again.
        release(request.id);
        return;
      }
      if (!result || result.request_id !== request.id) {
        fail({ code: 'OFFICE_PREVIEW_INVALID_RESPONSE', message: 'Invalid Office preview response', retryable: false });
        return;
      }
      if (result.state === 'error') {
        const nextError = result.error;
        fail(nextError && typeof nextError.code === 'string' && typeof nextError.message === 'string' && typeof nextError.retryable === 'boolean'
          ? nextError : { code: 'OFFICE_PREVIEW_INVALID_RESPONSE', message: 'Invalid Office preview error', retryable: false });
      } else if (result.state === 'ready') {
        try {
          const resolved = resolveOfficeArtifactUrl(result.url ?? '', result.capability ?? '');
          const nonce = uuidv7();
          const query = new URLSearchParams({ request_id: request.id, nonce });
          setCandidate({ requestId: request.id, capability: result.capability!, nonce, url: `${resolved}?${query}`, identity });
          setPhase('displaying');
        } catch {
          fail({ code: 'OFFICE_PREVIEW_INVALID_RESPONSE', message: 'Invalid Office preview artifact', retryable: false });
        }
      } else if (result.state === 'queued' || result.state === 'rendering') {
        setPhase(result.state);
        request.poll = setTimeout(() => { void poll(); }, POLL_INTERVAL_MS);
      } else {
        fail({ code: 'OFFICE_PREVIEW_INVALID_RESPONSE', message: 'Invalid Office preview state', retryable: false });
      }
    };
    const rejected = (reason: unknown) => {
      const httpError = isBackendHttpError(reason);
      fail({
        code: httpError ? reason.code : 'OFFICE_PREVIEW_CONNECTION_FAILED',
        message: reason instanceof Error ? reason.message : 'Office preview request failed',
        retryable: !httpError || reason.status >= 500 || reason.status === 408 || reason.status === 429,
      });
    };
    const poll = async () => {
      try { observe(await ipcBridge.officePreview.status.invoke({ request_id: request.id })); }
      catch (reason) { rejected(reason); }
    };
    void ipcBridge.officePreview.start.invoke({ request_id: request.id, file_path, workspace, doc_type: docType }).then(observe, rejected);

    return () => {
      request.cancelled = true;
      if (request.poll) clearTimeout(request.poll);
      if (activeRequest.current === request) activeRequest.current = null;
      // Keep a painted prior version until the replacement paints. Pending jobs
      // are cancelled immediately, even when start has not acknowledged them.
      if (!request.committed) release(request.id);
    };
  }, [docType, file_path, identity, retry, revision, workspace]);

  useEffect(() => {
    if (!candidate) return;
    const request = activeRequest.current;
    if (!request || request.id !== candidate.requestId) return;
    const failDisplay = () => {
      if (request.cancelled || request.committed || activeRequest.current !== request) return;
      setCandidate(null);
      setPhase('error');
      setError({ code: 'OFFICE_PREVIEW_DISPLAY_FAILED', message: 'The generated preview could not be displayed', retryable: true });
      release(request.id);
    };
    const cancelDeadline = scheduleOfficeFrameDeadline(failDisplay);
    const ready = (event: MessageEvent) => {
      if (request.cancelled || activeRequest.current !== request || !isOfficeArtifactReadyMessage(event, frames.current.get(candidate.requestId)?.contentWindow ?? null, candidate)) return;
      request.committed = true;
      if (displayedRef.current && displayedRef.current.requestId !== candidate.requestId) release(displayedRef.current.requestId);
      const position = positions.current.get(candidate.identity);
      if (position) frames.current.get(candidate.requestId)?.contentWindow?.postMessage({
        type: 'nomifun-office-preview-restore-scroll', request_id: candidate.requestId,
        capability: candidate.capability, nonce: candidate.nonce, ...position,
      }, '*');
      displayedRef.current = candidate;
      setDisplayed(candidate);
      setCandidate(null);
      setError(null);
      setPhase('ready');
    };
    window.addEventListener('message', ready);
    const iframe = frames.current.get(candidate.requestId);
    iframe?.addEventListener('error', failDisplay);
    return () => { cancelDeadline(); window.removeEventListener('message', ready); iframe?.removeEventListener('error', failDisplay); };
  }, [candidate]);

  useEffect(() => {
    const scrolled = (event: MessageEvent) => {
      const frame = displayedRef.current;
      if (!frame || !isOfficeArtifactMessage(event, frames.current.get(frame.requestId)?.contentWindow ?? null, frame, 'nomifun-office-preview-scroll')) return;
      const { x, y, sheet } = event.data as { x?: unknown; y?: unknown; sheet?: unknown };
      if (sheet !== undefined && (typeof sheet !== 'number' || !Number.isInteger(sheet) || sheet < 0 || sheet > 4096)) return;
      if (typeof x === 'number' && typeof y === 'number' && Number.isFinite(x) && Number.isFinite(y) && x >= 0 && y >= 0) {
        positions.current.set(frame.identity, { x, y, ...(typeof sheet === 'number' ? { sheet } : {}) });
      }
    };
    window.addEventListener('message', scrolled);
    return () => { window.removeEventListener('message', scrolled); };
  }, []);

  useEffect(() => () => {
    if (displayedRef.current) release(displayedRef.current.requestId);
    displayedRef.current = null;
  }, []);

  const visibleFrames = [displayed, candidate].filter((frame): frame is Frame => frame !== null && frame.identity === identity);
  const updating = phase !== 'ready' && displayed?.identity === identity;
  const errorMessage = error ? t(ERROR_I18N_KEYS[error.code] ?? 'preview.office.errors.failed', { defaultValue: error.message }) : '';
  return (
    <div className='h-full w-full flex flex-col bg-1 overflow-hidden' aria-busy={phase !== 'ready' && phase !== 'error'}>
      {updating && (
        <div role={error ? 'alert' : 'status'} className='px-16px py-10px text-12px bg-2 flex items-center gap-12px'>
          <span>{error ? t('preview.office.updateFailed', { message: errorMessage }) : t('preview.office.updating')}</span>
          {error?.retryable && <Button size='small' onClick={() => setRetry((value) => value + 1)}>{t('common.retry')}</Button>}
        </div>
      )}
      <div className='flex-1 min-h-0 relative'>
        {visibleFrames.map((frame) => (
          <iframe
            key={frame.requestId}
            ref={(element) => { if (element) frames.current.set(frame.requestId, element); else frames.current.delete(frame.requestId); }}
            src={frame.url}
            title={IFRAME_TITLE[docType]}
            sandbox='allow-scripts'
            referrerPolicy='no-referrer'
            aria-hidden={displayed?.requestId !== frame.requestId}
            inert={displayed?.requestId !== frame.requestId}
            className={`absolute inset-0 w-full h-full border-0 bg-1 ${displayed?.requestId !== frame.requestId ? 'opacity-0 pointer-events-none' : ''}`}
          />
        ))}
        {!displayed && (
          <div className='h-full w-full flex items-center justify-center'>
            <div className='flex flex-col items-center gap-12px max-w-400px text-center'>
              {error ? <span role='alert' className='text-13px text-danger'>{errorMessage}</span> : <><Spin size={32} /><span role='status' className='text-13px text-t-secondary'>{t(`preview.office.${phase}`)}</span></>}
              {error?.retryable && <Button size='small' type='primary' onClick={() => setRetry((value) => value + 1)}>{t('common.retry')}</Button>}
            </div>
          </div>
        )}
      </div>
    </div>
  );
};

export default OfficeArtifactViewer;
