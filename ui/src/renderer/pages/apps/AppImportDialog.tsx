import { ipcBridge } from '@/common';
import { appHosting } from '@/common/adapter/appHostingBridge';
import type { HostedApp } from '@/common/types/appHosting';
import { Alert, Button, Input, Modal } from '@arco-design/web-react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { isDesktopShell } from '@/renderer/utils/platform';
import { appErrorMessage } from './appHostingModel';
import styles from './AppsPage.module.css';

export default function AppImportDialog({ overwrite, developmentBusy = false, onFileOperationChange, onClose, onImported }: {
  overwrite: HostedApp; developmentBusy?: boolean; onFileOperationChange?: (busy: boolean) => void; onClose: () => void; onImported: (app: HostedApp) => void;
}) {
  const { t } = useTranslation();
  const [source, setSource] = useState('');
  const [archive, setArchive] = useState<File>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const pending = useRef(false);
  const mounted = useRef(false);
  const onPendingChange = useRef(onFileOperationChange);
  onPendingChange.current = onFileOperationChange;
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; if (pending.current) onPendingChange.current?.(false); };
  }, []);
  const desktop = isDesktopShell();
  const importApp = async () => {
    if (pending.current || developmentBusy) return;
    pending.current = true; setBusy(true); setError(''); onPendingChange.current?.(true);
    try {
      const app = archive ? await appHosting.overwriteZip(overwrite.id, archive) : await appHosting.overwrite.invoke({ id: overwrite.id, source: source.trim() });
      if (mounted.current) { onImported(app); onClose(); }
    } catch (e) { if (mounted.current) setError(appErrorMessage(e)); }
    finally {
      pending.current = false;
      if (mounted.current) { setBusy(false); onPendingChange.current?.(false); }
    }
  };
  const browse = async (folder: boolean) => {
    try {
      const selected = await ipcBridge.dialog.showOpen.invoke({ properties: folder ? ['openDirectory'] : ['openFile'],
        ...(folder ? {} : { filters: [{ name: 'ZIP', extensions: ['zip'] }] }) });
      if (selected?.[0]) { setSource(selected[0]); setArchive(undefined); }
    } catch (e) { setError(appErrorMessage(e)); }
  };
  return <Modal visible className={styles.modal} title={t('appHosting.overwrite.title')}
    style={{ width: 560 }} maskClosable={false} unmountOnExit onCancel={() => { if (!pending.current) onClose(); }} onOk={() => void importApp()}
    cancelButtonProps={{ disabled: busy }}
    confirmLoading={busy} okButtonProps={{ disabled: developmentBusy || !source.trim() && !archive }}
    okText={t('appHosting.overwrite.action')} cancelText={t('common.cancel')}>
    <div className={styles.settings}>
      <p>{t('appHosting.overwrite.hint')}</p>
      {developmentBusy && <p className={styles.muted}>{t('appHosting.workspace.workspaceBusy')}</p>}
      {error && <Alert type='error' content={error} />}
      <label>{t('appHosting.import.hostPath')}<Input value={source} onChange={(value) => { setSource(value); setArchive(undefined); }}
        aria-label={t('appHosting.import.hostPath')} placeholder={t('appHosting.import.pathPlaceholder')} /></label>
      {desktop && <div className={styles.actions}><Button onClick={() => void browse(true)}>{t('appHosting.import.folder')}</Button>
        <Button onClick={() => void browse(false)}>{t('appHosting.import.zip')}</Button></div>}
      <label>{t('appHosting.import.uploadZip')}<input type='file' accept='.zip,application/zip' aria-label={t('appHosting.import.uploadZip')}
        onChange={(event) => { setArchive(event.target.files?.[0]); setSource(''); }} /></label>
      {archive && <p>{archive.name}</p>}
      {!desktop && <p className={styles.muted}>{t('appHosting.import.remoteHint')}</p>}
    </div>
  </Modal>;
}
