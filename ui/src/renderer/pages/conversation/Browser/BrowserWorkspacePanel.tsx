import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Dropdown, Menu, Modal } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import { Add, ArrowLeft, ArrowRight, Close, Earth, Refresh } from '@icon-park/react';
import { browserClient, navigationUrl, newerSnapshot, type BrowserClient, type BrowserCommand, type BrowserSnapshot, type SurfaceBounds, type BrowserShortcut, type BrowserShortcutAction } from './client';
import { browserShortcut } from './keyboardShortcuts';
import styles from './BrowserWorkspacePanel.module.css';
import type { BrowserLinkRequest } from './BrowserLinkContext';
import { localBrowserLink } from './localBrowserLink';
import WebsiteDialog from './WebsiteDialog';
import { copyText } from '@/renderer/utils/ui/clipboard';

type Props = { conversationId: string; onClose: () => void; client?: BrowserClient; linkRequest?: BrowserLinkRequest; onLinkConsumed?: (id: number) => void };

export default function BrowserWorkspacePanel({ conversationId, onClose, client = browserClient, linkRequest, onLinkConsumed }: Props) {
  const { t } = useTranslation();
  const [snapshot, setSnapshot] = useState<BrowserSnapshot | null>(null);
  const [address, setAddress] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [clearFailed, setClearFailed] = useState(false);
  const [retry, setRetry] = useState(0);
  const [draftTab, setDraftTab] = useState(false);
  const [notice, setNotice] = useState('');
  const consumedLink = useRef<BrowserLinkRequest | undefined>(undefined);
  const slot = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const hide = useRef<() => void>(() => {});
  const requestLayout = useRef<() => void>(() => {});
  const blocked = useRef(false);
  const draft = useRef(false);
  const currentConversation = useRef(conversationId);
  const commandSequence = useRef(0);
  const shortcutHandler = useRef<(action: BrowserShortcutAction, source?: BrowserShortcut) => void>(() => {});
  const [confirmation, setConfirmation] = useState<{ kind: 'rebuild' | 'clear_site_data'; conversationId: string; generation: number } | null>(null);
  useEffect(() => { setConfirmation(null); }, [conversationId]);
  currentConversation.current = conversationId;
  draft.current = draftTab;
  useEffect(() => { requestLayout.current(); }, [draftTab]);
  const tabs = snapshot?.runtime?.tabs ?? [];
  const downloads = snapshot?.runtime?.downloads ?? [];
  const active = tabs.find(tab => tab.target.tab_id === snapshot?.runtime?.active_tab_id);
  const permission = active?.permission_requests?.[0];
  const dialog = active?.script_dialog;
  const locked = snapshot?.run.input_state === 'agent_running';
  const controlsDisabled = !snapshot || locked || snapshot.run.input_gate_failed || busy || Boolean(error);
  useEffect(() => { if (locked) setDraftTab(false); }, [locked]);

  useEffect(() => { if (!draftTab) setAddress(active?.url === 'about:blank' ? '' : active?.url ?? ''); }, [active?.url, active?.target.tab_id, draftTab]);

  useEffect(() => {
    let disposed = false;
    let attachment: number | undefined;
    let initialized = false;
    let attaching = false;
    let frame = 0;
    let sequence = 0;
    let measurement = 0;
    let lastLayout = '';
    let lastBounds: SurfaceBounds = { x: 0, y: 0, width: 1, height: 1 };
    const measure = async () => {
      const element = slot.current;
      if (!element) return null;
      const scale = await client.scaleFactor();
      if (disposed || !element.isConnected || element !== slot.current) return null;
      const rect = element.getBoundingClientRect();
      if (!rect.width || !rect.height) return null;
      // During the conversation/focus layout transition the slot can briefly
      // extend past the renderer viewport. Wait for its next layout instead
      // of sending invalid native bounds and leaving a permanent error panel.
      if (rect.x < 0 || rect.y < 0 || rect.x + rect.width > window.innerWidth || rect.y + rect.height > window.innerHeight) return null;
      const factor = window.devicePixelRatio / scale;
      return { x: rect.x * factor, y: rect.y * factor, width: rect.width * factor, height: rect.height * factor };
    };
    const obscured = () => {
      const rect = slot.current?.getBoundingClientRect();
      if (!rect) return true;
      return Array.from(document.querySelectorAll<HTMLElement>('[role="dialog"], [aria-modal="true"], [role="menu"], [role="tooltip"]')).some(element => {
        if (element.getAttribute('aria-hidden') === 'true') return false;
        const overlay = element.getBoundingClientRect();
        return overlay.width > 0 && overlay.height > 0 && overlay.right > rect.left && overlay.left < rect.right && overlay.bottom > rect.top && overlay.top < rect.bottom;
      });
    };
    const send = (bounds: SurfaceBounds, visible: boolean) => {
      if (attachment === undefined || disposed) return;
      const layout = JSON.stringify({ bounds, visible });
      if (layout === lastLayout) return;
      lastLayout = layout;
      const issued = ++sequence;
      void client.update(attachment, issued, bounds, visible).catch(reason => {
        if (!disposed && issued === sequence) {
          blocked.current = true; hide.current(); setError(String(reason));
        }
      });
    };
    const refresh = async () => {
      const issued = ++measurement;
      if (disposed || !initialized) return;
      try {
        const bounds = await measure();
        if (disposed || issued !== measurement) return;
        if (attachment === undefined) {
          if (!bounds || attaching) return;
          attaching = true;
          lastBounds = bounds;
          const id = await client.attach(conversationId, bounds, event => {
            if (disposed) return;
            if (event.kind === 'snapshot') setSnapshot(current => newerSnapshot(current, event.snapshot));
            else { blocked.current = true; setError(event.code); hide.current(); }
          });
          if (disposed) { await client.detach(id); return; }
          attachment = id;
          schedule();
          return;
        }
        if (bounds) lastBounds = bounds;
        send(lastBounds, Boolean(bounds) && !blocked.current && !draft.current && document.visibilityState !== 'hidden' && !obscured());
      } catch (reason) {
        if (!disposed && issued === measurement) { blocked.current = true; hide.current(); setError(String(reason)); }
      }
    };
    const schedule = () => {
      ++measurement;
      if (blocked.current || draft.current || document.visibilityState === 'hidden' || obscured()) hide.current();
      cancelAnimationFrame(frame); frame = requestAnimationFrame(() => void refresh());
    };
    requestLayout.current = schedule;
    hide.current = () => { ++measurement; send(lastBounds, false); };
    const resized = new ResizeObserver(schedule);
    if (slot.current) resized.observe(slot.current);
    const overlays = new MutationObserver(schedule);
    overlays.observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ['aria-hidden', 'aria-modal', 'open', 'role', 'class', 'style'] });
    window.addEventListener('resize', schedule);
    window.addEventListener('scroll', schedule, true);
    document.addEventListener('visibilitychange', schedule);
    setError('');
    setSnapshot(null); setAddress(''); setBusy(false); setNotice(''); setClearFailed(false); ++commandSequence.current;
    blocked.current = false;
    void (async () => {
      try {
        const initial = await client.ensure(conversationId);
        if (disposed) return;
        setSnapshot(current => newerSnapshot(current, initial));
        initialized = true;
        schedule();
      } catch (reason) { if (!disposed) setError(String(reason)); }
    })();
    return () => {
      disposed = true;
      ++commandSequence.current;
      cancelAnimationFrame(frame);
      resized.disconnect(); overlays.disconnect();
      window.removeEventListener('resize', schedule);
      window.removeEventListener('scroll', schedule, true);
      document.removeEventListener('visibilitychange', schedule);
      if (attachment !== undefined) void client.detach(attachment).catch(() => {});
      hide.current = () => {};
      requestLayout.current = () => {};
    };
  }, [conversationId, client, retry]);

  const run = useCallback(async (command: BrowserCommand, fromLink = false) => {
    if (controlsDisabled) return;
    const issued = ++commandSequence.current;
    const current = () => currentConversation.current === conversationId && commandSequence.current === issued;
    setBusy(true);
    try { const next = await client.command(conversationId, command); if (current()) {
      setSnapshot(previous => newerSnapshot(previous, next));
      if (command.command === 'open_external') setNotice(t('browserWorkspace.externalHandedOff'));
      if (command.command === 'close_all' || command.command === 'clear_site_data') setDraftTab(false);
      if (command.command === 'clear_site_data') setNotice(t('browserWorkspace.clearSiteDataDone'));
    } }
    catch (reason) { if (current()) {
      if (command.command === 'permission') setNotice(t('browserWorkspace.permissionExpired'));
      else if (command.command === 'dialog') setNotice(t('browserWorkspace.dialogExpired'));
      else if (command.command === 'cancel_download') setNotice(t('browserWorkspace.downloadCancelFailed'));
      else if (command.command === 'open_external') setNotice(t('browserWorkspace.externalFailed'));
      else if (command.command === 'open_downloads') setNotice(t('browserWorkspace.openDownloadsFailed'));
      else if (command.command === 'clear_site_data') { blocked.current = true; hide.current(); setClearFailed(true); setError(t('browserWorkspace.clearSiteDataFailed')); }
      else if (fromLink) setNotice(t('browserWorkspace.linkFailed'));
      else { blocked.current = true; hide.current(); setError(String(reason)); }
    } }
    finally { if (current()) setBusy(false); }
  }, [client, conversationId, controlsDisabled, t]);
  shortcutHandler.current = (action, source) => {
    if (controlsDisabled || confirmation || dialog || permission) return;
    if (source && (draftTab || source.conversation_id !== conversationId || !active ||
        source.target.tab_id !== active.target.tab_id || source.target.runtime_generation !== active.target.runtime_generation ||
        source.target.document_generation !== active.target.document_generation)) return;
    if (action === 'address') { input.current?.focus(); input.current?.select(); return; }
    if (action === 'new_tab') { setDraftTab(true); setAddress(''); input.current?.focus(); return; }
    if (action === 'close_tab' && draftTab) { setDraftTab(false); return; }
    if (!active || draftTab) return;
    if (action === 'back' && !active.can_go_back || action === 'forward' && !active.can_go_forward) return;
    void run({ command: action === 'close_tab' ? 'close' : action, target: active.target });
  };
  useEffect(() => {
    let disposed = false;
    let unsubscribe: (() => void) | undefined;
    void client.listenShortcuts(conversationId, event => { if (!disposed) shortcutHandler.current(event.action, event); })
      .then(stop => { if (disposed) stop(); else unsubscribe = stop; })
      .catch(() => { if (!disposed) setNotice(t('browserWorkspace.shortcutsUnavailable')); });
    return () => { disposed = true; unsubscribe?.(); };
  }, [client, conversationId, t]);
  useEffect(() => {
    if (!linkRequest || consumedLink.current === linkRequest || (!snapshot && !error)) return;
    consumedLink.current = linkRequest;
    onLinkConsumed?.(linkRequest.id);
    // Never queue user navigation until the Agent stops. The server checks the
    // run guard again, including a run that begins after this snapshot.
    if (controlsDisabled) {
      setNotice(t(locked ? 'browserWorkspace.linkLocked' : 'browserWorkspace.linkBusy'));
      return;
    }
    const link = localBrowserLink(linkRequest.url);
    if (!link) { setNotice(t('browserWorkspace.invalidUrl')); return; }
    setDraftTab(false);
    setNotice(linkRequest.mappedFrom ? t('browserWorkspace.linkMapped', { host: linkRequest.mappedFrom }) : '');
    const existing = tabs.find(tab => tab.url === link.url);
    void run(existing ? { command: 'activate', target: existing.target } : { command: 'create', url: link.url }, true);
  }, [linkRequest, onLinkConsumed, snapshot, error, controlsDisabled, locked, tabs, run, t]);
  const navigate = (event: React.FormEvent) => {
    event.preventDefault();
    const url = navigationUrl(address);
    if (!url) { input.current?.setCustomValidity(t('browserWorkspace.invalidUrl')); input.current?.reportValidity(); return; }
    const command: BrowserCommand = active && !draftTab ? { command: 'navigate', target: active.target, url } : { command: 'create', url };
    setDraftTab(false);
    void run(command);
  };

  const rebuild = () => {
    const generation = snapshot?.runtime?.runtime_generation;
    if (generation === undefined || busy || (locked && !snapshot?.run.input_gate_failed)) return;
    setConfirmation({ kind: 'rebuild', conversationId, generation });
  };
  const copyAddress = async () => {
    if (controlsDisabled || draftTab || !active?.url) return;
    const issued = ++commandSequence.current;
    const current = () => currentConversation.current === conversationId && commandSequence.current === issued;
    setBusy(true);
    try {
      await copyText(active.url);
      if (current()) setNotice(t('browserWorkspace.addressCopied'));
    } catch {
      if (current()) setNotice(t('browserWorkspace.addressCopyFailed'));
    } finally { if (current()) setBusy(false); }
  };
  const confirmRebuild = async () => {
    if (!confirmation || confirmation.conversationId !== conversationId || busy) return;
    if (confirmation.kind === 'clear_site_data') {
      if (controlsDisabled) return;
      const captured = confirmation;
      if (captured.generation !== snapshot?.runtime?.runtime_generation) {
        setConfirmation(null); setNotice(t('browserWorkspace.clearSiteDataFailed')); return;
      }
      await run({ command: 'clear_site_data', runtime_generation: captured.generation });
      setConfirmation(current => current === captured ? null : current);
      return;
    }
    const issued = ++commandSequence.current;
    const current = () => currentConversation.current === conversationId && commandSequence.current === issued;
    setBusy(true);
    try {
      await client.closeWorkspace(conversationId, confirmation.generation);
      if (current()) { blocked.current = false; setRetry(value => value + 1); }
    } catch {
      if (current()) setNotice(t('browserWorkspace.rebuildFailed'));
    } finally { if (current()) { setBusy(false); setConfirmation(null); } }
  };

  return <section className={styles.panel} aria-label={t('browserWorkspace.title')} onKeyDown={event => {
    const action = browserShortcut(event.nativeEvent);
    if (!action) return;
    event.preventDefault(); event.stopPropagation();
    if (!event.repeat) shortcutHandler.current(action);
  }}>
    {confirmation?.conversationId === conversationId && <Modal visible title={t(confirmation.kind === 'clear_site_data' ? 'browserWorkspace.clearSiteDataTitle' : 'browserWorkspace.rebuildTitle')}
      okText={t(confirmation.kind === 'clear_site_data' ? 'browserWorkspace.clearSiteDataConfirm' : 'browserWorkspace.rebuildConfirm')} cancelText={t('browserWorkspace.rebuildCancel')}
      okButtonProps={{ disabled: confirmation.kind === 'clear_site_data' && controlsDisabled }}
      confirmLoading={busy} cancelButtonProps={{ disabled: busy }} closable={!busy} maskClosable={!busy}
      onCancel={() => { if (!busy) setConfirmation(null); }} onOk={confirmRebuild}>
      {t(confirmation.kind === 'clear_site_data' ? 'browserWorkspace.clearSiteDataWarning' : 'browserWorkspace.rebuildWarning')}
    </Modal>}
    <div className={styles.tabs} role='tablist' aria-label={t('browserWorkspace.pages')}>
      {tabs.map(tab => <div className={styles.tab} key={tab.target.tab_id} data-active={tab.target.tab_id === active?.target.tab_id && !draftTab}>
        <button type='button' role='tab' aria-selected={tab.target.tab_id === active?.target.tab_id && !draftTab} disabled={controlsDisabled} onClick={() => { setDraftTab(false); void run({ command: 'activate', target: tab.target }); }}><Earth size={13} /><span>{tab.title || t('browserWorkspace.newTab')}</span></button>
        <button type='button' disabled={controlsDisabled} aria-label={t('browserWorkspace.closePage', { title: tab.title || t('browserWorkspace.newTab') })} onClick={() => void run({ command: 'close', target: tab.target })}><Close size={11} /></button>
      </div>)}
      <button type='button' className={styles.icon} disabled={controlsDisabled} aria-label={t('browserWorkspace.newTab')} onClick={() => { setDraftTab(true); setAddress(''); input.current?.focus(); }}><Add size={16} /></button>
      <button type='button' className={`${styles.icon} ${styles.close}`} aria-label={t('browserWorkspace.closePanel')} onClick={onClose}><Close size={15} /></button>
    </div>
    <form className={styles.navigation} onSubmit={navigate}>
      <button className={styles.icon} type='button' disabled={controlsDisabled || !active?.can_go_back} aria-label={t('browserWorkspace.back')} onClick={() => active && void run({ command: 'back', target: active.target })}><ArrowLeft size={16} /></button>
      <button className={styles.icon} type='button' disabled={controlsDisabled || !active?.can_go_forward} aria-label={t('browserWorkspace.forward')} onClick={() => active && void run({ command: 'forward', target: active.target })}><ArrowRight size={16} /></button>
      <button className={styles.icon} type='button' disabled={controlsDisabled || !active} aria-label={t(active?.lifecycle === 'loading' ? 'browserWorkspace.stopLoading' : 'browserWorkspace.reload')} onClick={() => active && void run({ command: active.lifecycle === 'loading' ? 'stop_loading' : 'reload', target: active.target })}>{active?.lifecycle === 'loading' ? <Close size={15} /> : <Refresh size={15} />}</button>
      <input ref={input} value={address} disabled={controlsDisabled} placeholder={t('browserWorkspace.addressPlaceholder')} aria-label={t('browserWorkspace.address')} onChange={event => { event.target.setCustomValidity(''); setAddress(event.target.value); }} spellCheck={false} autoComplete='off' />
      <Dropdown trigger='click' position='br' droplist={<Menu>
        <Menu.Item key='copy-address' disabled={controlsDisabled || draftTab || !active?.url} onClick={() => void copyAddress()}>{t('browserWorkspace.copyAddress')}</Menu.Item>
        <Menu.Item key='external' disabled={controlsDisabled || draftTab || !active || !/^https?:\/\//i.test(active.url)} onClick={() => { if (active && !draftTab) void run({ command: 'open_external', target: active.target }); }}>{t('browserWorkspace.openExternal')}</Menu.Item>
        <Menu.Item key='open-downloads' disabled={controlsDisabled} onClick={() => { if (snapshot?.runtime) void run({ command: 'open_downloads', runtime_generation: snapshot.runtime.runtime_generation }); }}>{t('browserWorkspace.openDownloads')}</Menu.Item>
        <Menu.Item key='clear-site-data' disabled={controlsDisabled || !active || draftTab} onClick={() => {
          if (!controlsDisabled && active && !draftTab && snapshot?.runtime) setConfirmation({ kind: 'clear_site_data', conversationId, generation: snapshot.runtime.runtime_generation });
        }}>{t('browserWorkspace.clearSiteDataTitle')}</Menu.Item>
        <Menu.ItemGroup title={t('browserWorkspace.downloads')}>
          <div className={styles.downloads} role='group' aria-label={t('browserWorkspace.downloads')}>
            {downloads.length === 0 && <div className={styles.downloadEmpty}>{t('browserWorkspace.downloadEmpty')}</div>}
            {[...downloads].reverse().map(download => {
              const target = tabs.find(tab => tab.target.tab_id === download.tab_id)?.target;
              return <div key={download.id} className={styles.download}>
                <div><span className={styles.downloadName} title={download.filename}>{download.filename}</span>
                  <small>{t(`browserWorkspace.downloadStates.${download.state}`)}{download.received_bytes > 0 && ` · ${Math.ceil(download.received_bytes / 1024).toLocaleString()} KB${download.total_bytes !== null ? ` / ${Math.ceil(download.total_bytes / 1024).toLocaleString()} KB` : ''}`}</small></div>
                {download.can_cancel && <button type='button' disabled={controlsDisabled || !target} aria-label={t('browserWorkspace.downloadCancel', { filename: download.filename })} onClick={event => { event.stopPropagation(); if (target) void run({ command: 'cancel_download', target, download_id: download.id }); }}>{t('browserWorkspace.dialogCancel')}</button>}
              </div>;
            })}
          </div>
        </Menu.ItemGroup>
        <Menu.Item key='close-all' disabled={controlsDisabled || tabs.length === 0} onClick={() => { if (snapshot?.runtime && tabs.length > 0) void run({ command: 'close_all', runtime_generation: snapshot.runtime.runtime_generation }); }}>{t('browserWorkspace.closeAllPages')}</Menu.Item>
        <Menu.Item key='rebuild' onClick={rebuild}>{t('browserWorkspace.rebuild')}</Menu.Item>
      </Menu>}>
        <button type='button' className={styles.icon} aria-label={t('browserWorkspace.menu')} disabled={!snapshot?.runtime || busy || (locked && !snapshot.run.input_gate_failed)}>⋯</button>
      </Dropdown>
    </form>
    <div className={styles.status} role='status'><span className={styles.dot} data-locked={locked} />{t(locked ? 'browserWorkspace.agentRunning' : 'browserWorkspace.userReady')}{locked && <span className={styles.hint}>{t('browserWorkspace.stopHint')}</span>}</div>
    {notice && <div className={styles.notice} role='status'><span>{notice}</span><button type='button' className={styles.icon} aria-label={t('browserWorkspace.dismissNotice')} onClick={() => setNotice('')}><Close size={12} /></button></div>}
    {!permission && active && Boolean(active.blocked_permissions?.length) && !locked && !draftTab && !error && <div className={styles.permission} role='status'>
      <span>{t('browserWorkspace.permissionRetryHint')}</span>
      <button type='button' disabled={controlsDisabled} onClick={() => void run({ command: 'reload', target: active.target })}>{t('browserWorkspace.reload')}</button>
    </div>}
    {permission && active && !locked && !draftTab && !error && <div className={styles.permission} role='group' aria-live='polite' aria-label={t('browserWorkspace.permissionTitle')}>
      <span>{t('browserWorkspace.permissionRequest', { origin: permission.origin, permission: t(`browserWorkspace.permissionKinds.${permission.kind}`, { defaultValue: permission.kind }) })}</span>
      <div><button type='button' disabled={controlsDisabled} onClick={() => void run({ command: 'permission', target: active.target, request_id: permission.request_id, allow: false })}>{t('browserWorkspace.permissionDeny')}</button>
      <button type='button' disabled={controlsDisabled} onClick={() => void run({ command: 'permission', target: active.target, request_id: permission.request_id, allow: true })}>{t('browserWorkspace.permissionAllow')}</button></div>
    </div>}
    <div ref={slot} className={styles.surface} data-browser-surface>
      {dialog && !draftTab && !error && <WebsiteDialog key={`${conversationId}:${dialog.request_id}`} dialog={dialog} locked={Boolean(locked || snapshot?.run.input_gate_failed)} busy={busy} onReply={command => void run(command)} />}
      {error ? <div className={styles.empty} role='alert'><Earth size={28} /><strong>{t('browserWorkspace.unavailable')}</strong><p>{error}</p><button type='button' onClick={() => clearFailed ? rebuild() : setRetry(value => value + 1)}>{t(clearFailed ? 'browserWorkspace.rebuild' : 'browserWorkspace.retry')}</button></div>
        : (tabs.length === 0 || draftTab) && <div className={styles.empty}><Earth size={30} /><strong>{t('browserWorkspace.start')}</strong><p>{t('browserWorkspace.startHint')}</p></div>}
    </div>
  </section>;
}
