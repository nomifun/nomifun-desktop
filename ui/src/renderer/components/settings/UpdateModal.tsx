/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useMemo, useRef, useState } from 'react';
import { Button, Progress, Message } from '@arco-design/web-react';
import { CheckOne, Download, FolderOpen, Refresh, CloseOne, Install } from '@icon-park/react';
import { ipcBridge } from '@/common';
import NomiModal from '@/renderer/components/base/NomiModal';
import MarkdownView from '@/renderer/components/Markdown';
import type {
  UpdateDownloadProgressEvent,
  UpdateReleaseInfo,
  AutoUpdateInstallPhase,
  AutoUpdateStatus,
} from '@/common/update/updateTypes';
import { useTranslation } from 'react-i18next';
import { getUpdateErrorMessageKey } from './updateErrorMessage';
import { deriveUpdateStatus, shouldApplyDownloadEvent } from './deriveUpdateStatus';
import { reportNoUpdateAvailable, reportUpdateAvailable } from '@renderer/hooks/system/useUpdateAvailability';

type UpdateStatus =
  | 'checking'
  | 'upToDate'
  | 'available'
  | 'downloading'
  | 'downloaded'
  | 'installing'
  | 'success'
  | 'error';

type UpdateInfo = UpdateReleaseInfo;

const BAIDU_RELEASE_MIRROR_URL = 'https://pan.baidu.com/s/5GPonoJNrwJ7GciBSDgXLaA';
const PRODUCT_WEBSITE_URL = 'https://www.nomifun.com';
const GITHUB_RELEASES_PAGE = 'https://github.com/nomifun/nomifun-tauri/releases/latest';

