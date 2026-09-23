import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Modal } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import { Add, ArrowLeft, ArrowRight, Close, Earth, Info, Lock, Refresh } from '@icon-park/react';
import { browserClient, browserCommandAction, navigationUrl, newerSnapshot, type AttachedProviderState, type BrowserClient, type BrowserCommand, type BrowserSnapshot, type SurfaceBounds, type BrowserShortcut, type BrowserShortcutAction } from './client';
import { browserShortcut } from './keyboardShortcuts';
import styles from './BrowserPanel.module.css';
import type { BrowserLinkRequest } from './BrowserLinkContext';
import { localBrowserLink } from './localBrowserLink';
import WebsiteDialog from './WebsiteDialog';
import { copyText } from '@/renderer/utils/ui/clipboard';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import { capabilityPermissionsHref } from '@/renderer/hooks/system/systemPermissionModel';

type Props = { agentSessionId: string; onClose: () => void; client?: BrowserClient; linkRequest?: BrowserLinkRequest; onLinkConsumed?: (id: number, handled: boolean) => void; onLinkAvailabilityChange?: (available: boolean) => void; hostSurfaceAvailable?: boolean; panelId?: string };
type BrowserFailureKind = 'capability' | 'provider' | 'host' | 'request' | 'clear';
type BrowserFailure = { kind: BrowserFailureKind; message: 'browserWorkspace.hostUnavailableHint' | 'browserWorkspace.requestFailed' | 'browserWorkspace.clearSiteDataFailed'; retryable: boolean };
const ZOOM_LEVELS = [50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200] as const;

export function browserFailure(reason: unknown): BrowserFailure {
  const http = isBackendHttpError(reason) ? reason : null;
  const code = http?.code ?? (typeof reason === 'string' ? reason : '');
  if (code === 'BROWSER_NATIVE_SURFACE_UNAVAILABLE' || http?.status === 501) {
    return { kind: 'host', message: 'browserWorkspace.hostUnavailableHint', retryable: false };
  }
  if (code === 'PROVIDER_UNAVAILABLE' || code === 'BROWSER_PROVIDER_ERROR' || http?.status === 422) {
    return { kind: 'provider', message: 'browserWorkspace.requestFailed', retryable: true };
  }
  if ((code === 'FORBIDDEN' || code === 'BROWSER_ACTION_DENIED') && !http?.authExpired) {
    return { kind: 'capability', message: 'browserWorkspace.requestFailed', retryable: false };
  }
  return { kind: 'request', message: 'browserWorkspace.requestFailed', retryable: true };
}

