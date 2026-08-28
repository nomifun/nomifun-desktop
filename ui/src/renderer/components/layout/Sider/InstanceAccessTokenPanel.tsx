/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { webui } from '@/common/adapter/ipcBridge';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import { copyText } from '@/renderer/utils/ui/clipboard';
import { Alert, Button, Popconfirm, Spin, Tooltip } from '@arco-design/web-react';
import { Caution, CheckOne, Copy, Delete, Key } from '@icon-park/react';
import React, { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

/** Mint, inspect and revoke the single NomiFun Desktop Remote access token. */
const InstanceAccessTokenPanel: React.FC = () => {
  const { t } = useTranslation();
  const [message, messageHolder] = useArcoMessage();
  const [configured, setConfigured] = useState<boolean | null>(null);
  const [statusLoading, setStatusLoading] = useState(true);
  const [minting, setMinting] = useState(false);
  const [revoking, setRevoking] = useState(false);
  const [plaintext, setPlaintext] = useState<string | null>(null);
  const [warning, setWarning] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    webui.instanceAccessToken.status
      .invoke()
      .then((result) => {
        if (alive) setConfigured(result.configured);
      })
      .catch((error) => {
        console.error('Instance access-token status failed:', error);
        if (alive) setConfigured(null);
      })
      .finally(() => {
        if (alive) setStatusLoading(false);
      });
    return () => {
      alive = false;
    };
  }, []);

  const friendlyError = useCallback(
    (error: unknown, fallback: string) =>
      isBackendHttpError(error) && error.backendMessage ? error.backendMessage : fallback,
    []
  );

  const handleCopy = useCallback(
    (text: string) => {
      copyText(text)
        .then(() => message.success(t('common.copySuccess')))
        .catch((error) => console.error('[InstanceAccessToken] Copy failed:', error));
    },
    [message, t]
  );

  const handleMint = useCallback(async () => {
    setMinting(true);
    try {
      const result = await webui.instanceAccessToken.mint.invoke();
      setPlaintext(result.token);
      setWarning(result.warning ?? null);
      setConfigured(true);
      message.success(t('settings.webui.instanceToken.minted'));
    } catch (error) {
      console.error('Mint instance access token failed:', error);
      message.error(friendlyError(error, t('settings.webui.instanceToken.mintFailed')));
    } finally {
      setMinting(false);
    }
  }, [friendlyError, message, t]);

  const handleRevoke = useCallback(async () => {
    setRevoking(true);
    try {
      await webui.instanceAccessToken.revoke.invoke();
      setConfigured(false);
      setPlaintext(null);
      setWarning(null);
      message.success(t('settings.webui.instanceToken.revoked'));
    } catch (error) {
      console.error('Revoke instance access token failed:', error);
      message.error(friendlyError(error, t('settings.webui.instanceToken.revokeFailed')));
    } finally {
      setRevoking(false);
    }
  }, [friendlyError, message, t]);

  return (
    <div className='flex flex-col gap-8px'>
      {messageHolder}
      <div className='flex items-center gap-6px px-2px'>
        <Key theme='outline' size='14' className='shrink-0 text-primary-6' />
        <span className='text-12px font-500 text-t-tertiary'>{t('settings.webui.instanceToken.title')}</span>
        <span className='ml-auto shrink-0'>
          {statusLoading ? (
            <Spin size={14} />
          ) : configured ? (
            <span className='inline-flex items-center gap-3px whitespace-nowrap text-12px text-success'>
              <CheckOne theme='filled' size='13' fill='currentColor' />
              {t('settings.webui.instanceToken.statusActive')}
            </span>
          ) : (
            <span className='whitespace-nowrap text-12px text-t-tertiary'>
              {t('settings.webui.instanceToken.statusNone')}
            </span>
          )}
        </span>
      </div>
      <div className='px-2px text-12px leading-relaxed text-t-tertiary'>
        {t('settings.webui.instanceToken.desc')}
      </div>

      {plaintext && (
        <div className='flex flex-col gap-4px'>
          <div className='inline-flex min-w-0 items-center gap-8px rd-100px border border-solid border-[rgba(var(--primary-6),0.4)] bg-primary-1 px-10px py-4px'>
            <Key theme='outline' size='14' className='shrink-0 text-primary-6' />
            <span className='flex-1 truncate font-mono text-13px text-t-primary'>{plaintext}</span>
            <Tooltip content={t('common.copy')}>
              <Button type='text' size='mini' className='inline-flex !h-24px rd-100px !px-6px' onClick={() => handleCopy(plaintext)}>
                <Copy size={14} />
              </Button>
            </Tooltip>
          </div>
          <div className='px-2px text-12px leading-relaxed text-warning'>
            {t('settings.webui.instanceToken.shownOnceHint')}
          </div>
        </div>
      )}

      {warning && (
        <Alert
          type='warning'
          showIcon
          icon={<Caution theme='outline' size='15' fill='currentColor' />}
          content={<span className='text-12px leading-relaxed'>{warning}</span>}
          className='!rd-10px !py-6px'
        />
      )}

      <div className='flex items-center gap-8px'>
        <Button type='primary' size='small' long loading={minting} disabled={revoking || statusLoading} onClick={() => void handleMint()}>
          <span className='inline-flex items-center gap-4px'>
            <Key theme='outline' size='14' fill='currentColor' />
            {configured
              ? t('settings.webui.instanceToken.regenerate')
              : t('settings.webui.instanceToken.generate')}
          </span>
        </Button>
        {configured && (
          <Popconfirm
            position='top'
            title={t('settings.webui.instanceToken.revokeConfirm')}
            okText={t('settings.webui.instanceToken.revoke')}
            cancelText={t('common.cancel')}
            onOk={() => void handleRevoke()}
          >
            <Button type='outline' status='danger' size='small' loading={revoking} disabled={minting}>
              <span className='inline-flex items-center gap-4px'>
                <Delete theme='outline' size='14' fill='currentColor' />
                {t('settings.webui.instanceToken.revoke')}
              </span>
            </Button>
          </Popconfirm>
        )}
      </div>
    </div>
  );
};

export default InstanceAccessTokenPanel;
