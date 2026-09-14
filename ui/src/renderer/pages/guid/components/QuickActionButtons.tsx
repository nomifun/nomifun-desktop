/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { webui } from '@/common/adapter/ipcBridge';
import { isDesktopShell } from '@/renderer/utils/platform';
import { BookOne, Comment, Down, Earth, Help, PlayOne, Refresh } from '@icon-park/react';
import React, { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { openExternalUrl } from '@/renderer/utils/platform';
import GuidPopover from './GuidPopover';
import styles from './GuidHomeUtilities.module.css';

type QuickActionButtonsProps = {
  onOpenBugReport: () => void;
};

type WebuiQuickStatus = 'checking' | 'running' | 'stopped' | 'error';

const WEBUI_STATUS_CACHE_TTL_MS = 3000;
let webuiStatusCache: {
  quickStatus: WebuiQuickStatus;
  at: number;
} | null = null;

const QuickActionButtons: React.FC<QuickActionButtonsProps> = ({
  onOpenBugReport,
}) => {
  const { t, i18n } = useTranslation();
  const navigate = useNavigate();
  const canCheckUpdate = isDesktopShell();
  const [helpOpen, setHelpOpen] = useState(false);
  const [webuiQuickStatus, setWebuiQuickStatus] = useState<WebuiQuickStatus>('checking');

  useEffect(() => {
    let alive = true;
    const loadStatus = async () => {
      const now = Date.now();
      if (webuiStatusCache && now - webuiStatusCache.at < WEBUI_STATUS_CACHE_TTL_MS) {
        setWebuiQuickStatus(webuiStatusCache.quickStatus);
        return;
      }

      try {
        const result = await webui.getStatus.invoke();
        if (!alive) return;
        if (result) {
          const quickStatus: WebuiQuickStatus = result.running ? 'running' : 'stopped';
          setWebuiQuickStatus(quickStatus);
          webuiStatusCache = { quickStatus, at: Date.now() };
          return;
        }
        setWebuiQuickStatus('error');
        webuiStatusCache = { quickStatus: 'error', at: Date.now() };
      } catch {
        if (!alive) return;
        setWebuiQuickStatus('error');
        webuiStatusCache = { quickStatus: 'error', at: Date.now() };
      }
    };

    void loadStatus();

    const unsubscribe = webui.statusChanged.on((payload) => {
      const nextQuickStatus: WebuiQuickStatus = payload.running ? 'running' : 'stopped';
      setWebuiQuickStatus(nextQuickStatus);
      webuiStatusCache = { quickStatus: nextQuickStatus, at: Date.now() };
    });

    return () => {
      alive = false;
      unsubscribe();
    };
  }, []);

  const handleOpenWebUI = useCallback(() => {
    void navigate('/open-capabilities');
  }, [navigate]);

  const handleCheckUpdate = useCallback(() => {
    window.dispatchEvent(new CustomEvent('nomifun-open-update-modal', { detail: { source: 'guid' } }));
  }, []);

  const webuiStatusLabel =
    webuiQuickStatus === 'running'
      ? t('guid.utilities.running')
      : webuiQuickStatus === 'checking'
        ? t('guid.utilities.checking')
        : webuiQuickStatus === 'error'
          ? t('guid.utilities.unavailable')
          : t('guid.utilities.stopped');
  const webuiIconColor =
    webuiQuickStatus === 'running'
      ? 'rgb(var(--success-6))'
      : webuiQuickStatus === 'checking'
        ? 'rgb(var(--primary-6))'
        : webuiQuickStatus === 'error'
          ? 'var(--color-text-3)'
          : 'var(--color-text-4)';

  const external = (url: string) => { setHelpOpen(false); void openExternalUrl(url); };
  const chinese = (i18n.resolvedLanguage || i18n.language).toLowerCase().startsWith('zh');
  return (
    <footer className={styles.utilities} aria-label={t('guid.utilities.title')}>
      <GuidPopover open={helpOpen} onOpenChange={setHelpOpen} label={t('guid.utilities.help')}
        triggerClassName={styles.utility} trigger={<><Help size={15} />{t('guid.utilities.help')}<Down size={12} /></>}>
        <div className={styles.helpMenu}>
          <button type='button' onClick={() => external('https://www.nomifun.com/docs')}><BookOne size={16} />{t('guid.utilities.docs')}</button>
          <button type='button' onClick={() => external(chinese ? 'https://www.bilibili.com/video/BV1kwKZ6UE5X/' : 'https://youtu.be/AsEToBDFR9s')}><PlayOne size={16} />{t('guid.utilities.video')}</button>
          <button type='button' onClick={() => external('https://www.nomifun.com/contact')}><Comment size={16} />{t('guid.utilities.community')}</button>
          <button type='button' onClick={() => { setHelpOpen(false); onOpenBugReport(); }}><Help size={16} />{t('conversation.welcome.quickActionFeedback')}</button>
        </div>
      </GuidPopover>
      <div className={styles.systemTools}>
        <button type='button' className={styles.utility} onClick={handleOpenWebUI}>
          <Earth size={14} style={{ color: webuiIconColor }} />WebUI<span className={styles.status}>{webuiStatusLabel}</span>
        </button>
        {canCheckUpdate && <button type='button' className={styles.utility} onClick={handleCheckUpdate}>
          <Refresh size={14} />{t('conversation.welcome.quickActionCheckUpdate')}
        </button>}
      </div>
    </footer>
  );
};

export default QuickActionButtons;
