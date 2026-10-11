/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import type { StorageRelocationResponse } from '@/common/protocolBindings/StorageRelocationResponse';
import DirectorySelectionModal from '@/renderer/components/settings/DirectorySelectionModal';
import { formatBytes } from '@/renderer/utils/file/formatBytes';
import { isDesktopShell } from '@/renderer/utils/platform';
import { Alert, Button, Input, Modal, Spin } from '@arco-design/web-react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

interface StorageRelocationModalProps {
  visible: boolean;
  currentDir: string;
  status?: StorageRelocationResponse;
  onStatusChange: (status: StorageRelocationResponse) => void;
  onClose: () => void;
}

const requestMessage = (error: unknown): string => {
  if (isBackendHttpError(error) && error.backendMessage) return error.backendMessage;
  return error instanceof Error ? error.message : String(error);
};

export default function StorageRelocationModal({ visible, currentDir, status, onStatusChange, onClose }: StorageRelocationModalProps) {
  const { t } = useTranslation();
  const [targetDir, setTargetDir] = useState('');
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);
  const [error, setError] = useState<string | null>(null);
  const [showDirectorySelector, setShowDirectorySelector] = useState(false);
  const [loadingStatus, setLoadingStatus] = useState(false);
  const [statusVerified, setStatusVerified] = useState(false);

  useEffect(() => {
    if (!visible) return;
    setError(null);
    setTargetDir(status?.target_dir ?? '');
    setLoadingStatus(true);
    setStatusVerified(false);
    let active = true;
    ipcBridge.storageRelocation.status.invoke().then((next) => {
      if (!active) return;
      onStatusChange(next);
      setStatusVerified(true);
      setTargetDir(next.target_dir ?? '');
    }).catch((caught) => {
      if (active) setError(requestMessage(caught));
    }).finally(() => { if (active) setLoadingStatus(false); });
    return () => { active = false; };
    // Opening always verifies the durable plan; updates are handled by the
    // exact prepare/start/cancel response, not another reset of the form.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [visible]);

  const run = async (action: () => Promise<void>) => {
    if (busyRef.current || loadingStatus || !statusVerified) return;
    busyRef.current = true;
    setBusy(true);
    setError(null);
    try { await action(); } catch (caught) { setError(requestMessage(caught)); }
    finally { busyRef.current = false; setBusy(false); }
  };

  const activePlan = Boolean(status?.operation_id && !['idle', 'completed', 'canceled'].includes(status.phase));
  const ready = Boolean(status?.operation_id && status.blockers.length === 0
    && (status.phase === 'prepared' || (status.phase === 'failed' && status.cancellable)));
  const disabled = busy || loadingStatus || !statusVerified;
  const dismissDisabled = busy || loadingStatus;

  const retryStatus = async () => {
    if (busyRef.current || loadingStatus) return;
    setLoadingStatus(true);
    setError(null);
    try {
      const next = await ipcBridge.storageRelocation.status.invoke();
      onStatusChange(next);
      setTargetDir(next.target_dir ?? '');
      setStatusVerified(true);
    } catch (caught) { setError(requestMessage(caught)); }
    finally { setLoadingStatus(false); }
  };

  const prepare = (path: string) => run(async () => {
    if (!path.trim()) return;
    const next = await ipcBridge.storageRelocation.prepare.invoke({ target_dir: path });
    onStatusChange(next);
    setTargetDir(next.target_dir ?? path);
  });

  const chooseDirectory = () => {
    if (disabled || activePlan) return;
    if (!isDesktopShell()) { setShowDirectorySelector(true); return; }
    void run(async () => {
      const paths = await ipcBridge.dialog.showOpen.invoke({
        defaultPath: targetDir || currentDir,
        properties: ['openDirectory', 'createDirectory'],
      });
      if (!paths?.[0]) return;
      setTargetDir(paths[0]);
      const next = await ipcBridge.storageRelocation.prepare.invoke({ target_dir: paths[0] });
      onStatusChange(next);
      setTargetDir(next.target_dir ?? paths[0]);
    });
  };

  const cancel = () => run(async () => {
    if (!status?.operation_id || !status.cancellable) return;
    const next = await ipcBridge.storageRelocation.cancel.invoke({ operation_id: status.operation_id });
    onStatusChange(next);
    setTargetDir('');
    onClose();
  });

  const restart = async () => {
    if (!isDesktopShell()) return;
    try { await ipcBridge.application.restart.invoke(); }
    catch (caught) { throw new Error(`${t(status?.restart_required && !status.cancellable
      ? 'settings.storageRelocation.recoveryRequired' : 'settings.storageRelocation.restartFailed')} ${requestMessage(caught)}`); }
  };

  const start = () => run(async () => {
    if (!ready || !status?.operation_id) return;
    const next = await ipcBridge.storageRelocation.start.invoke({ operation_id: status.operation_id });
    onStatusChange(next);
    if (next.restart_required) await restart();
  });

  const phaseText = (() => {
    switch (status?.phase) {
      case 'prepared': return t('settings.storageRelocation.prepared');
      case 'awaiting_restart': return t('settings.storageRelocation.awaitingRestart');
      case 'copying': return t('settings.storageRelocation.copying');
      case 'verifying': return t('settings.storageRelocation.verifying');
      case 'completed': return t('settings.storageRelocation.completed');
      case 'failed': return status.cancellable
        ? t('settings.storageRelocation.failed') : t('settings.storageRelocation.recoveryRequired');
      case 'canceled': return t('settings.storageRelocation.canceled');
      default: return activePlan ? t('settings.storageRelocation.unknownPhase') : '';
    }
  })();

  return <>
    <Modal visible={visible} title={t('settings.storageRelocation.title')} style={{ width: 600 }}
      closable={!dismissDisabled} maskClosable={!dismissDisabled} escToExit={!dismissDisabled} onCancel={dismissDisabled ? undefined : onClose}
      footer={<div className='flex justify-end gap-8px'>
        {status?.cancellable && activePlan && <Button disabled={disabled} onClick={() => void cancel()}>{t('settings.storageRelocation.cancel')}</Button>}
        <Button disabled={dismissDisabled} onClick={onClose}>{t('settings.storageRelocation.close')}</Button>
        {!statusVerified && !loadingStatus && <Button onClick={() => void retryStatus()}>{t('settings.storageRelocation.retryCheck')}</Button>}
        {ready && <Button type='primary' loading={busy} disabled={disabled} onClick={() => void start()}>{isDesktopShell()
          ? t('settings.storageRelocation.saveAndRestart') : t('settings.storageRelocation.savePlan')}</Button>}
        {status?.restart_required && isDesktopShell() && <Button type='primary' loading={busy} disabled={disabled} onClick={() => void run(restart)}>{t('settings.storageRelocation.restartNow')}</Button>}
      </div>}>
      <Spin loading={loadingStatus} style={{ display: 'block' }}>
        <div className='flex flex-col gap-12px' style={{ maxHeight: 'calc(100vh - 220px)', overflowY: 'auto' }}>
          <div className='text-13px text-t-secondary'>{t('settings.storageRelocation.description')}</div>
          <div><div className='text-13px text-t-secondary'>{t('settings.storageRelocation.current')}</div>
            <div className='text-13px text-t-primary' style={{ overflowWrap: 'anywhere' }}>{status?.current_dir || currentDir}</div></div>
          <label className='text-13px text-t-secondary' htmlFor='storage-relocation-target'>{t('settings.storageRelocation.target')}</label>
          <div className='flex gap-8px'>
            <Input id='storage-relocation-target' value={targetDir} onChange={setTargetDir} disabled={disabled || activePlan}
              placeholder={t('settings.storageRelocation.targetPlaceholder')} onPressEnter={() => { if (!activePlan) void prepare(targetDir); }} />
            <Button disabled={disabled || activePlan} onClick={chooseDirectory}>{t('settings.storageRelocation.chooseDirectory')}</Button>
          </div>
          {!activePlan && <div className='flex justify-end'><Button disabled={disabled || !targetDir.trim()} loading={busy}
            onClick={() => void prepare(targetDir)}>{t('settings.storageRelocation.check')}</Button></div>}
          {phaseText && <div role='status' className='text-13px text-t-primary'>{phaseText}</div>}
          {activePlan && <div className='text-13px text-t-secondary'>{t('settings.storageRelocation.total', {
            size: status?.total_bytes ? formatBytes(status.total_bytes) : '0 B', count: status?.file_count ?? 0,
          })}</div>}
          {status?.blockers.map((blocker) => <Alert key={blocker.code} type='error' content={blocker.message} />)}
          {status?.warnings.map((warning, index) => <Alert key={index} type='warning' content={warning} />)}
          {ready && <Alert type='info' content={isDesktopShell()
            ? t('settings.storageRelocation.confirmDescription') : t('settings.storageRelocation.confirmDescriptionWeb')} />}
          {status?.restart_required && <Alert type='info' content={!status.cancellable
            ? t('settings.storageRelocation.recoveryRequired') : isDesktopShell()
              ? t('settings.storageRelocation.restartNotice') : t('settings.storageRelocation.restartServer')} />}
          {status?.error && <Alert type='error' content={status.error} />}
          {error && <Alert type='error' content={error} />}
        </div>
      </Spin>
    </Modal>
    {showDirectorySelector && <DirectorySelectionModal visible onCancel={() => setShowDirectorySelector(false)} onConfirm={(paths) => {
      setShowDirectorySelector(false);
      if (paths?.[0]) { setTargetDir(paths[0]); void prepare(paths[0]); }
    }} />}
  </>;
}