export default function BrowserPanel({ agentSessionId, onClose, client = browserClient, linkRequest, onLinkConsumed, onLinkAvailabilityChange, hostSurfaceAvailable = true, panelId }: Props) {
  const { t } = useTranslation();
  const [snapshot, setSnapshot] = useState<BrowserSnapshot | null>(null);
  const [address, setAddress] = useState('');
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<BrowserFailure | null>(null);
  const [clearFailed, setClearFailed] = useState(false);
  const [retry, setRetry] = useState(0);
  const [draftTab, setDraftTab] = useState(false);
  const [notice, setNotice] = useState('');
  const [menuOpen, setMenuOpen] = useState(false);
  const [authorityOpen, setAuthorityOpen] = useState(false);
  const [attachedState, setAttachedState] = useState<AttachedProviderState | null>(null);
  const consumedLink = useRef<BrowserLinkRequest | undefined>(undefined);
  const slot = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const menuButton = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const hide = useRef<() => void>(() => {});
  const requestLayout = useRef<() => void>(() => {});
  const blocked = useRef(false);
  const draft = useRef(false);
  const currentAgentSession = useRef(agentSessionId);
  const focusedEmptySession = useRef('');
  const commandSequence = useRef(0);
  const shortcutHandler = useRef<(action: BrowserShortcutAction, source?: BrowserShortcut) => void>(() => {});
  const [confirmation, setConfirmation] = useState<{ kind: 'rebuild' | 'clear_site_data'; agentSessionId: string; generation: number } | null>(null);
  useEffect(() => { setConfirmation(null); setMenuOpen(false); setAuthorityOpen(false); }, [agentSessionId]);
  currentAgentSession.current = agentSessionId;
  draft.current = draftTab;
  useEffect(() => { requestLayout.current(); }, [draftTab, authorityOpen]);
  const tabs = snapshot?.runtime?.tabs ?? [];
  const downloads = snapshot?.runtime?.downloads ?? [];
  const active = tabs.find(tab => tab.target.tab_id === snapshot?.runtime?.active_tab_id);
  const zoomPercent = active?.zoom_percent ?? 100;
  const zoomOutPercent = [...ZOOM_LEVELS].reverse().find(percent => percent < zoomPercent);
  const zoomInPercent = ZOOM_LEVELS.find(percent => percent > zoomPercent);
  const permission = active?.permission_requests?.[0];
  const hasSystemPermissionBlock = Boolean(active?.blocked_permissions?.some((kind) =>
    ['camera', 'microphone', 'camera_microphone', 'geolocation', 'notifications'].includes(kind)
  ));
  const permissionMayNeedSystemAccess = Boolean(permission &&
    ['camera', 'microphone', 'camera_microphone', 'geolocation', 'notifications'].includes(permission.kind)
  );
  const dialog = active?.script_dialog;
  const locked = snapshot?.run.input_state === 'agent_running';
  const attachedProvider = snapshot?.provider_kind === 'attached_chrome';
  const controlsDisabled = !snapshot || attachedProvider || locked || snapshot.run.input_gate_failed || busy || Boolean(failure);
  const canNavigate = !controlsDisabled && Boolean(snapshot?.allowed_actions.includes('browser/navigate'));
  const canZoom = canNavigate && Boolean(active) && !draftTab;
  const canAct = !controlsDisabled && Boolean(snapshot?.allowed_actions.includes('browser/act'));
  const canDownload = !controlsDisabled && Boolean(snapshot?.allowed_actions.includes('browser/download'));
  const canAcceptLinks = hostSurfaceAvailable && snapshot?.provider_kind === 'managed' && canNavigate;
  useEffect(() => {
    onLinkAvailabilityChange?.(Boolean(canAcceptLinks));
    return () => onLinkAvailabilityChange?.(false);
  }, [canAcceptLinks, onLinkAvailabilityChange]);
  useEffect(() => { if (locked) setDraftTab(false); }, [locked]);
  useEffect(() => {
    if (!menuOpen) return undefined;
    const dismiss = (event: PointerEvent) => {
      const target = event.target as Node | null;
      if (target && (menu.current?.contains(target) || menuButton.current?.contains(target))) return;
      setMenuOpen(false);
    };
    document.addEventListener('pointerdown', dismiss);
    return () => document.removeEventListener('pointerdown', dismiss);
  }, [menuOpen]);

  useEffect(() => { if (!draftTab) setAddress(active?.url === 'about:blank' ? '' : active?.url ?? ''); }, [active?.url, active?.target.tab_id, draftTab]);
  useEffect(() => {
    if (snapshot?.provider_kind !== 'managed' || tabs.length !== 0 || locked || snapshot.run.input_gate_failed || failure) return;
    if (focusedEmptySession.current === agentSessionId) return;
    focusedEmptySession.current = agentSessionId;
    input.current?.focus();
  }, [agentSessionId, failure, locked, snapshot?.provider_kind, snapshot?.run.input_gate_failed, tabs.length]);

  useEffect(() => {
    if (!hostSurfaceAvailable) {
      setSnapshot(null); setAddress(''); setBusy(false); setNotice(''); setClearFailed(false);
      setAttachedState(null); ++commandSequence.current;
      blocked.current = true;
      setFailure({ kind: 'host', message: 'browserWorkspace.hostUnavailableHint', retryable: false });
      return () => { requestLayout.current = () => {}; hide.current = () => {}; };
    }
    let disposed = false;
    let attachment: number | undefined;
    let initialized = false;
    let nativeSurface = false;
    let attaching = false;
    let frame = 0;
    let sequence = 0;
    let measurement = 0;
    let providerTimer: number | undefined;
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
          blocked.current = true; hide.current(); setFailure(browserFailure(reason));
        }
      });
    };
    const refresh = async () => {
      const issued = ++measurement;
      if (disposed || !initialized || !nativeSurface) return;
      try {
        const bounds = await measure();
        if (disposed || issued !== measurement) return;
        if (attachment === undefined) {
          if (!bounds || attaching) return;
          attaching = true;
          lastBounds = bounds;
          const id = await client.attach(agentSessionId, bounds, event => {
            if (disposed) return;
            if (event.kind === 'snapshot') setSnapshot(current => newerSnapshot(current, event.snapshot));
            else { blocked.current = true; setFailure(browserFailure(event.code)); hide.current(); }
          });
          if (disposed) { await client.detach(id); return; }
          attachment = id;
          schedule();
          return;
        }
        if (bounds) lastBounds = bounds;
        send(lastBounds, Boolean(bounds) && !blocked.current && !draft.current && document.visibilityState !== 'hidden' && !obscured());
      } catch (reason) {
        if (!disposed && issued === measurement) { blocked.current = true; hide.current(); setFailure(browserFailure(reason)); }
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
    setFailure(null);
    setSnapshot(null); setAddress(''); setBusy(false); setNotice(''); setClearFailed(false); setAttachedState(null); ++commandSequence.current;
    blocked.current = false;
    void (async () => {
      try {
        const initial = await client.ensure(agentSessionId);
        if (disposed) return;
        setSnapshot(current => newerSnapshot(current, initial));
        initialized = true;
        nativeSurface = initial.provider_kind === 'managed';
        if (nativeSurface) schedule();
        else {
          const verifyAttached = async () => {
            try {
              const provider = await client.attachedProvider();
              if (disposed) return false;
              if (!provider || provider.state !== 'connected') {
                setAttachedState(provider?.state ?? 'connection_lost');
                setFailure({ kind: 'provider', message: 'browserWorkspace.requestFailed', retryable: true });
                return false;
              }
              setAttachedState(provider.state);
              return true;
            } catch {
              if (!disposed) {
                setAttachedState(null);
                setFailure({ kind: 'provider', message: 'browserWorkspace.requestFailed', retryable: true });
              }
              return false;
            }
          };
          if (await verifyAttached() && !disposed) {
            providerTimer = window.setInterval(() => {
              void verifyAttached().then(connected => {
                if (!connected && providerTimer !== undefined) window.clearInterval(providerTimer);
              });
            }, 3000);
          }
        }
      } catch (reason) { if (!disposed) setFailure(browserFailure(reason)); }
    })();
    return () => {
      disposed = true;
      ++commandSequence.current;
      cancelAnimationFrame(frame);
      if (providerTimer !== undefined) window.clearInterval(providerTimer);
      resized.disconnect(); overlays.disconnect();
      window.removeEventListener('resize', schedule);
      window.removeEventListener('scroll', schedule, true);
      document.removeEventListener('visibilitychange', schedule);
      if (attachment !== undefined) void client.detach(attachment).catch(() => {});
      hide.current = () => {};
      requestLayout.current = () => {};
    };
  }, [agentSessionId, client, hostSurfaceAvailable, retry]);

  const run = useCallback(async (command: BrowserCommand, fromLink = false) => {
    if (controlsDisabled) return;
    // Human tab close accepts either navigation or interaction authority.
    const allowed = snapshot?.allowed_actions.includes(browserCommandAction(command))
      || (command.command === 'close' && snapshot?.allowed_actions.includes('browser/navigate'));
    if (!allowed) {
      setNotice(t('browserWorkspace.actionUnavailableHint'));
      return;
    }
    const issued = ++commandSequence.current;
    const current = () => currentAgentSession.current === agentSessionId && commandSequence.current === issued;
    setBusy(true);
    try { const next = await client.command(agentSessionId, command); if (current()) {
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
      else if (command.command === 'set_zoom') setNotice(t('browserWorkspace.zoomFailed'));
      else if (command.command === 'clear_site_data') { blocked.current = true; hide.current(); setClearFailed(true); setFailure({ kind: 'clear', message: 'browserWorkspace.clearSiteDataFailed', retryable: true }); }
      else if (fromLink) setNotice(t('browserWorkspace.linkFailed'));
      else { blocked.current = true; hide.current(); setFailure(browserFailure(reason)); }
    } }
    finally { if (current()) setBusy(false); }
  }, [client, agentSessionId, controlsDisabled, snapshot?.allowed_actions, t]);
  shortcutHandler.current = (action, source) => {
    if (controlsDisabled || confirmation || dialog || permission) return;
    if (source && (draftTab || source.agent_session_id !== agentSessionId || !active ||
        source.target.tab_id !== active.target.tab_id || source.target.runtime_generation !== active.target.runtime_generation ||
        source.target.document_generation !== active.target.document_generation)) return;
    if (action === 'address') { if (canNavigate) { input.current?.focus(); input.current?.select(); } return; }
    if (action === 'new_tab') { if (canNavigate) { setDraftTab(true); setAddress(''); input.current?.focus(); } return; }
    if (action === 'close_tab' && draftTab) { setDraftTab(false); return; }
    if (!active || draftTab) return;
    if (action === 'back' && !active.can_go_back || action === 'forward' && !active.can_go_forward) return;
    void run({ command: action === 'close_tab' ? 'close' : action, target: active.target });
  };
  useEffect(() => {
    let disposed = false;
    let unsubscribe: (() => void) | undefined;
    void client.listenShortcuts(agentSessionId, event => { if (!disposed) shortcutHandler.current(event.action, event); })
      .then(stop => { if (disposed) stop(); else unsubscribe = stop; })
      .catch(() => { if (!disposed) setNotice(t('browserWorkspace.shortcutsUnavailable')); });
    return () => { disposed = true; unsubscribe?.(); };
  }, [client, agentSessionId, t]);
  useEffect(() => {
    if (!linkRequest || consumedLink.current === linkRequest || (!snapshot && !failure)) return;
    consumedLink.current = linkRequest;
    // Never queue user navigation until the Agent stops. The server checks the
    // run guard again, including a run that begins after this snapshot.
    if (!canAcceptLinks || controlsDisabled) {
      onLinkConsumed?.(linkRequest.id, false);
      return;
    }
    const link = localBrowserLink(linkRequest.url);
    if (!link) { onLinkConsumed?.(linkRequest.id, false); return; }
    onLinkConsumed?.(linkRequest.id, true);
    setDraftTab(false);
    setNotice(linkRequest.mappedFrom ? t('browserWorkspace.linkMapped', { host: linkRequest.mappedFrom }) : '');
    const existing = tabs.find(tab => tab.url === link.url);
    void run(existing ? { command: 'activate', target: existing.target } : { command: 'create', url: link.url }, true);
  }, [canAcceptLinks, linkRequest, onLinkConsumed, snapshot, failure, controlsDisabled, tabs, run, t]);
  const navigate = (event: React.FormEvent) => {
    event.preventDefault();
    if (!canNavigate) { setNotice(t('browserWorkspace.actionUnavailableHint')); return; }
    const url = navigationUrl(input.current?.value ?? address);
    if (!url) { input.current?.setCustomValidity(t('browserWorkspace.invalidUrl')); input.current?.reportValidity(); return; }
    const command: BrowserCommand = active && !draftTab ? { command: 'navigate', target: active.target, url } : { command: 'create', url };
    setDraftTab(false);
    void run(command);
  };

  const rebuild = () => {
    const generation = snapshot?.runtime?.runtime_generation;
    if (generation === undefined || busy || (locked && !snapshot?.run.input_gate_failed)) return;
    setConfirmation({ kind: 'rebuild', agentSessionId, generation });
  };
  const copyAddress = async () => {
    if (controlsDisabled || draftTab || !active?.url) return;
    const issued = ++commandSequence.current;
    const current = () => currentAgentSession.current === agentSessionId && commandSequence.current === issued;
    setBusy(true);
    try {
      await copyText(active.url);
      if (current()) setNotice(t('browserWorkspace.addressCopied'));
    } catch {
      if (current()) setNotice(t('browserWorkspace.addressCopyFailed'));
    } finally { if (current()) setBusy(false); }
  };
  const confirmRebuild = async () => {
    if (!confirmation || confirmation.agentSessionId !== agentSessionId || busy) return;
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
    const current = () => currentAgentSession.current === agentSessionId && commandSequence.current === issued;
    setBusy(true);
    try {
      await client.closeResource(agentSessionId, confirmation.generation);
      if (current()) { blocked.current = false; setRetry(value => value + 1); }
    } catch {
      if (current()) setNotice(t('browserWorkspace.rebuildFailed'));
    } finally { if (current()) { setBusy(false); setConfirmation(null); } }
  };

  const browserTitle = t('settings.openCapabilities.domainBrowserTitle', { defaultValue: 'Browser' });
  const providerLabel = snapshot?.provider_kind === 'attached_chrome'
    ? t('browserWorkspace.provider.attachedChrome', { defaultValue: 'Authorized Chrome' })
    : t('browserWorkspace.provider.managed', { defaultValue: 'Built-in isolated browser' });
  const failureTitle = failure?.kind === 'capability'
    ? t('browserWorkspace.capabilityUnavailableTitle', { defaultValue: 'Browser capability unavailable' })
    : failure?.kind === 'provider'
      ? t('browserWorkspace.providerUnavailableTitle', { defaultValue: 'Browser provider unavailable' })
      : t('browserWorkspace.unavailable');
  const failureHint = failure?.kind === 'capability'
    ? t('browserWorkspace.capabilityUnavailableHint', { defaultValue: "This session's Agent does not include Browser. Enable the Browser module in Agent Workbench, then start a new session." })
    : failure?.kind === 'provider'
      ? t('browserWorkspace.providerUnavailableHint', { defaultValue: 'The Browser module is enabled, but its selected provider is not connected. Check the provider and retry.' })
      : failure?.kind === 'host'
        ? t('browserWorkspace.surfaceUnavailableHint', { defaultValue: 'This device cannot present the selected Browser provider.' })
        : failure ? t(failure.message) : '';
  const limitedActions = snapshot?.provider_kind === 'managed'
    && (!snapshot.allowed_actions.includes('browser/navigate')
      || !snapshot.allowed_actions.includes('browser/act')
      || !snapshot.allowed_actions.includes('browser/download'));
  useEffect(() => { if (!limitedActions) setAuthorityOpen(false); }, [limitedActions]);
  const authorityHintId = `${panelId ?? 'agent-session-browser'}-authority-hint`;
  const showAuthority = () => { hide.current(); setAuthorityOpen(true); };
  const changeZoom = (percent: number) => {
    if (!canZoom || !active) return;
    setMenuOpen(false);
    menuButton.current?.focus();
    void run({ command: 'set_zoom', target: active.target, percent });
  };
  const focusAdjacentTab = (event: React.KeyboardEvent<HTMLButtonElement>, index: number) => {
    if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey || !['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
    event.preventDefault();
    const nextIndex = event.key === 'Home' ? 0 : event.key === 'End' ? tabs.length - 1 : (index + (event.key === 'ArrowRight' ? 1 : -1) + tabs.length) % tabs.length;
    const next = tabs[nextIndex];
    if (!next) return;
    const buttons = event.currentTarget.closest('[role="tablist"]')?.querySelectorAll<HTMLButtonElement>('[role="tab"]');
    buttons?.[nextIndex]?.focus();
    setDraftTab(false);
    void run({ command: 'activate', target: next.target });
  };
  const focusMenuItem = (edge: 'first' | 'last' = 'first') => {
    requestAnimationFrame(() => {
      const items = [...(menu.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]:not(:disabled)') ?? [])];
      items[edge === 'first' ? 0 : items.length - 1]?.focus();
    });
  };
  const handleMenuKey = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Escape') {
      event.preventDefault(); event.stopPropagation(); setMenuOpen(false); menuButton.current?.focus(); return;
    }
    if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
    event.preventDefault();
    const items = [...(menu.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]:not(:disabled)') ?? [])];
    if (!items.length) return;
    const current = items.indexOf(document.activeElement as HTMLButtonElement);
    const index = event.key === 'Home' ? 0 : event.key === 'End' ? items.length - 1 : (current + (event.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length;
    items[index]?.focus();
  };

  return <section id={panelId} className={styles.panel} aria-label={browserTitle} tabIndex={-1} onKeyDown={event => {
    if (event.key === 'Escape' && !confirmation && !dialog && !permission && !menuOpen) {
      event.preventDefault(); event.stopPropagation(); onClose(); return;
    }
    const action = browserShortcut(event.nativeEvent);
    if (!action) return;
    event.preventDefault(); event.stopPropagation();
    if (!event.repeat) shortcutHandler.current(action);
  }}>
    <h2 className={styles.srTitle}>{browserTitle}</h2>
    {confirmation?.agentSessionId === agentSessionId && <Modal visible title={t(confirmation.kind === 'clear_site_data' ? 'browserWorkspace.clearSiteDataTitle' : 'browserWorkspace.rebuildTitle')}
      okText={t(confirmation.kind === 'clear_site_data' ? 'browserWorkspace.clearSiteDataConfirm' : 'browserWorkspace.rebuildConfirm')} cancelText={t('browserWorkspace.rebuildCancel')}
      okButtonProps={{ disabled: confirmation.kind === 'clear_site_data' && controlsDisabled }}
      confirmLoading={busy} cancelButtonProps={{ disabled: busy }} closable={!busy} maskClosable={!busy}
      onCancel={() => { if (!busy) setConfirmation(null); }} onOk={confirmRebuild}>
      {t(confirmation.kind === 'clear_site_data' ? 'browserWorkspace.clearSiteDataWarning' : 'browserWorkspace.rebuildWarning')}
    </Modal>}
    <div className={styles.tabs} role='tablist' aria-label={t('browserWorkspace.pages')}>
      {tabs.map((tab, index) => <div className={styles.tab} key={tab.target.tab_id} data-active={tab.target.tab_id === active?.target.tab_id && !draftTab}>
        <button type='button' role='tab' aria-selected={tab.target.tab_id === active?.target.tab_id && !draftTab} tabIndex={tab.target.tab_id === active?.target.tab_id && !draftTab ? 0 : -1} disabled={!canNavigate} onKeyDown={event => focusAdjacentTab(event, index)} onClick={() => { setDraftTab(false); void run({ command: 'activate', target: tab.target }); }}><Earth size={13} /><span>{tab.title || t('browserWorkspace.newTab')}</span></button>
        <button type='button' disabled={!canNavigate && !canAct} aria-label={t('browserWorkspace.closePage', { title: tab.title || t('browserWorkspace.newTab') })} onClick={() => void run({ command: 'close', target: tab.target })}><Close size={11} /></button>
      </div>)}
      <button type='button' className={styles.icon} disabled={!canNavigate} aria-label={t('browserWorkspace.newTab')} onClick={() => { setDraftTab(true); setAddress(''); input.current?.focus(); }}><Add size={16} /></button>
      <button type='button' className={`${styles.icon} ${styles.close}`} aria-label={t('browserWorkspace.closePanel')} onClick={onClose}><Close size={15} /></button>
    </div>
    <form className={styles.navigation} onSubmit={navigate}>
      <button className={styles.icon} type='button' disabled={!canNavigate || !active?.can_go_back} aria-label={t('browserWorkspace.back')} onClick={() => active && void run({ command: 'back', target: active.target })}><ArrowLeft size={16} /></button>
      <button className={styles.icon} type='button' disabled={!canNavigate || !active?.can_go_forward} aria-label={t('browserWorkspace.forward')} onClick={() => active && void run({ command: 'forward', target: active.target })}><ArrowRight size={16} /></button>
      <button className={styles.icon} type='button' disabled={!canNavigate || !active} aria-label={t(active?.lifecycle === 'loading' ? 'browserWorkspace.stopLoading' : 'browserWorkspace.reload')} onClick={() => active && void run({ command: active.lifecycle === 'loading' ? 'stop_loading' : 'reload', target: active.target })}>{active?.lifecycle === 'loading' ? <Close size={15} /> : <Refresh size={15} />}</button>
      <input ref={input} value={address} disabled={!canNavigate} placeholder={t('browserWorkspace.addressPlaceholder')} aria-label={t('browserWorkspace.address')} onChange={event => { event.target.setCustomValidity(''); setAddress(event.target.value); }} spellCheck={false} autoComplete='off' />
      {limitedActions && <div className={styles.authorityHost} onMouseEnter={showAuthority} onMouseLeave={() => setAuthorityOpen(false)} onFocus={showAuthority} onBlur={() => setAuthorityOpen(false)}>
        <button type='button' className={`${styles.icon} ${styles.authorityTrigger}`} aria-label={t('browserWorkspace.limitedAccessStatus')} aria-describedby={authorityOpen ? authorityHintId : undefined} onKeyDown={event => {
          if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); setAuthorityOpen(false); }
        }}><Info size={15} /></button>
        {authorityOpen && <div id={authorityHintId} className={styles.authorityPopover} role='tooltip'><div className={styles.authorityContent}>
          <strong>{t('browserWorkspace.limitedAccessStatus')}</strong><span>{providerLabel}</span><p>{t('browserWorkspace.limitedAccessHint')}</p>
        </div></div>}
      </div>}
      <div className={styles.menuHost}>
        <button ref={menuButton} type='button' className={styles.icon} aria-label={t('browserWorkspace.menu')} aria-haspopup='menu' aria-expanded={menuOpen} disabled={!snapshot?.runtime || busy || (locked && !snapshot.run.input_gate_failed)} onClick={() => setMenuOpen(open => !open)} onKeyDown={event => {
          if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
          event.preventDefault(); setMenuOpen(true); focusMenuItem(event.key === 'ArrowUp' ? 'last' : 'first');
        }}>⋯</button>
        {menuOpen && <div ref={menu} className={styles.menuPopover} role='menu' aria-label={t('browserWorkspace.menu')} onKeyDown={handleMenuKey}>
          <div className={styles.zoomRow} role='group' aria-label={t('browserWorkspace.zoom')}>
            <span>{t('browserWorkspace.zoom')}</span>
            <div className={styles.zoomActions}>
              <button type='button' role='menuitem' aria-label={t('browserWorkspace.zoomOut')} disabled={!canZoom || zoomOutPercent === undefined} onClick={() => { if (zoomOutPercent !== undefined) changeZoom(zoomOutPercent); }}>−</button>
              <button type='button' role='menuitem' aria-label={t('browserWorkspace.zoomReset')} disabled={!canZoom || zoomPercent === 100} onClick={() => changeZoom(100)}>{zoomPercent}%</button>
              <button type='button' role='menuitem' aria-label={t('browserWorkspace.zoomIn')} disabled={!canZoom || zoomInPercent === undefined} onClick={() => { if (zoomInPercent !== undefined) changeZoom(zoomInPercent); }}>+</button>
            </div>
          </div>
          <div className={styles.menuDivider} />
          <button type='button' role='menuitem' disabled={controlsDisabled || draftTab || !active?.url} onClick={() => { setMenuOpen(false); void copyAddress(); }}>{t('browserWorkspace.copyAddress')}</button>
          <button type='button' role='menuitem' disabled={!canAct || draftTab || !active || !/^https?:\/\//i.test(active.url)} onClick={() => { setMenuOpen(false); if (active && !draftTab) void run({ command: 'open_external', target: active.target }); }}>{t('browserWorkspace.openExternal')}</button>
          <button type='button' role='menuitem' disabled={!canDownload || snapshot?.provider_kind !== 'managed'} onClick={() => { setMenuOpen(false); if (snapshot?.runtime) void run({ command: 'open_downloads', runtime_generation: snapshot.runtime.runtime_generation }); }}>{t('browserWorkspace.openDownloads')}</button>
          <button type='button' role='menuitem' disabled={!canAct || !active || draftTab || snapshot?.provider_kind !== 'managed'} onClick={() => {
            setMenuOpen(false);
            if (!controlsDisabled && active && !draftTab && snapshot?.runtime) setConfirmation({ kind: 'clear_site_data', agentSessionId, generation: snapshot.runtime.runtime_generation });
          }}>{t('browserWorkspace.clearSiteDataTitle')}</button>
          <div className={styles.menuDivider} />
          <div className={styles.menuGroupLabel}>{t('browserWorkspace.downloads')}</div>
          <div className={styles.downloads} role='group' aria-label={t('browserWorkspace.downloads')}>
            {downloads.length === 0 && <div className={styles.downloadEmpty}>{t('browserWorkspace.downloadEmpty')}</div>}
            {[...downloads].reverse().map(download => {
              const target = tabs.find(tab => tab.target.tab_id === download.tab_id)?.target;
              return <div key={download.id} className={styles.download}>
                <div><span className={styles.downloadName} title={download.filename}>{download.filename}</span>
                  <small>{t(`browserWorkspace.downloadStates.${download.state}`)}{download.received_bytes > 0 && ` · ${Math.ceil(download.received_bytes / 1024).toLocaleString()} KB${download.total_bytes !== null ? ` / ${Math.ceil(download.total_bytes / 1024).toLocaleString()} KB` : ''}`}</small></div>
                {download.can_cancel && <button type='button' disabled={!canDownload || !target} aria-label={t('browserWorkspace.downloadCancel', { filename: download.filename })} onClick={event => { event.stopPropagation(); if (target) void run({ command: 'cancel_download', target, download_id: download.id }); }}>{t('browserWorkspace.dialogCancel')}</button>}
              </div>;
            })}
          </div>
          <div className={styles.menuDivider} />
          <button type='button' role='menuitem' disabled={!canAct || tabs.length === 0} onClick={() => { setMenuOpen(false); if (snapshot?.runtime && tabs.length > 0) void run({ command: 'close_all', runtime_generation: snapshot.runtime.runtime_generation }); }}>{t('browserWorkspace.closeAllPages')}</button>
          <button type='button' role='menuitem' disabled={snapshot?.provider_kind !== 'managed'} onClick={() => { setMenuOpen(false); rebuild(); }}>{t('browserWorkspace.rebuild')}</button>
        </div>}
      </div>
    </form>
    {!limitedActions && <div className={styles.status} role='status' aria-live='polite'>
      <span className={styles.state}><span className={styles.dot} data-locked={locked} />{t(failure || snapshot?.run.input_gate_failed ? 'browserWorkspace.notReady' : !snapshot || (attachedProvider && attachedState !== 'connected') ? 'browserWorkspace.opening' : locked ? 'browserWorkspace.agentRunning' : attachedProvider ? 'browserWorkspace.attachedChromeStatus' : 'browserWorkspace.userReady')}</span>
      {snapshot && <span className={styles.provider} data-kind={snapshot.provider_kind}>{providerLabel}</span>}
    </div>}
    {locked && <div className={styles.runLock} role='status'><Lock size={15} /><span><strong>{t('browserWorkspace.agentRunning')}</strong>{t('browserWorkspace.stopHint')}</span></div>}
    {snapshot?.run.input_gate_failed && <div className={styles.runLock} data-error role='alert'><Lock size={15} /><span><strong>{t('browserWorkspace.notReady')}</strong>{t('browserWorkspace.requestFailed')}</span></div>}
    {notice && <div className={styles.notice} role='status'><span>{notice}</span><button type='button' className={styles.icon} aria-label={t('browserWorkspace.dismissNotice')} onClick={() => setNotice('')}><Close size={12} /></button></div>}
    {!permission && active && Boolean(active.blocked_permissions?.length) && !locked && !draftTab && !failure && <div className={styles.permission} role='status'>
      <span>{t('browserWorkspace.permissionRetryHint')}</span>
      {hasSystemPermissionBlock && <a className={styles.permissionLink} href={capabilityPermissionsHref('browser-use')}>{t('browserWorkspace.permissionSettings')}</a>}
      <button type='button' disabled={!canNavigate} onClick={() => void run({ command: 'reload', target: active.target })}>{t('browserWorkspace.reload')}</button>
    </div>}
    {permission && active && !locked && !draftTab && !failure && <div className={styles.permission} role='group' aria-live='polite' aria-label={t('browserWorkspace.permissionTitle')}>
      <span>{t('browserWorkspace.permissionRequest', { origin: permission.origin, permission: t(`browserWorkspace.permissionKinds.${permission.kind}`, { defaultValue: permission.kind }) })}</span>
      <div>{permissionMayNeedSystemAccess && <a className={styles.permissionLink} href={capabilityPermissionsHref('browser-use')}>{t('browserWorkspace.permissionGuide')}</a>}
      <button type='button' disabled={!canAct} onClick={() => void run({ command: 'permission', target: active.target, request_id: permission.request_id, allow: false })}>{t('browserWorkspace.permissionDeny')}</button>
      <button type='button' disabled={!canAct} onClick={() => void run({ command: 'permission', target: active.target, request_id: permission.request_id, allow: true })}>{t('browserWorkspace.permissionAllow')}</button></div>
    </div>}
    <div ref={slot} className={styles.surface} data-browser-surface data-provider={snapshot?.provider_kind} aria-busy={!snapshot && !failure}>
      {dialog && !draftTab && !failure && <WebsiteDialog key={`${agentSessionId}:${dialog.request_id}`} dialog={dialog} locked={Boolean(!canAct || locked || snapshot?.run.input_gate_failed)} busy={busy} onReply={command => void run(command)} />}
      {failure ? <div className={styles.empty} role='alert'><Earth size={28} /><strong>{failureTitle}</strong><p>{failureHint}</p>{failure.retryable && <button type='button' onClick={() => clearFailed ? rebuild() : setRetry(value => value + 1)}>{t(clearFailed ? 'browserWorkspace.rebuild' : 'browserWorkspace.retry')}</button>}</div>
        : !snapshot ? <div className={styles.empty} role='status'><span className={styles.loadingMark} aria-hidden='true' /><strong>{t('browserWorkspace.opening')}</strong><p>{t('browserWorkspace.loadingHint', { defaultValue: "Checking this session's Browser access and provider…" })}</p></div>
          : snapshot.provider_kind === 'attached_chrome' && !draftTab ? <div className={`${styles.empty} ${styles.providerEmpty}`}><Earth size={30} /><strong>{providerLabel}</strong><p>{t('browserWorkspace.attachedChromeHint', { defaultValue: 'Pages stay in your authorized Chrome while the Agent and this panel use the same bound Browser resource.' })}</p></div>
            : (tabs.length === 0 || draftTab) && <div className={styles.empty}><Earth size={30} /><strong>{t('browserWorkspace.start')}</strong><p>{t('browserWorkspace.capabilityStartHint', { defaultValue: 'Enter an address above. This Browser resource belongs to the current session and is shared with its Agent.' })}</p></div>}
    </div>
  </section>;
}
