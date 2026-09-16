import { useEffect, useRef, useState } from 'react';
import { Button, Popover } from '@arco-design/web-react';
import { Earth } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { systemBrowserClient, type SystemBrowserClient, type SystemBrowserChoice, type SystemBrowserSnapshot } from './client';
import styles from './SystemBrowserControl.module.css';

type Props = { conversationId: string; locked: boolean; available: boolean; client?: SystemBrowserClient };

export default function SystemBrowserControl({ conversationId, locked, available, client = systemBrowserClient }: Props) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [snapshot, setSnapshot] = useState<SystemBrowserSnapshot | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [choices, setChoices] = useState<SystemBrowserChoice[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState('');
  const [ownConnecting, setOwnConnecting] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const ownConnect = useRef<{ previous?: string; incarnation?: string; settled: boolean; cancelled: boolean } | null>(null);
  const cancelPending = useRef(false);
  const sequence = useRef(0);
  const pending = useRef(false);
  const current = useRef({ conversationId, locked, available });
  current.current = { conversationId, locked, available };
  useEffect(() => {
    setOpen(false); setSnapshot(null); setLoaded(false); setChoices(null); setBusy(false); setNotice(''); pending.current = false;
    ownConnect.current = null; cancelPending.current = false; setOwnConnecting(false); setCancelling(false);
    ++sequence.current;
    return () => { ++sequence.current; };
  }, [conversationId, client]);
  useEffect(() => { if (locked) setChoices(null); }, [locked]);

  const perform = async (action: 'refresh' | 'connect' | 'choices' | 'grant' | 'disconnect', choiceId?: string) => {
    if (pending.current || !current.current.available || current.current.conversationId !== conversationId) return;
    if (action !== 'refresh' && current.current.locked) return;
    if (action !== 'refresh' && action !== 'connect' && !snapshot) return;
    if (action === 'connect' && (!loaded || (snapshot && snapshot.state !== 'disconnected'))) return;
    if ((action === 'choices' || action === 'grant') && snapshot?.state !== 'connected') return;
    const issued = ++sequence.current;
    const isCurrent = () => issued === sequence.current && current.current.conversationId === conversationId;
    pending.current = true; setBusy(true); setNotice('');
    const attempt = action === 'connect' ? { previous: snapshot?.incarnation, incarnation: undefined as string | undefined, settled: false, cancelled: false } : null;
    if (attempt) { ownConnect.current = attempt; setOwnConnecting(true); }
    // Every inventory refresh invalidates old opaque choice tokens on the server.
    setChoices(null);
    try {
      if (action === 'choices') {
        const result = await client.choices(conversationId, snapshot!.incarnation);
        if (isCurrent() && !current.current.locked) setChoices(result.tabs);
      } else {
        const result = action === 'connect' ? await client.connect(conversationId, snapshot?.incarnation)
          : action === 'grant' ? await client.grant(conversationId, snapshot!.incarnation, choiceId!)
          : action === 'disconnect' ? await client.disconnect(conversationId, snapshot!.incarnation)
          : await client.snapshot(conversationId);
        if (attempt && result) attempt.incarnation = result.incarnation;
        if (isCurrent()) { setSnapshot(result); setLoaded(true); }
      }
    } catch {
      if (isCurrent()) {
        setNotice(t('browserWorkspace.systemBrowser.requestFailed'));
        setLoaded(false);
        if (action !== 'refresh') {
          // Recovery is observational only: do not replay a connect/grant/close.
          try {
            const result = await client.snapshot(conversationId);
            if (isCurrent()) { setSnapshot(result); setLoaded(true); }
          } catch { /* Keep actions fenced until an explicit refresh succeeds. */ }
        }
      }
    } finally {
      if (attempt && ownConnect.current === attempt) {
        attempt.settled = true; setOwnConnecting(false);
        if (!attempt.cancelled) ownConnect.current = null;
      }
      if (isCurrent()) { pending.current = false; setBusy(false); }
    }
  };
  const cancelConnect = async () => {
    const attempt = ownConnect.current;
    if (!attempt || cancelPending.current || !current.current.available || current.current.conversationId !== conversationId) return;
    // A configuration request can itself project Preparing. Only our still-
    // pending connect permits cancellation through that projection; this never
    // grants authority to change an existing connection while Agent is running.
    if (current.current.locked && attempt.settled) return;
    attempt.cancelled = true;
    cancelPending.current = true; setCancelling(true); pending.current = true; setBusy(true); setChoices(null); setNotice('');
    const issued = ++sequence.current; // Fence the original POST, including its finally/recovery GET.
    const isCurrent = () => issued === sequence.current && current.current.conversationId === conversationId;
    try {
      const live = await client.snapshot(conversationId);
      if (!isCurrent()) return;
      if (!live || live.state === 'disconnected' || live.incarnation === attempt.previous || (attempt.incarnation && live.incarnation !== attempt.incarnation)) {
        setSnapshot(live); setLoaded(true);
        if (attempt.settled && (!live || live.state === 'disconnected' || (attempt.incarnation && live.incarnation !== attempt.incarnation))) { ownConnect.current = null; setOwnConnecting(false); }
        setNotice(t('browserWorkspace.systemBrowser.cancelUnconfirmed'));
        return;
      }
      if (current.current.locked && attempt.settled) {
        setSnapshot(live); setLoaded(true);
        setNotice(t('browserWorkspace.systemBrowser.locked'));
        return;
      }
      const result = await client.disconnect(conversationId, live.incarnation);
      if (isCurrent()) { setSnapshot(result); setLoaded(true); ownConnect.current = null; setOwnConnecting(false); }
    } catch {
      if (isCurrent()) {
        setLoaded(false); setNotice(t('browserWorkspace.systemBrowser.cancelUnconfirmed'));
        // Observe once after an uncertain DELETE; never retry the mutation.
        try { const live = await client.snapshot(conversationId); if (isCurrent()) { setSnapshot(live); setLoaded(true); } } catch { /* Explicit refresh remains available. */ }
      }
    } finally {
      if (isCurrent()) { cancelPending.current = false; setCancelling(false); pending.current = false; setBusy(false); }
    }
  };
  const disabled = busy || locked || !loaded || !available;
  const connected = snapshot?.state === 'connected';
  const canConnect = !snapshot || snapshot.state === 'disconnected';
  const status = cancelling ? 'disconnecting' : ownConnecting ? 'connecting' : snapshot?.state ?? 'disconnected';

  return <Popover trigger='click' position='br' popupVisible={open} onVisibleChange={visible => {
    setOpen(visible);
    if (visible) void perform('refresh');
  }} content={<div className={styles.panel} role='dialog' aria-label={t('browserWorkspace.systemBrowser.title')}>
    <strong>{t('browserWorkspace.systemBrowser.title')}</strong>
    {!available ? <p role='status'>{t('browserWorkspace.systemBrowser.unavailable')}</p> : <>
      <p>{t('browserWorkspace.systemBrowser.description')}</p>
      <p className={styles.hint}>{t('browserWorkspace.systemBrowser.setup')}</p>
      <div className={styles.actions}>
        <span role='status'>{loaded || ownConnecting || cancelling ? t(`browserWorkspace.systemBrowser.states.${status}`) : t('browserWorkspace.systemBrowser.unknown')}</span>
        <Button size='mini' disabled={busy} onClick={() => void perform('refresh')}>{t('browserWorkspace.systemBrowser.refresh')}</Button>
      </div>
      {locked && !ownConnecting && <p role='status'>{t('browserWorkspace.systemBrowser.locked')}</p>}
      {notice && <p role='alert'>{notice}</p>}
      <div className={styles.actions}>
        {ownConnect.current ? <Button size='small' disabled={cancelling || !available || (locked && !ownConnecting)} onClick={() => void cancelConnect()}>{t('browserWorkspace.systemBrowser.cancelConnect')}</Button>
          : canConnect ? <Button type='primary' size='small' disabled={disabled} onClick={() => void perform('connect')}>{t('browserWorkspace.systemBrowser.connect')}</Button>
          : <Button size='small' disabled={disabled || snapshot?.state === 'connecting' || snapshot?.state === 'disconnecting'} onClick={() => void perform('disconnect')}>{t('browserWorkspace.systemBrowser.disconnect')}</Button>}
        {connected && <Button size='small' disabled={disabled} onClick={() => void perform('choices')}>{t('browserWorkspace.systemBrowser.chooseTabs')}</Button>}
      </div>
      {choices !== null && <div className={styles.list} role='group' aria-label={t('browserWorkspace.systemBrowser.availableTabs')}>
        {choices.length === 0 && <p>{t('browserWorkspace.systemBrowser.noChoices')}</p>}
        {choices.map(choice => <div className={styles.tab} key={choice.choice_id}>
          <div><strong>{choice.title || t('browserWorkspace.newTab')}</strong><span>{choice.url}</span></div>
          <Button size='mini' disabled={disabled} aria-label={t('browserWorkspace.systemBrowser.grantTab', { title: choice.title || t('browserWorkspace.newTab') })} onClick={() => void perform('grant', choice.choice_id)}>{t('browserWorkspace.systemBrowser.authorize')}</Button>
        </div>)}
      </div>}
      <div className={styles.list} role='group' aria-label={t('browserWorkspace.systemBrowser.authorizedTabs')}>
        <strong>{t('browserWorkspace.systemBrowser.authorizedTabs')}</strong>
        {!snapshot?.tabs.length && <p>{t('browserWorkspace.systemBrowser.noAuthorizedTabs')}</p>}
        {snapshot?.tabs.map(tab => <div className={styles.tab} key={tab.tab_id}><div><strong>{tab.title || t('browserWorkspace.newTab')}</strong><span>{tab.url}</span></div></div>)}
      </div>
    </>}
  </div>}>
    <Button size='mini' shape='round' aria-label={t('browserWorkspace.systemBrowser.title')} aria-expanded={open}><Earth size={14} /><span>{t('browserWorkspace.systemBrowser.title')}</span></Button>
  </Popover>;
}
