/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { copyText } from '@/renderer/utils/ui/clipboard';
import { Button, Message, Select, Tooltip } from '@arco-design/web-react';
import { Copy, Refresh } from '@icon-park/react';
import React, { Suspense, useId } from 'react';
import { useTranslation } from 'react-i18next';
import ConnectionHint from './ConnectionHint';
import styles from './WebuiQrLoginPanel.module.css';

const QRCodeSVGLazy = React.lazy(async () => {
  const mod = await import('qrcode.react');
  return { default: mod.QRCodeSVG };
});

export interface WebuiQrLoginPanelProps {
  baseUrls: string[];
  selectedBaseUrl: string;
  onBaseUrlChange: (url: string) => void;
  qrUrl: string | null;
  expiresAt: number | null;
  loading: boolean;
  onRefresh: () => void;
}

export default function WebuiQrLoginPanel({
  baseUrls,
  selectedBaseUrl,
  onBaseUrlChange,
  qrUrl,
  expiresAt,
  loading,
  onRefresh,
}: WebuiQrLoginPanelProps) {
  const { t } = useTranslation();
  const panelId = useId();
  const titleId = `${panelId}-title`;
  const addressId = `${panelId}-address`;
  const addressLabelId = `${panelId}-address-label`;
  const addressHelpId = `${panelId}-address-help`;
  const loadingPlaceholder = <span className={styles.placeholder} role='status'>{t('common.loading')}</span>;

  const handleCopy = async () => {
    if (!qrUrl || loading) return;
    try {
      await copyText(qrUrl);
      Message.success(t('common.copySuccess'));
    } catch {
      Message.error(t('common.copyFailed'));
    }
  };

  return <section className={styles.panel} aria-labelledby={titleId} data-testid='webui-qr-login-panel'>
    <h3 id={titleId} className={styles.title}>{t('settings.webui.quickLoginTitle')}</h3>
    <div className={styles.content}>
      <div className={styles.qrFrame} aria-busy={loading}>
        {loading ? loadingPlaceholder : qrUrl ? (
          <Suspense fallback={loadingPlaceholder}>
            <QRCodeSVGLazy value={qrUrl} size={140} level='M' role='img' aria-label={t('settings.webui.quickLoginTitle')} />
          </Suspense>
        ) : <span className={styles.placeholder} role='status'>{t('settings.webui.qrGenerateFailed')}</span>}
      </div>

      <div className={styles.details}>
        <p className={styles.hint}>{t('settings.webui.quickLoginHint')}</p>
        {baseUrls.length > 0 && <div className={styles.addressField}>
          <div className={styles.addressLabel}>
            <label id={addressLabelId} htmlFor={addressId}>{t('settings.webui.loginAddress')}</label>
            <ConnectionHint label={t('settings.webui.loginAddress')} content={t('settings.webui.qrAddressPickerDesc')} id={addressHelpId} />
          </div>
          {baseUrls.length > 1 ? (
            <Select
              id={addressId}
              aria-labelledby={addressLabelId}
              aria-describedby={addressHelpId}
              size='small'
              value={selectedBaseUrl || baseUrls[0]}
              onChange={onBaseUrlChange}
              className={styles.addressSelect}
            >
              {baseUrls.map((url) => <Select.Option key={url} value={url}>{url}</Select.Option>)}
            </Select>
          ) : <code id={addressId} className={styles.singleAddress} aria-labelledby={addressLabelId}>{selectedBaseUrl || baseUrls[0]}</code>}
        </div>}

        <div className={styles.actions}>
          <Button size='small' type='primary' className={styles.copyButton} icon={<Copy size={14} fill='currentColor' />} disabled={!qrUrl || loading} onClick={() => void handleCopy()}>
            {t('settings.webui.copyQrLink')}
          </Button>
          <Tooltip content={t('settings.webui.refreshQr')} trigger={['hover', 'focus']}>
            <Button
              size='small'
              type='text'
              className={styles.refreshButton}
              aria-label={t('settings.webui.refreshQr')}
              disabled={loading}
              onClick={onRefresh}
              icon={<Refresh size={15} fill='currentColor' />}
            />
          </Tooltip>
        </div>
        {expiresAt !== null && <span className={styles.expiry}>
          {t('settings.webui.qrExpires', { time: new Date(expiresAt).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' }) })}
        </span>}
      </div>
    </div>
  </section>;
}
