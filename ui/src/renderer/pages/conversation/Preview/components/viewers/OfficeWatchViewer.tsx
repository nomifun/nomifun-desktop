/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import { getBaseUrl, isBackendHttpError } from '@/common/adapter/httpBridge';
import { openExternalUrl } from '@/renderer/utils/platform';
import { Button, Spin } from '@arco-design/web-react';
import React, { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

export type DocType = 'ppt' | 'word' | 'excel';
type OfficeWatchErrorCode =
  | 'OFFICECLI_NOT_FOUND'
  | 'OFFICECLI_INSTALL_FAILED'
  | 'OFFICECLI_PORT_TIMEOUT'
  | 'OFFICECLI_START_FAILED'
  | 'PATH_OUTSIDE_SANDBOX';

const BRIDGE = {
  ppt: ipcBridge.pptPreview,
  word: ipcBridge.wordPreview,
  excel: ipcBridge.excelPreview,
} as const;

const PROXY_PATH: Record<DocType, string> = {
  ppt: '/api/ppt-proxy',
  word: '/api/office-watch-proxy',
  excel: '/api/office-watch-proxy',
};

const IFRAME_TITLE: Record<DocType, string> = {
  ppt: 'PPT Preview',
  word: 'Word Preview',
  excel: 'Excel Preview',
};

const I18N_KEYS = {
  ppt: {
    loading: 'preview.ppt.loading',
    installing: 'preview.ppt.installing',
    startFailed: 'preview.ppt.startFailed',
    installHint: 'preview.ppt.installHint',
  },
  word: {
    loading: 'preview.word.watch.loading',
    installing: 'preview.word.watch.installing',
    startFailed: 'preview.word.watch.startFailed',
    installHint: 'preview.word.watch.installHint',
  },
  excel: {
    loading: 'preview.excel.watch.loading',
    installing: 'preview.excel.watch.installing',
    startFailed: 'preview.excel.watch.startFailed',
    installHint: 'preview.excel.watch.installHint',
  },
} as const;

const OFFICE_ERROR_I18N_KEYS: Record<OfficeWatchErrorCode, string> = {
  OFFICECLI_NOT_FOUND: 'preview.office.errors.officecliNotFound',
  OFFICECLI_INSTALL_FAILED: 'preview.office.errors.installFailed',
  OFFICECLI_PORT_TIMEOUT: 'preview.office.errors.portTimeout',
  OFFICECLI_START_FAILED: 'preview.office.errors.startFailed',
  PATH_OUTSIDE_SANDBOX: 'preview.office.errors.outsideSandbox',
};

const OFFICECLI_INSTALL_URL = 'https://www.npmjs.com/package/officecli';

interface OfficeWatchViewerProps {
  docType: DocType;
  file_path?: string;
  content?: string;
  workspace?: string;
}

interface OfficeWatchErrorState {
  code?: OfficeWatchErrorCode;
  message: string;
}

export function resolveOfficeWatchUrl(
  url: string,
  capability: string,
  docType: DocType,
  baseUrl = getBaseUrl()
): string {
  const expectedPrefix = PROXY_PATH[docType];
  const match = url.match(new RegExp(`^${expectedPrefix}/([0-9a-f]{64})(/.*)$`));
  const parsed = new URL(url, 'http://nomifun.invalid');
  const canonicalRelativeUrl = `${parsed.pathname}${parsed.search}${parsed.hash}`;
  if (
    !match ||
    match[1] !== capability ||
    !/^[0-9a-f]{64}$/.test(capability) ||
    canonicalRelativeUrl !== url
  ) {
    throw new Error('Invalid Office preview capability URL');
  }

  return `${baseUrl.replace(/\/+$/, '')}${url}`;
}

function normalizeOfficeWatchErrorCode(error?: string | null): OfficeWatchErrorCode | undefined {
  switch (error) {
    case 'OFFICECLI_NOT_FOUND':
    case 'OFFICECLI_INSTALL_FAILED':
    case 'OFFICECLI_PORT_TIMEOUT':
    case 'OFFICECLI_START_FAILED':
    case 'PATH_OUTSIDE_SANDBOX':
      return error;
    default:
      return undefined;
  }
}

/**
 * Shared Office watch viewer.
 *
 * Launches an `officecli watch` child process via IPC, waits for the local
 * HTTP server to be ready, then renders it in an iframe. Cleans up the
 * process on unmount.
 *
 * Used by PptViewer, OfficeDocViewer, and ExcelViewer — each passes its
 * docType to select the correct IPC bridge, proxy path, and i18n keys.
 */
const OfficeWatchViewer: React.FC<OfficeWatchViewerProps> = ({ docType, file_path, workspace }) => {
  const { t } = useTranslation();
  const keys = I18N_KEYS[docType];

  const [watchUrl, setWatchUrl] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [status, setStatus] = useState<'starting' | 'installing'>('starting');
  const [error, setError] = useState<OfficeWatchErrorState | null>(null);
  const [retryKey, setRetryKey] = useState(0);

  useEffect(() => {
    const bridge = BRIDGE[docType];

    if (!file_path) {
      setLoading(false);
      setError({ message: t('preview.errors.missingFilePath') });
      return;
    }

    let cancelled = false;
    let activeCapability: string | null = null;

    const releaseCapability = (capability: string) =>
      bridge.stop.invoke({ capability }).catch(() => {});

    const unsubStatus = bridge.status.on((evt) => {
      if (cancelled) return;
      if (evt.state === 'installing') setStatus('installing');
      else if (evt.state === 'starting') setStatus('starting');
    });

    const start = async () => {
      setLoading(true);
      setStatus('starting');
      setError(null);
      try {
        const result = await bridge.start.invoke({ file_path, workspace });
        const errorCode = normalizeOfficeWatchErrorCode(result.error);
        if (errorCode) {
          if (result.capability) void releaseCapability(result.capability);
          if (!cancelled) {
            setError({
              code: errorCode,
              message: t(OFFICE_ERROR_I18N_KEYS[errorCode]),
            });
            setLoading(false);
          }
          return;
        }

        const url = result.url;
        const capability = result.capability;
        if (!url || !capability) {
          if (capability) void releaseCapability(capability);
          throw new Error(t(keys.startFailed));
        }
        let resolvedUrl: string;
        try {
          resolvedUrl = resolveOfficeWatchUrl(url, capability, docType);
        } catch (err) {
          void releaseCapability(capability);
          throw err;
        }

        activeCapability = capability;
        if (cancelled) {
          activeCapability = null;
          void releaseCapability(capability);
          return;
        }

        // Small delay to ensure the watch HTTP server is fully ready for the iframe
        await new Promise((r) => setTimeout(r, 300));
        if (!cancelled) {
          setWatchUrl(resolvedUrl);
          setLoading(false);
        }
      } catch (err) {
        if (!cancelled) {
          const backendCode = isBackendHttpError(err) ? normalizeOfficeWatchErrorCode(err.code) : undefined;
          if (backendCode) {
            setError({
              code: backendCode,
              message: t(OFFICE_ERROR_I18N_KEYS[backendCode]),
            });
            setLoading(false);
            return;
          }
          const msg = err instanceof Error ? err.message : t(keys.startFailed);
          setError({ message: msg });
          setLoading(false);
        }
      }
    };

    void start();

    return () => {
      cancelled = true;
      unsubStatus();
      if (activeCapability) {
        const capability = activeCapability;
        activeCapability = null;
        void releaseCapability(capability);
      }
    };
  }, [docType, file_path, retryKey, t, workspace]);

  if (loading) {
    return (
      <div className='h-full w-full flex items-center justify-center bg-bg-1'>
        <div className='flex flex-col items-center gap-12px'>
          <Spin size={32} />
          <span className='text-13px text-t-secondary'>
            {status === 'installing' ? t(keys.installing) : t(keys.loading)}
          </span>
        </div>
      </div>
    );
  }

  if (error) {
    const showRetry = error.code === 'OFFICECLI_INSTALL_FAILED' || error.code === 'OFFICECLI_PORT_TIMEOUT';
    const showInstallLink = error.code === 'OFFICECLI_NOT_FOUND';

    return (
      <div className='h-full w-full flex items-center justify-center bg-bg-1'>
        <div className='text-center max-w-400px'>
          <div className='text-16px text-danger mb-8px'>{error.message}</div>
          {!error.code && <div className='text-12px text-t-secondary mb-12px'>{t(keys.installHint)}</div>}
          {showInstallLink && (
            <div className='flex justify-center'>
              <Button type='text' size='small' onClick={() => void openExternalUrl(OFFICECLI_INSTALL_URL)}>
                {t('preview.office.installLinkText')}
              </Button>
            </div>
          )}
          {showRetry && (
            <div className='flex justify-center'>
              <Button size='small' type='primary' onClick={() => setRetryKey((value) => value + 1)}>
                {t('common.retry', { defaultValue: 'Retry' })}
              </Button>
            </div>
          )}
        </div>
      </div>
    );
  }

  if (!watchUrl) return null;

  return <iframe src={watchUrl} className='w-full h-full border-0 bg-bg-1' title={IFRAME_TITLE[docType]} />;
};

export default OfficeWatchViewer;
