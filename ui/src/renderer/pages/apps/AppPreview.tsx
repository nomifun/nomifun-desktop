import type { HostedApp } from '@/common/types/appHosting';
import { Button } from '@arco-design/web-react';
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { isDesktopShell, openExternalUrl } from '@/renderer/utils/platform';
import { canEmbedAppUrl, canOpenAppUrl, type AppPreviewFileChange } from './appHostingModel';
import styles from './AppsPage.module.css';

export default function AppPreview({ app, onPreview, previewBusy = false, previewDisabled = false, previewFileChange }: {
  app: HostedApp;
  onPreview?: () => Promise<HostedApp | undefined>;
  previewBusy?: boolean;
  previewDisabled?: boolean;
  previewFileChange?: AppPreviewFileChange;
}) {
  const { t } = useTranslation();
  const [requested, setRequested] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [refreshRevision, setRefreshRevision] = useState(0);
  const section = useRef<HTMLElement>(null);
  const viewport = useRef<HTMLDialogElement>(null);
  const returnFocus = useRef<HTMLElement | null>(null);
  const seenFileTurn = useRef<string | undefined>(undefined);
  const url = app.settings.web_url;
  const openable = canOpenAppUrl(url, isDesktopShell(), window.location.hostname);
  const embeddable = canEmbedAppUrl(url, isDesktopShell(), window.location.href);
  const running = app.state === 'running';
  useEffect(() => {
    const observationKey = previewFileChange ? `${previewFileChange.sessionId}:${previewFileChange.turnId}` : undefined;
    if (!previewFileChange || seenFileTurn.current === observationKey) return;
    seenFileTurn.current = observationKey;
    // Only a Host-verified completed mutation on the already shown process
    // reloads it. A new process/URL already owns a new iframe; a dismissed
    // preview stays dismissed. The observation is never a hosting operation.
    if (requested && running && embeddable && viewport.current
      && previewFileChange.appId === app.id
      && previewFileChange.pid === app.pid && previewFileChange.url === url) setRefreshRevision(value => value + 1);
  }, [previewFileChange, requested, running, embeddable, app.id, app.pid, url]);
  useEffect(() => {
    // The Host's successful preview operation is already the user's preview
    // request. Show that webpage without another click or another operation.
    // A stable process/URL keeps dismissal and the existing iframe state.
    if (app.preview && running && embeddable) setRequested(true);
  }, [app.id, app.pid, app.preview, url, running, embeddable]);
  const startPreview = async () => {
    setRequested(false);
    const result = await onPreview?.();
    if (result?.id === app.id && result.state === 'running' && canEmbedAppUrl(result.settings.web_url, isDesktopShell(), window.location.href)) {
      setRequested(true);
      setExpanded(true);
    }
  };
  // A stop hides the webpage without discarding its presentation preference.
  // Agent rebuilds may restart in production mode; explicit dismissal still
  // clears the preference, and a later run returns inline rather than modal.
  useEffect(() => { if (!running) setExpanded(false); }, [running]);
  useLayoutEffect(() => {
    const dialog = viewport.current;
    if (dialog) {
      // Native modality constrains background focus across the app iframe.
      // Closing and reopening this same node changes mode without reloading it.
      if (dialog.open) dialog.close();
      if (expanded) dialog.showModal(); else dialog.show();
    }
    if (!expanded && returnFocus.current) {
      if (returnFocus.current.isConnected) returnFocus.current.focus();
      else section.current?.focus();
      returnFocus.current = null;
    }
  }, [expanded, requested, embeddable, running]);
  return <section ref={section} tabIndex={-1} className={`${styles.previewSection} ${styles.inspectorCard}`} aria-label={t('appHosting.workspace.preview')}>
    <div className={styles.sectionHeading}><h3>{t('appHosting.workspace.preview')}</h3>
      <div className={styles.actions}>
        {running && openable && <Button type='text' size='small' onClick={() => void openExternalUrl(url!)}>{t('appHosting.workspace.openWindow')}</Button>}
        {requested && running && embeddable && <Button type='text' size='small' onClick={() => setRefreshRevision(value => value + 1)}>{t('appHosting.workspace.refreshPreview')}</Button>}
        {requested && running && embeddable && <Button type='text' size='small' onClick={(event) => { if (event.currentTarget instanceof HTMLElement) returnFocus.current = event.currentTarget; setExpanded(true); }}>{t('appHosting.workspace.expand')}</Button>}
      </div>
    </div>
    {requested && running && embeddable ? <>
      <dialog ref={viewport} className={`${styles.previewViewport} ${expanded ? styles.previewExpanded : ''}`}
        role={expanded ? 'dialog' : 'region'} aria-modal={expanded ? 'true' : undefined} aria-label={t('appHosting.workspace.previewTitle', { name: app.name })}
        onCancel={(event) => { event.preventDefault(); setExpanded(false); }} onClick={(event) => {
          if (!expanded || event.target !== event.currentTarget) return;
          const bounds = event.currentTarget.getBoundingClientRect();
          if (event.clientX < bounds.left || event.clientX > bounds.right || event.clientY < bounds.top || event.clientY > bounds.bottom) setExpanded(false);
        }}>
        <div className={styles.previewExpansionHeader} hidden={!expanded}><strong>{t('appHosting.workspace.previewTitle', { name: app.name })}</strong>
          <div className={styles.actions}>
            <Button size='small' onClick={() => setRefreshRevision(value => value + 1)}>{t('appHosting.workspace.refreshPreview')}</Button>
            <Button size='small' onClick={() => void openExternalUrl(url!)}>{t('appHosting.workspace.openWindow')}</Button>
            <Button size='small' onClick={() => setExpanded(false)}>{t('appHosting.workspace.collapse')}</Button>
          </div></div>
        <iframe key={`${app.id}:${app.pid}:${url}:${refreshRevision}`} src={url!} title={t('appHosting.workspace.previewTitle', { name: app.name })}
          allow="camera 'none'; microphone 'none'"
          className={styles.previewFrame} sandbox='allow-scripts allow-forms allow-same-origin' referrerPolicy='no-referrer' />
      </dialog>
      <Button type='text' size='small' onClick={() => { setRequested(false); setExpanded(false); }}>{t('appHosting.workspace.closePreview')}</Button></>
      : <div className={styles.previewPlaceholder}>
        <p>{!url ? t('appHosting.workspace.noWebpage') : !running ? t('appHosting.workspace.previewStopped') : !openable ? t('appHosting.remoteAddress') : !embeddable ? t('appHosting.workspace.sameOrigin') : t('appHosting.workspace.previewHint')}</p>
        {running && embeddable && <Button onClick={(event) => {
          if (event.currentTarget instanceof HTMLElement) returnFocus.current = event.currentTarget;
          setRequested(true); setExpanded(true);
        }}>{t('appHosting.workspace.loadPreview')}</Button>}
        {onPreview && (!running || !url) && <Button type='primary' disabled={previewDisabled} loading={previewBusy}
          onClick={(event) => {
            if (event.currentTarget instanceof HTMLElement) returnFocus.current = event.currentTarget;
            void startPreview();
          }}>{t('appHosting.actions.preview')}</Button>}
      </div>}
  </section>;
}
