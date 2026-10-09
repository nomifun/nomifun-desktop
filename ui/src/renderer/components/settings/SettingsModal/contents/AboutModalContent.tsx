/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */
import { Button, Divider } from '@arco-design/web-react';
import { Download, Github, Refresh, Right } from '@icon-park/react';
import React, { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { isDesktopShell, openExternalUrl } from '@/renderer/utils/platform';
import { httpGet } from '@/common/adapter/httpBridge';
import PageHeader from '@/renderer/components/layout/PageHeader';
import { NOMIFUN_PUBLIC_LINKS } from './FeedbackReportModal';
import './AboutModalContent.css';

// The public health endpoint supplies the same version in desktop and WebUI.
const healthGet = httpGet<{ version?: string }>('/health');

const AboutModalContent: React.FC = () => {
  const { t } = useTranslation();
  const isDesktop = isDesktopShell();
  const [appVersion, setAppVersion] = useState('');
  useEffect(() => {
    let alive = true;
    healthGet.invoke().then((health) => {
      if (alive && health?.version) setAppVersion(health.version);
    }).catch((error) => console.error('Failed to read app version:', error));
    return () => { alive = false; };
  }, []);

  const openLink = async (url: string) => {
    try { await openExternalUrl(url); } catch (error) { console.error('Failed to open link:', error); }
  };
  const checkUpdate = () => {
    window.dispatchEvent(new CustomEvent('nomifun-open-update-modal', { detail: { source: 'about' } }));
  };
  const linkItems = [
    { title: t('settings.helpDocumentation'), url: NOMIFUN_PUBLIC_LINKS.officialWebsite },
    { title: t('settings.updateLog'), url: NOMIFUN_PUBLIC_LINKS.releases },
    { title: t('settings.bugReport'), url: NOMIFUN_PUBLIC_LINKS.issues },
    { title: t('settings.contactMe'), url: NOMIFUN_PUBLIC_LINKS.contact },
    { title: t('settings.officialWebsite'), url: NOMIFUN_PUBLIC_LINKS.officialWebsite },
  ];

  return <div className='about-page'>
    <PageHeader className='about-page__heading' title='NomiFun' description={t('settings.appDescription')} />
    <div className='about-page__version'>
      <span>v{appVersion || '—'}</span>
      <Button type='text' size='small' aria-label='GitHub' icon={<Github theme='outline' size={20} />}
        onClick={() => void openLink(NOMIFUN_PUBLIC_LINKS.repository)} />
    </div>
    {isDesktop && <div className='about-page__updates'>
      <Button type='primary' onClick={checkUpdate} icon={<Refresh theme='outline' size={14} />}>{t('settings.checkForUpdates')}</Button>
      <Button onClick={() => void openLink(NOMIFUN_PUBLIC_LINKS.baiduPan)} icon={<Download theme='outline' size={14} />}>{t('settings.baiduManualDownload')}</Button>
    </div>}
    <Divider className='my-12px' />
    <div className='about-page__links'>
      {linkItems.map((item) => <button key={item.title} type='button' className='about-page__link' onClick={() => void openLink(item.url)}>
        <span><span className='about-page__link-title'>{item.title}</span><span className='about-page__link-url'>{item.url}</span></span>
        <Right theme='outline' size={16} />
      </button>)}
    </div>
  </div>;
};

export default AboutModalContent;