const UpdateModal: React.FC = () => {
  const { t } = useTranslation();
  const [visible, setVisible] = useState(false);
  const [status, setStatus] = useState<UpdateStatus>('checking');
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
  const [currentVersion, setCurrentVersion] = useState<string>('');
  const [downloadId, setDownloadId] = useState<string | null>(null);
  const [progress, setProgress] = useState({ percent: 0, speed: '', total: 0, transferred: 0 });
  const [installPhase, setInstallPhase] = useState<AutoUpdateInstallPhase>('preparing');
  const [errorMsg, setErrorMsg] = useState('');
  const [downloadPath, setDownloadPath] = useState('');
  const [releasePageUrl, setReleasePageUrl] = useState('');
  // Whether electron-updater auto-update is available (determined automatically, not user-controllable)
  const [autoUpdateAvailable, setAutoUpdateAvailable] = useState(false);
  const [autoUpdateInfo, setAutoUpdateInfo] = useState<{ version: string; releaseNotes?: string } | null>(null);
  const installRequestedRef = useRef(false);
  // A download already in flight. `startDownload` had no re-entrancy guard, so a
  // second trigger started a second flow whose independent byte counter fought
  // the first one over the single progress bar.
  const downloadRequestedRef = useRef(false);
  // Version the in-flight download belongs to, so a progress frame from a
  // superseded flow cannot repaint the bar.
  const downloadVersionRef = useRef<string | null>(null);

  const resetState = () => {
    setStatus('checking');
    setUpdateInfo(null);
    setCurrentVersion('');
    setDownloadId(null);
    setProgress({ percent: 0, speed: '', total: 0, transferred: 0 });
    setInstallPhase('preparing');
    installRequestedRef.current = false;
    downloadRequestedRef.current = false;
    downloadVersionRef.current = null;
    setErrorMsg('');
    setDownloadPath('');
    setReleasePageUrl('');
    setAutoUpdateAvailable(false);
    setAutoUpdateInfo(null);
  };

  const includePrerelease = useMemo(() => localStorage.getItem('update.includePrerelease') === 'true', [visible]);
  const hasCompatibleManualAsset = Boolean(updateInfo?.recommendedAsset);

  const openReleasePage = () => {
    const target = releasePageUrl || GITHUB_RELEASES_PAGE;
    void ipcBridge.shell.openExternal.invoke(target).catch((error) => {
      console.error('Failed to open release page:', error);
    });
  };

  const openBaiduReleaseMirror = () => {
    void ipcBridge.shell.openExternal.invoke(BAIDU_RELEASE_MIRROR_URL).catch((error) => {
      console.error('Failed to open Baidu release mirror:', error);
    });
  };

  const openProductWebsite = () => {
    void ipcBridge.shell.openExternal.invoke(PRODUCT_WEBSITE_URL).catch((error) => {
      console.error('Failed to open product website:', error);
    });
  };

  const checkForUpdates = async () => {
    setStatus('checking');
    try {
      // Try auto-update (electron-updater) first
      let autoUpdateOk = false;
      let retainedVersion: string | null = null;
      let packageState: import('@/common/adapter/tauriShell').TauriUpdatePackageState | null = null;
      let packageVersion: string | null = null;
      // Captured locally: setAutoUpdateInfo below only lands on the NEXT render,
      // so reading that state back in this same pass would see a stale version.
      let autoUpdateVersion = '';
      try {
        const res = await ipcBridge.autoUpdate.check.invoke({ includePrerelease });
        retainedVersion = res?.data?.retainedVersion ?? null;
        packageState = res?.data?.packageState ?? null;
        packageVersion = res?.data?.packageVersion ?? null;
        if (res?.success && res.data?.updateInfo) {
          autoUpdateOk = true;
          autoUpdateVersion = res.data.updateInfo.version;
          reportUpdateAvailable(res.data.updateInfo.version);
          setAutoUpdateInfo({
            version: res.data.updateInfo.version,
            releaseNotes: res.data.updateInfo.releaseNotes,
          });
        } else if (res?.msg) {
          console.warn('Auto-update check failed, using manual mode:', res.msg);
        }
      } catch (err) {
        console.warn('Auto-update check error, using manual mode:', err);
      }
      setAutoUpdateAvailable(autoUpdateOk);

      // Always run manual check for version info and release notes
      const res = await ipcBridge.update.check.invoke({ includePrerelease });
      if (!res?.success) {
        throw new Error(res?.msg || t('update.checkFailed'));
      }
      setCurrentVersion(res.data?.currentVersion || '');

      if (autoUpdateOk) {
        // Auto-update available — use manual check data for display only
        if (res.data?.latest) {
          setUpdateInfo(res.data.latest);
          setReleasePageUrl(res.data.latest.htmlUrl || '');
        }
        // The native slot is the only thing that knows whether bytes are already
        // retained or a download is still running; derive from it so a re-check
        // can happen at any time and always lands on the truth.
        const availableVersion = res.data?.latest?.version || autoUpdateVersion;
        const derived = deriveUpdateStatus({
          availableVersion,
          retainedVersion,
          slotState: packageState,
          slotVersion: packageVersion,
        });
        if (derived === 'downloading') {
          // Re-attach to the running download rather than re-arming Download.
          downloadRequestedRef.current = true;
          downloadVersionRef.current = packageVersion;
        }
        setStatus(derived);
        return;
      }

      // Manual mode
      if (res.data?.updateAvailable && res.data.latest) {
        reportUpdateAvailable(res.data.latest.version);
        setUpdateInfo(res.data.latest);
        setReleasePageUrl(res.data.latest.htmlUrl || '');
        if (!res.data.latest.recommendedAsset) {
          setErrorMsg(t('update.noCompatibleAssetManual'));
        }
        setStatus('available');
        return;
      }

      setUpdateInfo(res.data?.latest || null);
      setReleasePageUrl(res.data?.latest?.htmlUrl || '');
      reportNoUpdateAvailable();
      setStatus('upToDate');
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      console.error('Update check failed:', err);
      const errorMessageKey = getUpdateErrorMessageKey(msg);
      setErrorMsg(errorMessageKey === 'update.releaseFeedUnavailable' ? t(errorMessageKey) : msg || t(errorMessageKey));
      if (errorMessageKey === 'update.releaseFeedUnavailable') {
        setReleasePageUrl((url) => url || GITHUB_RELEASES_PAGE);
      }
      setStatus('error');
    }
  };

  const startDownload = async () => {
    if (downloadRequestedRef.current) return;
    if (!updateInfo && !autoUpdateAvailable) return;
    downloadRequestedRef.current = true;
    downloadVersionRef.current = updateInfo?.version || autoUpdateInfo?.version || null;
    setStatus('downloading');
    setProgress({ percent: 0, speed: '', total: 0, transferred: 0 });
    try {
      // Prefer the manual path so the URL is the CDN-rewritten asset.url.
      // Fall back to electron-updater (GitHub) only when the GitHub API manual check failed
      // but the yml-based auto-update check succeeded — a rare edge case.
      // 优先走手动路径（URL 是重写后的 CDN 地址）。仅当 GitHub API 失败但 electron-updater 检查成功时，
      // 回退到 electron-updater 的下载（走 GitHub），保证用户能升级。
      if (updateInfo?.recommendedAsset) {
        const asset = updateInfo.recommendedAsset;
        const res = await ipcBridge.update.download.invoke({
          url: asset.url,
          fallbackUrl: asset.fallbackUrl,
          file_name: asset.name,
        });
        if (!res?.success || !res.data) {
          throw new Error(res?.msg || t('update.downloadStartFailed'));
        }
        setDownloadId(res.data.downloadId);
        setDownloadPath(res.data.file_path);
        return;
      }

      if (autoUpdateAvailable) {
        const res = await ipcBridge.autoUpdate.download.invoke();
        if (!res?.success) {
          throw new Error(res?.msg || t('update.downloadStartFailed'));
        }
        // The native download is complete once this resolves (the status emitter
        // has already moved the UI to 'downloaded'), so the guard can be
        // released for a genuine future retry.
        downloadRequestedRef.current = false;
        return;
      }

      throw new Error(t('update.noCompatibleAssetManual'));
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      console.error('Download failed:', err);
      // A failed download must be retryable; only a LIVE download holds the guard.
      downloadRequestedRef.current = false;
      downloadVersionRef.current = null;
      setErrorMsg(msg);
      setStatus('error');
    }
  };

  const quitAndInstall = async () => {
    if (installRequestedRef.current) return;
    installRequestedRef.current = true;
    setInstallPhase('preparing');
    setProgress({ percent: 0, speed: '', total: 0, transferred: 0 });
    setErrorMsg('');
    setStatus('installing');
    try {
      await ipcBridge.autoUpdate.quitAndInstall.invoke();
    } catch (err: unknown) {
      installRequestedRef.current = false;
      const msg = err instanceof Error ? err.message : String(err);
      console.error('Install failed:', err);
      const messageKey = getUpdateErrorMessageKey(msg);
      Message.error(t(messageKey));
      if (messageKey === 'update.packageNoLongerReady') {
        // The native side no longer holds this package, so the 'downloaded'
        // screen would offer an Install button that can only fail again — and its
        // only other affordance is the manual mirror, not the re-download the
        // message asks for. Re-check instead: the status is then derived from the
        // slot and the user lands on a screen that can actually act.
        void checkForUpdates();
        return;
      }
      setStatus('downloaded');
    }
  };

  const formatSpeed = (bytesPerSecond: number) => {
    if (bytesPerSecond > 1024 * 1024) {
      return `${(bytesPerSecond / (1024 * 1024)).toFixed(1)} MB/s`;
    }
    return `${(bytesPerSecond / 1024).toFixed(1)} KB/s`;
  };

  const formatSize = (bytes: number) => {
    if (bytes > 1024 * 1024) {
      return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
    }
    return `${(bytes / 1024).toFixed(1)} KB`;
  };

  const handleOpenUpdateModal = () => {
    setVisible(true);
    if (installRequestedRef.current) return;
    // Always re-check, even while a download is running. Skipping the reset here
    // instead looked safer but disabled the ONLY recovery path: a download whose
    // invoke never settles (sleep / network switch) would leave the guard set and
    // the modal frozen for the rest of the session. checkForUpdates re-derives
    // 'downloading' from the native slot, so a live download is re-attached
    // rather than hidden behind a re-armed Download button.
    resetState();
    void checkForUpdates();
  };

  useEffect(() => {
    const removeOpenListener = ipcBridge.update.open.on(handleOpenUpdateModal);
    window.addEventListener('nomifun-open-update-modal', handleOpenUpdateModal);

    return () => {
      removeOpenListener();
      window.removeEventListener('nomifun-open-update-modal', handleOpenUpdateModal);
    };
  }, []);

  // Listen for auto-update status events (e.g. from startup check)
  useEffect(() => {
    const removeListener = ipcBridge.autoUpdate.status.on((evt: AutoUpdateStatus) => {
      if (!evt) return;
      // Discard every frame from a superseded download flow, terminal ones
      // included: a stale completion used to flip the modal to the Install screen
      // while the live download was still mid-transfer.
      if (
        (evt.status === 'downloading' || evt.status === 'downloaded' || evt.status === 'error') &&
        !shouldApplyDownloadEvent(evt.version, downloadVersionRef.current)
      ) {
        return;
      }

      switch (evt.status) {
        case 'checking':
          break;
        case 'available':
          reportUpdateAvailable(evt.version);
          setAutoUpdateAvailable(true);
          setAutoUpdateInfo({
            version: evt.version || '',
            releaseNotes: evt.releaseNotes,
          });
          setStatus('available');
          setVisible(true);
          break;
        case 'not-available':
          reportNoUpdateAvailable();
          setStatus('upToDate');
          break;
        case 'downloading':
          // Ignore a series from a superseded download: two live flows keep
          // separate byte counters, and letting both write here is what made one
          // bar flip between two unrelated progress readings.
          if (evt.progress) {
            setProgress({
              percent: Math.round(evt.progress.percent),
              speed: formatSpeed(evt.progress.bytesPerSecond),
              total: evt.progress.total,
              transferred: evt.progress.transferred,
            });
          }
          break;
        case 'downloaded':
          downloadRequestedRef.current = false;
          setStatus('downloaded');
          break;
        case 'installing':
          setStatus('installing');
          setInstallPhase(evt.installPhase || 'preparing');
          if (evt.progress) {
            setProgress({
              percent: Math.round(evt.progress.percent),
              speed: formatSpeed(evt.progress.bytesPerSecond),
              total: evt.progress.total,
              transferred: evt.progress.transferred,
            });
          }
          break;
        case 'error':
          setStatus('error');
          setErrorMsg(evt.error || t('update.downloadFailed'));
          break;
      }
    });

    return () => {
      removeListener();
    };
  }, [t]);

  useEffect(() => {
    const removeProgressListener = ipcBridge.update.downloadProgress.on((evt: UpdateDownloadProgressEvent) => {
      if (!evt) return;
      if (!downloadId || evt.downloadId !== downloadId) return;

      setProgress({
        percent: Math.round(evt.percent ?? 0),
        speed: formatSpeed(evt.bytesPerSecond ?? 0),
        total: evt.totalBytes ?? 0,
        transferred: evt.receivedBytes ?? 0,
      });

      if (evt.status === 'completed') {
        downloadRequestedRef.current = false;
        setStatus('success');
        if (evt.file_path) {
          setDownloadPath(evt.file_path);
        }
      } else if (evt.status === 'error' || evt.status === 'cancelled') {
        downloadRequestedRef.current = false;
        setStatus('error');
        setErrorMsg(evt.error || t('update.downloadFailed'));
      }
    });

    return () => {
      removeProgressListener();
    };
  }, [downloadId, t]);

  const handleClose = () => {
    if (installRequestedRef.current) return;
    setVisible(false);
  };

  const openFile = () => {
    if (!downloadPath) return;
    void ipcBridge.shell.openFile.invoke(downloadPath).catch((error) => {
      console.error('Failed to open file:', error);
    });
  };

  const showInFolder = () => {
    if (!downloadPath) return;
    void ipcBridge.shell.showItemInFolder.invoke(downloadPath).catch((error) => {
      console.error('Failed to show item in folder:', error);
    });
  };

  const renderDisclaimer = (className = '') => (
    <div className={`text-12px leading-18px text-warning-6 ${className}`}>{t('update.disclaimer')}</div>
  );

  const renderBaiduManualDownloadButton = (className = '') => (
    <Button
      size='small'
      onClick={openBaiduReleaseMirror}
      icon={<Download size='14' />}
      className={`!px-16px ${className}`}
    >
      {t('settings.baiduManualDownload')}
    </Button>
  );

  const renderContent = () => {
    switch (status) {
      case 'checking':
        return (
          <div className='flex flex-col items-center justify-center py-48px'>
            {/* 环形 spinner：border-3 是颜色类（--bg-3）而不是 3px 宽度，配上本仓库没有
                border-style 全局重置，两层圆环一条边都画不出来。宽度/样式/颜色分开写。
                `border-3` is a colour (--bg-3), not a width — with no border-style
                reset in this repo both rings painted nothing. */}
            <div className='w-48px h-48px mb-20px relative'>
              <div className='absolute inset-0 border-3px border-solid border-[var(--color-fill-3)] rounded-full' />
              <div className='absolute inset-0 border-3px border-solid border-primary border-t-transparent rounded-full animate-spin' />
            </div>
            <div className='text-15px text-t-primary font-500'>{t('update.checking')}</div>
            <div className='mt-16px'>{renderBaiduManualDownloadButton()}</div>
          </div>
        );

      case 'upToDate':
        return (
          <div className='flex flex-col items-center justify-center py-48px'>
            <div className='w-56px h-56px bg-[rgba(var(--success-6),0.12)] rounded-full flex items-center justify-center mb-20px'>
              <CheckOne theme='filled' size='28' fill='rgb(var(--success-6))' />
            </div>
            <div className='text-16px text-t-primary font-600 mb-8px'>{t('update.upToDateTitle')}</div>
            <div className='text-13px text-t-tertiary'>
              {t('update.currentVersion', { version: currentVersion || '-' })}
            </div>
            <div className='mt-16px'>{renderBaiduManualDownloadButton()}</div>
          </div>
        );

      case 'available':
        return (
          <div className='flex flex-col h-full'>
            {/* Version info header */}
            <div className='flex items-center justify-between px-24px py-16px border-b border-b-solid border-arco-2 bg-fill-1'>
              <div className='flex items-center gap-12px'>
                <div className='w-40px h-40px bg-[rgba(var(--primary-6),0.12)] rounded-10px flex items-center justify-center'>
                  <Download size='20' fill='rgb(var(--primary-6))' />
                </div>
                <div>
                  <div className='text-15px font-600 text-t-primary'>{t('update.availableTitle')}</div>
                  <div className='text-12px text-t-tertiary mt-2px'>
                    {currentVersion} →{' '}
                    <span className='text-primary-6 font-500'>
                      {updateInfo?.version || autoUpdateInfo?.version}
                    </span>
                  </div>
                </div>
              </div>
              <div className='flex flex-wrap items-center justify-end gap-8px'>
                {!hasCompatibleManualAsset && !autoUpdateAvailable && releasePageUrl ? (
                  <Button type='primary' size='small' onClick={openReleasePage} className='!px-16px'>
                    {t('update.goToRelease')}
                  </Button>
                ) : autoUpdateAvailable ? (
                  <Button type='primary' size='small' onClick={startDownload} className='!px-16px'>
                    {t('update.downloadButton')}
                  </Button>
                ) : (
                  <Button type='primary' size='small' onClick={startDownload} className='!px-16px'>
                    {t('update.downloadButton')}
                  </Button>
                )}
                {renderBaiduManualDownloadButton()}
              </div>
            </div>

            {!hasCompatibleManualAsset && !autoUpdateAvailable && (
              <div className='mx-24px mt-12px px-12px py-10px text-12px rounded-8px bg-[rgba(var(--warning-6),0.1)] text-warning-6'>
                {t('update.noCompatibleAssetManual')}
              </div>
            )}

            <div className='mx-24px mt-12px px-12px py-10px rounded-8px border border-solid border-[rgba(var(--primary-6),0.16)] bg-[rgba(var(--primary-6),0.06)] text-12px leading-18px text-t-secondary'>
              <div>{t('update.downloadSourceHint')}</div>
              <div className='mt-4px'>
                {t('update.baiduMirrorHint')}{' '}
                <button
                  type='button'
                  onClick={openBaiduReleaseMirror}
                  title={BAIDU_RELEASE_MIRROR_URL}
                  className='cursor-pointer border-0 bg-transparent p-0 text-12px leading-18px text-primary-6 underline-offset-2 hover:underline'
                >
                  {t('update.baiduMirrorLink')}
                </button>
              </div>
              <div className='mt-4px'>
                {t('update.productWebsiteHint')}{' '}
                <button
                  type='button'
                  onClick={openProductWebsite}
                  title={PRODUCT_WEBSITE_URL}
                  className='cursor-pointer border-0 bg-transparent p-0 text-12px leading-18px text-primary-6 underline-offset-2 hover:underline'
                >
                  {PRODUCT_WEBSITE_URL}
                </button>
              </div>
            </div>

            {/* Release notes content */}
            <div className='flex-1 min-h-0 overflow-y-auto px-24px py-16px custom-scrollbar'>
              {updateInfo?.name && <div className='text-14px font-500 text-t-primary mb-12px'>{updateInfo.name}</div>}
              {updateInfo?.body || autoUpdateInfo?.releaseNotes ? (
                <div className='text-13px text-t-secondary leading-relaxed'>
                  <MarkdownView allowHtml>{updateInfo?.body || autoUpdateInfo?.releaseNotes || ''}</MarkdownView>
                </div>
              ) : (
                <div className='text-13px text-t-tertiary italic'>{t('update.noReleaseNotes')}</div>
              )}
            </div>
          </div>
        );

      case 'downloading':
        return (
          <div className='flex flex-col items-center justify-center py-48px px-32px'>
            <div className='w-56px h-56px bg-[rgba(var(--primary-6),0.12)] rounded-full flex items-center justify-center mb-20px'>
              <Download size='24' fill='rgb(var(--primary-6))' className='animate-bounce' />
            </div>
            <div className='text-16px text-t-primary font-600 mb-20px'>{t('update.downloadingTitle')}</div>
            <div className='w-full max-w-320px'>
              <Progress
                percent={progress.percent}
                status='normal'
                showText={false}
                strokeWidth={6}
                className='!mb-12px'
              />
              <div className='flex justify-between text-12px text-t-tertiary'>
                <span>
                  {/* A missing Content-Length leaves the total unknown; showing
                      "12.4 MB / 0.0 KB" read as a broken download. */}
                  {progress.total > 0
                    ? `${formatSize(progress.transferred)} / ${formatSize(progress.total)}`
                    : formatSize(progress.transferred)}
                </span>
                <span className='text-primary-6 font-500'>{progress.speed}</span>
              </div>
            </div>
            <div className='mt-16px'>{renderBaiduManualDownloadButton()}</div>
          </div>
        );

      case 'downloaded':
        return (
          <div className='flex flex-col items-center justify-center py-48px px-32px'>
            <div className='w-56px h-56px bg-[rgba(var(--success-6),0.12)] rounded-full flex items-center justify-center mb-20px'>
              <CheckOne theme='filled' size='28' fill='rgb(var(--success-6))' />
            </div>
            <div className='text-16px text-t-primary font-600 mb-8px'>{t('update.readyToInstall')}</div>
            <div className='mb-24px text-13px text-warning-6 max-w-360px text-center'>
              {t('update.installWarning')}
            </div>
            <div className='flex flex-wrap justify-center gap-12px'>
              <Button
                type='primary'
                size='small'
                onClick={quitAndInstall}
                icon={<Install size='14' />}
                className='!px-16px'
              >
                {t('update.installNow')}
              </Button>
              {renderBaiduManualDownloadButton()}
            </div>
          </div>
        );

      case 'installing': {
        const isHandingOff = installPhase === 'installing';
        return (
          <div className='flex flex-col items-center justify-center py-48px px-32px'>
            <div className='w-56px h-56px mb-20px relative'>
              <div className='absolute inset-0 border-3px border-solid border-[var(--color-fill-3)] rounded-full' />
              <div className='absolute inset-0 border-3px border-solid border-primary border-t-transparent rounded-full animate-spin' />
              <div className='absolute inset-0 flex items-center justify-center'>
                <Install size='20' fill='rgb(var(--primary-6))' />
              </div>
            </div>
            <div aria-live='polite' className='text-center'>
              <div className='text-16px text-t-primary font-600 mb-8px'>
                {isHandingOff ? t('update.installingTitle') : t('update.preparingInstallTitle')}
              </div>
              <div className='text-13px text-t-tertiary max-w-360px'>
                {isHandingOff ? t('update.installingDesc') : t('update.preparingInstallDesc')}
              </div>
            </div>
          </div>
        );
      }

      case 'success':
        return (
          <div className='flex flex-col items-center justify-center py-48px px-32px'>
            <div className='w-56px h-56px bg-[rgba(var(--success-6),0.12)] rounded-full flex items-center justify-center mb-20px'>
              <CheckOne theme='filled' size='28' fill='rgb(var(--success-6))' />
            </div>
            <div className='text-16px text-t-primary font-600 mb-8px'>{t('update.downloadCompleteTitle')}</div>
            <div className='text-12px text-t-tertiary mb-24px text-center max-w-360px break-all line-clamp-2'>
              {downloadPath}
            </div>
            <div className='flex flex-wrap justify-center gap-12px'>
              <Button size='small' onClick={showInFolder} icon={<FolderOpen size='14' />} className='!px-16px'>
                {t('update.showInFolder')}
              </Button>
              <Button type='primary' size='small' onClick={openFile} className='!px-16px'>
                {t('update.openFile')}
              </Button>
              {renderBaiduManualDownloadButton()}
            </div>
          </div>
        );

      case 'error':
        return (
          <div className='flex flex-col items-center justify-center py-48px px-32px'>
            <div className='w-56px h-56px bg-[rgba(var(--danger-6),0.12)] rounded-full flex items-center justify-center mb-20px'>
              <CloseOne theme='filled' size='28' fill='rgb(var(--danger-6))' />
            </div>
            <div className='text-16px text-t-primary font-600 mb-8px'>{t('update.errorTitle')}</div>
            <div className='text-13px text-t-tertiary mb-24px text-center max-w-360px'>{errorMsg}</div>
            <div className='flex flex-wrap justify-center gap-12px'>
              <Button size='small' onClick={checkForUpdates} icon={<Refresh size='14' />} className='!px-16px'>
                {t('common.retry')}
              </Button>
              <Button type='primary' size='small' onClick={openReleasePage} className='!px-16px'>
                {t('update.goToRelease')}
              </Button>
              {renderBaiduManualDownloadButton()}
              <Button size='small' onClick={openProductWebsite} className='!px-16px'>
                {t('update.productWebsiteLink')}
              </Button>
            </div>
          </div>
        );
    }
  };

  return (
    <NomiModal
      visible={visible}
      onCancel={handleClose}
      size={status === 'available' ? 'medium' : 'small'}
      header={{
        title: t('update.modalTitle'),
        showClose: status !== 'installing',
      }}
      footer={{ render: () => null }}
      contentStyle={{
        height: status === 'available' ? '420px' : 'auto',
        padding: 0,
        overflow: 'hidden',
      }}
    >
      <div className='flex flex-col h-full w-full'>
        <div className='min-h-0 flex-1'>{renderContent()}</div>
        {/* 同方向的 border-t-solid：无方向的 border-solid 会给四边都上样式，另外三边没有
            宽度类会回落到 medium≈3px。bg-fill-1/60 也是死写法（bg-fill-N 规则以 $ 锚定，
            斜杠透明度让它一条都匹配不上），改用 color-mix 拿到同样的 60% 填充。
            Same-direction style + a fill that actually compiles: `bg-fill-1/60` matched
            no rule at all, so this strip had no background of any kind.
            注意 updateDisclaimer.test.ts 用正则要求类名字符串紧跟在括号后面。 */}
        {renderDisclaimer(
          'shrink-0 border-t border-t-solid border-[rgba(var(--warning-6),0.18)] bg-[color-mix(in_srgb,var(--color-fill-1)_60%,transparent)] px-20px py-10px text-center'
        )}
      </div>
    </NomiModal>
  );
};

export default UpdateModal;
