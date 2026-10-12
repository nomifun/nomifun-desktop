import { ipcBridge } from '@/common';
import { appHosting } from '@/common/adapter/appHostingBridge';
import type { AppImportKind, HostedApp } from '@/common/types/appHosting';
import { Alert, Button, Input, Radio } from '@arco-design/web-react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { isDesktopShell } from '@/renderer/utils/platform';
import { appErrorMessage } from './appHostingModel';
import styles from './AppsPage.module.css';

export default function AppImportForm({ app, developmentBusy = false, onFileOperationChange, onImported }: { app?: HostedApp; developmentBusy?: boolean; onFileOperationChange?: (busy: boolean) => void; onImported: (app: HostedApp) => void }) {
  const { t } = useTranslation();
  const [source, setSource] = useState('');
  const [name, setName] = useState(app?.name ?? '');
  const [kind, setKind] = useState<AppImportKind>(app?.import_kind ?? 'source');
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
    if (pending.current || developmentBusy || (!source.trim() && !archive)) return;
    pending.current = true; setBusy(true); setError(''); onPendingChange.current?.(true);
    try {
      const result = app
        ? archive ? await appHosting.importIntoZip(app.id, archive, kind, app.name) : await appHosting.importInto.invoke({ id: app.id, source: source.trim(), name: app.name, import_kind: kind })
        : archive ? await appHosting.importZip(archive, kind, name.trim()) : await appHosting.import.invoke({ source: source.trim(), name: name.trim() || undefined, import_kind: kind });
      if (mounted.current) onImported(result);
    } catch (e) { if (mounted.current) setError(appErrorMessage(e)); }
    finally {
      pending.current = false;
      if (mounted.current) { setBusy(false); onPendingChange.current?.(false); }
    }
  };
  const browse = async (folder: boolean) => {
    try {
      const selected = await ipcBridge.dialog.showOpen.invoke({ properties: folder ? ['openDirectory'] : ['openFile'], ...(folder ? {} : { filters: [{ name: 'ZIP', extensions: ['zip'] }] }) });
      if (selected?.[0]) { setSource(selected[0]); setArchive(undefined); }
    } catch (e) { setError(appErrorMessage(e)); }
  };
  return <section className={`${styles.settings} ${styles.inspectorCard}`} aria-label={t('appHosting.import.title')}>
    <h3>{t('appHosting.import.title')}</h3><p>{t('appHosting.import.manualHint')}</p>
    {error && <Alert type='error' content={error} />}
    {!app && <label>{t('appHosting.name')}<Input value={name} onChange={setName} aria-label={t('appHosting.name')} /></label>}
    <Radio.Group value={kind} onChange={setKind} aria-label={t('appHosting.import.kind')}>
      <Radio value='source'>{t('appHosting.import.source')}</Radio><Radio value='artifact'>{t('appHosting.import.artifact')}</Radio>
    </Radio.Group>
    <label>{t('appHosting.import.hostPath')}<Input value={source} onChange={(value) => { setSource(value); setArchive(undefined); }} aria-label={t('appHosting.import.hostPath')} placeholder={t('appHosting.import.pathPlaceholder')} /></label>
    {desktop && <div className={styles.actions}><Button size='small' onClick={() => void browse(true)}>{t('appHosting.import.folder')}</Button><Button size='small' onClick={() => void browse(false)}>{t('appHosting.import.zip')}</Button></div>}
    <label>{t('appHosting.import.uploadZip')}<input type='file' accept='.zip,application/zip' aria-label={t('appHosting.import.uploadZip')}
      onChange={(event) => { setArchive(event.target.files?.[0]); setSource(''); }} /></label>
    {archive && <p>{archive.name}</p>}
    <p className={styles.muted}>{t(desktop ? 'appHosting.import.hint' : 'appHosting.import.remoteHint')}</p>
    {developmentBusy && <p className={styles.muted}>{t('appHosting.workspace.workspaceBusy')}</p>}
    <Button type='primary' loading={busy} disabled={developmentBusy || !source.trim() && !archive} onClick={() => void importApp()}>{t('appHosting.import.action')}</Button>
  </section>;
}
