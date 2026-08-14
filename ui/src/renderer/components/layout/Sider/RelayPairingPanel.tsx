/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Alert, Button, Input, Message, Tooltip } from '@arco-design/web-react';
import { Copy } from '@icon-park/react';
import React, { Suspense, useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { ipcBridge } from '@/common';
import type { IRelayPairingStatus } from '@/common/adapter/ipcBridge';
import { copyText } from '@/renderer/utils/ui/clipboard';

const QRCodeSVGLazy = React.lazy(async () => {
  const mod = await import('qrcode.react');
  return { default: mod.QRCodeSVG };
});

export const RELAY_PAIRING_PREFIX = 'nomifun-relay-pair:v1:';

export function isSafeRelayPairingUrl(value: string | undefined): value is string {
  return typeof value === 'string' && value.startsWith('nomi://pair');
}

const initialStatus: IRelayPairingStatus = { state: 'disconnected' };

const RelayPairingPanel: React.FC = () => {
  const { t } = useTranslation();
  const [envelope, setEnvelope] = useState('');
  const [status, setStatus] = useState<IRelayPairingStatus>(initialStatus);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refreshStatus = useCallback(async (): Promise<void> => {
    try {
      const next = await ipcBridge.relayPairing.getStatus.invoke();
      setStatus(next);
      setError(next.state === 'error' ? next.error || t('settings.webui.relayPairingFailed') : null);
    } catch {
      // A transient Tauri invocation failure should not erase the last useful
      // status. The next poll will retry.
    }
  }, [t]);

  useEffect(() => {
    let active = true;
    const poll = (): void => {
      void ipcBridge.relayPairing.getStatus
        .invoke()
        .then((next) => {
          if (!active) return;
          setStatus(next);
          setError(next.state === 'error' ? next.error || t('settings.webui.relayPairingFailed') : null);
        })
        .catch(() => {});
    };
    poll();
    const timer = window.setInterval(poll, 1_000);
    return () => {
      active = false;
      window.clearInterval(timer);
    };
  }, [t]);

  const connect = async (): Promise<void> => {
    const value = envelope.trim();
    if (!value.startsWith(RELAY_PAIRING_PREFIX)) {
      setError(t('settings.webui.relayPairingInvalid'));
      return;
    }

    setLoading(true);
    setError(null);
    setStatus({ state: 'connecting' });
    try {
      const next = await ipcBridge.relayPairing.bootstrap.invoke({
        pairingEnvelope: value,
      });
      if (next.state === 'connected' && !isSafeRelayPairingUrl(next.pairUrl)) {
        setStatus({ state: 'error' });
        setError(t('settings.webui.relayPairingFailed'));
        return;
      }
      setStatus(next);
      setEnvelope('');
      if (next.state === 'error') {
        setError(next.error || t('settings.webui.relayPairingFailed'));
      }
    } catch (cause) {
      setStatus({ state: 'error' });
      setError(
        typeof cause === 'string' && cause.trim()
          ? cause
          : t('settings.webui.relayPairingFailed')
      );
    } finally {
      setLoading(false);
    }
  };

  const runAction = async (
    action: 'restart' | 'stop' | 'disconnect'
  ): Promise<void> => {
    setLoading(true);
    setError(null);
    try {
      const next = await ipcBridge.relayPairing[action].invoke();
      setStatus(next);
      if (next.state === 'error') {
        setError(next.error || t('settings.webui.relayPairingFailed'));
      }
      if (action === 'restart') await refreshStatus();
    } catch (cause) {
      setError(
        typeof cause === 'string' && cause.trim()
          ? cause
          : t('settings.webui.relayPairingOperationFailed')
      );
    } finally {
      setLoading(false);
    }
  };

  const handleCopy = (value: string): void => {
    void copyText(value)
      .then(() => Message.success(t('common.copySuccess')))
      .catch(() => Message.error(t('common.copyFailed')));
  };

  const connected = status.state === 'connected' && isSafeRelayPairingUrl(status.pairUrl);

  return (
    <div className='flex flex-col gap-8px rd-10px border border-solid border-arco-2 bg-fill-1 px-10px py-10px'>
      <div className='text-13px font-500 text-t-primary'>
        {t('settings.webui.relayPairingTitle')}
      </div>
      <div className='text-12px text-t-secondary leading-relaxed'>
        {t('settings.webui.relayPairingDesc')}
      </div>
      <Input.Password
        value={envelope}
        onChange={setEnvelope}
        placeholder={t('settings.webui.relayPairingPlaceholder')}
        autoComplete='off'
        maxLength={8192}
        onPressEnter={() => void connect()}
      />
      <div className='flex items-center gap-8px'>
        <Button
          type='primary'
          size='small'
          loading={loading}
          disabled={!envelope.trim()}
          onClick={() => void connect()}
        >
          {t('settings.webui.relayPairingConnect')}
        </Button>
        <span className='text-12px text-t-tertiary'>
          {status.state === 'connecting'
            ? t('settings.webui.relayPairingConnecting')
            : connected
              ? t('settings.webui.relayPairingConnected')
              : status.state === 'error'
                ? t('settings.webui.relayPairingError')
                : t('settings.webui.relayPairingIdle')}
        </span>
      </div>
      {status.relay ? (
        <div className='flex flex-wrap items-center gap-6px'>
          {status.state === 'connected' || status.state === 'error' || status.state === 'disconnected' ? (
            <Button
              size='mini'
              loading={loading}
              onClick={() => void runAction('restart')}
            >
              {t('settings.webui.relayPairingRestart')}
            </Button>
          ) : null}
          {status.state === 'connected' || status.state === 'connecting' ? (
            <Button
              size='mini'
              loading={loading}
              onClick={() => void runAction('stop')}
            >
              {t('settings.webui.relayPairingStop')}
            </Button>
          ) : null}
          <Button
            size='mini'
            status='danger'
            loading={loading}
            onClick={() => void runAction('disconnect')}
          >
            {t('settings.webui.relayPairingDisconnect')}
          </Button>
        </div>
      ) : null}
      {error ? <Alert type='error' content={error} showIcon /> : null}
      {connected && status.pairUrl ? (
        <div className='flex flex-col items-center gap-8px border-t border-t-solid border-arco-2 pt-10px'>
          <div className='text-12px text-t-secondary'>
            {t('settings.webui.relayPairingResult')}
          </div>
          <div className='rd-8px bg-white p-8px'>
            <Suspense
              fallback={
                <div className='flex h-160px w-160px items-center justify-center'>
                  <span className='text-12px text-t-tertiary'>{t('common.loading')}</span>
                </div>
              }
            >
              <QRCodeSVGLazy value={status.pairUrl} size={160} level='M' />
            </Suspense>
          </div>
          <div className='flex w-full items-center gap-6px'>
            <code className='min-w-0 flex-1 truncate text-11px text-t-secondary'>
              {status.pairUrl}
            </code>
            <Tooltip content={t('settings.webui.relayPairingCopy')}>
              <Button
                type='text'
                size='mini'
                className='shrink-0'
                onClick={() => handleCopy(status.pairUrl as string)}
              >
                <Copy size={14} />
              </Button>
            </Tooltip>
          </div>
        </div>
      ) : null}
    </div>
  );
};

export default RelayPairingPanel;
