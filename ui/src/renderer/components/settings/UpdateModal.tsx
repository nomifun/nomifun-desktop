/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useMemo, useRef, useState } from 'react';
import { Button, Progress, Message } from '@arco-design/web-react';
import { CheckOne, Download } from '@icon-park/react';
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
import './UpdateModal.css';

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

type UpdatePresentation = 'compact' | 'detail';

const formatVersion = (version?: string | null) => {
  if (!version) return '-';
  return version.startsWith('v') ? version : `v${version}`;
};

const BAIDU_RELEASE_MIRROR_URL = 'https://pan.baidu.com/s/5GPonoJNrwJ7GciBSDgXLaA';
const PRODUCT_WEBSITE_URL = 'https://www.nomifun.com';
const PRODUCT_CONTACT_URL = 'https://www.nomifun.com/contact';
const GITHUB_ISSUES_PAGE = 'https://github.com/nomifun/nomifun-tauri/issues';
const GITHUB_RELEASES_PAGE = 'https://github.com/nomifun/nomifun-tauri/releases/latest';

const UpdateModal: React.FC = () => {
  const { t } = useTranslation();
  const [visible, setVisible] = useState(false);
  const [presentation, setPresentation] = useState<UpdatePresentation>('compact');
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
    setPresentation('compact');
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

  const openProductContact = () => {
    void ipcBridge.shell.openExternal.invoke(PRODUCT_CONTACT_URL).catch((error) => {
      console.error('Failed to open product contact page:', error);
    });
  };

  const openGitHubIssues = () => {
    void ipcBridge.shell.openExternal.invoke(GITHUB_ISSUES_PAGE).catch((error) => {
      console.error('Failed to open GitHub issues:', error);
    });
  };

  const checkForUpdates = async () => {
    // Every check result belongs to the compact surface. In particular, retrying
    // from expanded error details must not revive the removed legacy checking
    // modal while the request is in flight.
    setPresentation('compact');
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
    // The compact card owns progress presentation. This does not alter the
    // existing download path; it only collapses the optional detail dialog.
    setPresentation('compact');
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

  const showDetails = () => {
    if (status === 'available' || status === 'error') {
      setPresentation('detail');
    }
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

  /* 同方向的 border-t-solid：无方向的 border-solid 会给四边都上样式，另外三边没有
     宽度类会回落到 medium≈3px。bg-fill-1/60 也是死写法（bg-fill-N 规则以 $ 锚定，
     斜杠透明度让它一条都匹配不上），改用 color-mix 拿到同样的 60% 填充。
     Same-direction style + a fill that actually compiles: `bg-fill-1/60` matched
     no rule at all, so this strip had no background of any kind.
     注意 updateDisclaimer.test.ts 用正则要求类名字符串紧跟在括号后面。 */
  const disclaimer = renderDisclaimer(
    'shrink-0 border-t border-t-solid border-[rgba(var(--warning-6),0.18)] bg-[color-mix(in_srgb,var(--color-fill-1)_60%,transparent)] px-20px py-8px text-left update-modal__disclaimer'
  );

  const renderCompactContent = () => {
    const availableVersion = updateInfo?.version || autoUpdateInfo?.version || '';
    const canDismiss = status !== 'installing';
    const title =
      status === 'downloading' || status === 'downloaded' || status === 'installing'
        ? t('update.compactProgressTitle')
        : status === 'error'
          ? t('update.errorTitle')
          : t('update.compactTitle');

    return (
      <section
        className={`update-compact-card update-compact-card--${status}`}
        role='dialog'
        aria-modal='false'
        aria-label={title}
      >
        <div className='update-compact-card__header'>
          <div className='update-compact-card__title'>{title}</div>
          {canDismiss && (
            <button
              type='button'
              className='update-compact-card__close'
              onClick={handleClose}
              aria-label={t('common.close')}
            >
              <span aria-hidden='true'>×</span>
            </button>
          )}
        </div>

        {status === 'checking' && (
          <div className='update-compact-card__status' aria-live='polite'>
            <span className='update-compact-card__spinner' aria-hidden='true' />
            <span>{t('update.checking')}</span>
          </div>
        )}

        {status === 'upToDate' && (
          <div className='update-compact-card__status' aria-live='polite'>
            <span className='update-compact-card__status-icon update-compact-card__status-icon--success'>
              <CheckOne theme='filled' size='15' />
            </span>
            <div>
              <div className='update-compact-card__status-label'>{t('update.upToDateTitle')}</div>
              <div className='update-compact-card__muted'>
                {t('update.currentVersion', { version: formatVersion(currentVersion) })}
              </div>
            </div>
          </div>
        )}

        {status === 'available' && (
          <>
            <div className='update-compact-card__version-row'>
              <span>{formatVersion(currentVersion)}</span>
              <span className='update-compact-card__version-arrow' aria-hidden='true'>
                →
              </span>
              <strong>{formatVersion(availableVersion)}</strong>
              <button type='button' className='update-compact-card__detail-link' onClick={showDetails}>
                {t('update.viewDetails')}
              </button>
            </div>
            <div className='update-compact-card__actions'>
              <Button type='primary' size='mini' onClick={startDownload} className='update-compact-card__action'>
                {t('update.updateNow')}
              </Button>
              <Button size='mini' onClick={handleClose} className='update-compact-card__action'>
                {t('update.later')}
              </Button>
            </div>
          </>
        )}

        {status === 'downloading' && (
          <div aria-live='polite'>
            <div className='update-compact-card__download-heading'>
              <span>{formatVersion(availableVersion || downloadVersionRef.current)}</span>
              <span className='update-compact-card__muted'>{t('update.downloadingTitle')}</span>
            </div>
            <Progress
              percent={progress.percent}
              status='normal'
              showText={false}
              strokeWidth={6}
              className='update-compact-card__progress'
            />
            <div className='update-compact-card__progress-meta'>
              <span>
                {progress.total > 0
                  ? `${formatSize(progress.transferred)} / ${formatSize(progress.total)}`
                  : formatSize(progress.transferred)}
              </span>
              <span>{progress.speed || `${Math.round(progress.percent)}%`}</span>
            </div>
          </div>
        )}

        {status === 'downloaded' && (
          <>
            <div className='update-compact-card__status' aria-live='polite'>
              <span className='update-compact-card__status-icon update-compact-card__status-icon--success'>
                <CheckOne theme='filled' size='15' />
              </span>
              <div>
                <div className='update-compact-card__status-label'>{t('update.readyToInstall')}</div>
                <div className='update-compact-card__muted'>{t('update.readyToInstallDesc')}</div>
              </div>
            </div>
            <div className='update-compact-card__actions'>
              <Button
                type='primary'
                size='mini'
                onClick={quitAndInstall}
                className='update-compact-card__action update-compact-card__action--wide'
              >
                {t('update.installNow')}
              </Button>
              <Button size='mini' onClick={handleClose} className='update-compact-card__action'>
                {t('update.later')}
              </Button>
            </div>
          </>
        )}

        {status === 'installing' && (
          <div className='update-compact-card__status' aria-live='polite'>
            <span className='update-compact-card__spinner' aria-hidden='true' />
            <div>
              <div className='update-compact-card__status-label'>
                {installPhase === 'installing' ? t('update.installingTitle') : t('update.preparingInstallTitle')}
              </div>
              <div className='update-compact-card__muted'>
                {installPhase === 'installing' ? t('update.installingDesc') : t('update.preparingInstallDesc')}
              </div>
            </div>
          </div>
        )}

        {status === 'success' && (
          <>
            <div className='update-compact-card__status' aria-live='polite'>
              <span className='update-compact-card__status-icon update-compact-card__status-icon--success'>
                <CheckOne theme='filled' size='15' />
              </span>
              <div className='update-compact-card__status-label'>{t('update.downloadCompleteTitle')}</div>
            </div>
            <div className='update-compact-card__actions'>
              <Button size='mini' onClick={showInFolder} className='update-compact-card__action'>
                {t('update.showInFolder')}
              </Button>
              <Button type='primary' size='mini' onClick={openFile} className='update-compact-card__action'>
                {t('update.openFile')}
              </Button>
            </div>
          </>
        )}

        {status === 'error' && (
          <>
            <div className='update-compact-card__error-message' aria-live='polite'>
              {errorMsg}
            </div>
            <div className='update-compact-card__error-tools'>
              <button type='button' className='update-compact-card__detail-link' onClick={showDetails}>
                {t('update.viewDetails')}
              </button>
              <div className='update-compact-card__actions'>
                <Button
                  type='primary'
                  size='mini'
                  onClick={checkForUpdates}
                  className='update-compact-card__action'
                >
                  {t('common.retry')}
                </Button>
                <Button size='mini' onClick={handleClose} className='update-compact-card__action'>
                  {t('update.later')}
                </Button>
              </div>
            </div>
          </>
        )}
      </section>
    );
  };

  const renderDetailContent = () => {
    switch (status) {
      case 'available':
        return (
          <div className='flex h-full min-h-0 flex-col'>
            <div className='update-modal__available'>
              <div className='update-modal__metadata'>
                <div className='update-modal__meta-row'>
                  <span className='update-modal__meta-label'>{t('update.versionLabel')}</span>
                  <strong>{updateInfo?.version || autoUpdateInfo?.version || '-'}</strong>
                </div>
                <div className='update-modal__meta-row'>
                  <span className='update-modal__meta-label'>{t('update.sizeLabel')}</span>
                  <span>
                    {updateInfo?.recommendedAsset?.size ? formatSize(updateInfo.recommendedAsset.size) : '-'}
                  </span>
                </div>
                <div className='update-modal__details-label'>{t('update.detailsLabel')}</div>
              </div>

              {!hasCompatibleManualAsset && !autoUpdateAvailable && (
                <div className='update-modal__compatibility-warning'>
                  {t('update.noCompatibleAssetManual')}
                </div>
              )}

              <div
                className='update-modal__release-scroll custom-scrollbar'
                tabIndex={0}
                role='region'
                aria-label={t('update.detailsLabel')}
              >
                {updateInfo?.name && <div className='update-modal__release-title'>{updateInfo.name}</div>}
                {updateInfo?.body || autoUpdateInfo?.releaseNotes ? (
                  <MarkdownView allowHtml compact fontSize='13px' lineHeight='20px'>
                    {updateInfo?.body || autoUpdateInfo?.releaseNotes || ''}
                  </MarkdownView>
                ) : (
                  <div className='text-13px text-t-tertiary italic'>{t('update.noReleaseNotes')}</div>
                )}

                <div className='update-modal__source-note'>
                  <div>{t('update.downloadSourceHint')}</div>
                  <div>
                    {t('update.baiduMirrorHint')}{' '}
                    <button
                      type='button'
                      onClick={openBaiduReleaseMirror}
                      title={BAIDU_RELEASE_MIRROR_URL}
                      className='update-modal__inline-link'
                    >
                      {t('update.baiduMirrorLink')}
                    </button>
                    <span aria-hidden='true'> · </span>
                    <button
                      type='button'
                      onClick={openProductWebsite}
                      title={PRODUCT_WEBSITE_URL}
                      className='update-modal__inline-link'
                    >
                      {t('update.productWebsiteLink')}
                    </button>
                  </div>
                </div>
              </div>
            </div>
            {disclaimer}
            <div className='update-modal__actions'>
              {!hasCompatibleManualAsset && !autoUpdateAvailable && releasePageUrl ? (
                <Button type='primary' size='small' onClick={openReleasePage} className='update-modal__action'>
                  {t('update.goToRelease')}
                </Button>
              ) : autoUpdateAvailable ? (
                <Button type='primary' size='small' onClick={startDownload} className='update-modal__action'>
                  {t('update.downloadButton')}
                </Button>
              ) : (
                <Button type='primary' size='small' onClick={startDownload} className='update-modal__action'>
                  {t('update.downloadButton')}
                </Button>
              )}
              {renderBaiduManualDownloadButton('update-modal__action')}
            </div>
          </div>
        );

      case 'error':
        return (
          <div className='update-modal__error'>
            <div className='update-modal__error-content'>
              <div className='update-modal__error-message' aria-live='polite'>
                {errorMsg}
              </div>
              <div className='update-modal__error-links'>
                <div className='update-modal__error-link-row'>
                  <span>{t('update.feedbackIssueLabel')}</span>
                  <button type='button' onClick={openGitHubIssues} className='update-modal__error-link'>
                    {GITHUB_ISSUES_PAGE}
                  </button>
                </div>
                <div className='update-modal__error-link-row'>
                  <span>{t('update.contactUsLabel')}</span>
                  <button type='button' onClick={openProductContact} className='update-modal__error-link'>
                    {PRODUCT_CONTACT_URL}
                  </button>
                </div>
                <div className='update-modal__error-link-row'>
                  <span>{t('update.releasePageLabel')}</span>
                  <button type='button' onClick={openReleasePage} className='update-modal__error-link'>
                    {releasePageUrl || GITHUB_RELEASES_PAGE}
                  </button>
                </div>
                <div className='update-modal__error-link-row'>
                  <span>{t('update.productWebsiteHint')}</span>
                  <button type='button' onClick={openProductWebsite} className='update-modal__error-link'>
                    {PRODUCT_WEBSITE_URL}
                  </button>
                </div>
              </div>
            </div>
            {disclaimer}
            <div className='update-modal__actions update-modal__actions--error'>
              <Button
                size='small'
                onClick={checkForUpdates}
                className='update-modal__action update-modal__error-action--primary'
              >
                {t('common.retry')}
              </Button>
              <Button size='small' onClick={openBaiduReleaseMirror} className='update-modal__action'>
                {t('settings.baiduManualDownload')}
              </Button>
            </div>
          </div>
        );
    }
    return null;
  };

  const isAvailableDialog = status === 'available';
  const isErrorDialog = status === 'error';
  const canShowDetails = isAvailableDialog || isErrorDialog;

  if (!visible) return null;

  if (presentation === 'compact' || !canShowDetails) {
    return <div className='update-compact-card-host'>{renderCompactContent()}</div>;
  }

  return (
    <NomiModal
      visible
      onCancel={handleClose}
      alignCenter
      className={
        isAvailableDialog
          ? 'nomifun-update-modal nomifun-update-modal--available'
          : isErrorDialog
            ? 'nomifun-update-modal nomifun-update-modal--error'
            : 'nomifun-update-modal'
      }
      style={isAvailableDialog ? { width: '720px' } : { width: '640px' }}
      header={{
        title: isAvailableDialog ? t('update.availableTitle') : t('update.errorDialogTitle'),
        showClose: true,
        className: 'update-modal__header',
      }}
      footer={{ render: () => null }}
      contentStyle={{
        height: isAvailableDialog ? 'min(350px, calc(100vh - 144px))' : 'auto',
        padding: 0,
        overflow: 'hidden',
      }}
    >
      <div className='flex h-full w-full min-h-0 flex-col'>{renderDetailContent()}</div>
    </NomiModal>
  );
};

export default UpdateModal;
