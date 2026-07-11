import React, { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { isTauriRuntime } from '@/common/adapter/tauriRuntime';
import { injectCompanionCustomCss } from '@/renderer/utils/theme/applyCustomCss';
import {
  MEMORY_PANEL_EVENTS,
  type MemoryPanelActionAckPayload,
  type MemoryPanelActivatePayload,
  type MemoryPanelClosePayload,
  type MemoryPanelClosedPayload,
  type MemoryPanelMeasuredPayload,
  type MemoryPanelPhase,
  type MemoryPanelPresentPayload,
  type MemoryPanelProbePayload,
  type MemoryPanelReadyPayload,
  type MemoryPanelSnapshotPayload,
  type MemoryPanelVisiblePayload,
} from '@/renderer/pages/companion/memoryPanelProtocol';
import { emitToWindow, hideMemoryPanelWindow, listenCurrentWindow } from '@/renderer/pages/companion/memoryPanelShell';
import './memoryPanel.css';

const MemoryPanelPage: React.FC = () => {
  const { t } = useTranslation();
  const [snapshot, setSnapshot] = useState<MemoryPanelSnapshotPayload | null>(null);
  const [phase, setPhase] = useState<MemoryPanelPhase>('closed');
  const [placement, setPlacement] = useState('above');
  const cardRef = useRef<HTMLElement | null>(null);
  const snapshotRef = useRef(snapshot);
  const phaseRef = useRef(phase);
  const closeTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  snapshotRef.current = snapshot;
  phaseRef.current = phase;

  const finishClose = useCallback(async (payload: MemoryPanelClosePayload) => {
    const current = snapshotRef.current;
    if (!current || current.requestId !== payload.requestId || phaseRef.current === 'closing') return;
    setPhase('closing');
    phaseRef.current = 'closing';
    if (closeTimerRef.current) clearTimeout(closeTimerRef.current);
    const reduced = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false;
    closeTimerRef.current = setTimeout(async () => {
      await hideMemoryPanelWindow(payload.requestId).catch(() => false);
      const closed: MemoryPanelClosedPayload = { ...payload, restoreFocus: payload.reason === 'escape' };
      await emitToWindow(current.ownerWindowLabel, MEMORY_PANEL_EVENTS.closed, closed).catch(() => {});
      setSnapshot(null);
      setPhase('closed');
    }, reduced ? 0 : 140);
  }, []);

  useLayoutEffect(() => {
    const current = snapshot;
    const card = cardRef.current;
    if (!current || !card) return;
    const rect = card.getBoundingClientRect();
    const measured: MemoryPanelMeasuredPayload = {
      requestId: current.requestId,
      ownerWindowLabel: current.ownerWindowLabel,
      width: Math.ceil(rect.width),
      height: Math.ceil(rect.height),
    };
    void emitToWindow(current.ownerWindowLabel, MEMORY_PANEL_EVENTS.measured, measured);
  }, [snapshot]);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    void Promise.all([
      listenCurrentWindow<MemoryPanelProbePayload>(MEMORY_PANEL_EVENTS.probe, (payload) => {
        const ready: MemoryPanelReadyPayload = payload;
        void emitToWindow(payload.ownerWindowLabel, MEMORY_PANEL_EVENTS.ready, ready);
      }),
      listenCurrentWindow<MemoryPanelSnapshotPayload>(MEMORY_PANEL_EVENTS.snapshot, (payload) => {
        if (closeTimerRef.current) clearTimeout(closeTimerRef.current);
        document.documentElement.setAttribute('data-theme', payload.theme);
        document.body.setAttribute('arco-theme', payload.theme);
        injectCompanionCustomCss(payload.customCss);
        setSnapshot(payload);
        setPhase('preparing');
      }),
      listenCurrentWindow<MemoryPanelPresentPayload>(MEMORY_PANEL_EVENTS.present, (payload) => {
        if (snapshotRef.current?.requestId !== payload.requestId) return;
        setPlacement(payload.placement);
        setPhase('opening');
      }),
      listenCurrentWindow<MemoryPanelVisiblePayload>(MEMORY_PANEL_EVENTS.visible, (payload) => {
        if (snapshotRef.current?.requestId !== payload.requestId) return;
        setPhase('open');
        phaseRef.current = 'open';
        requestAnimationFrame(() => {
          void import('@tauri-apps/api/window').then(({ getCurrentWindow }) =>
            getCurrentWindow().isFocused().then((focused) => {
              if (!focused) void finishClose({ requestId: payload.requestId, reason: 'blur' });
            })
          );
        });
      }),
      listenCurrentWindow<MemoryPanelClosePayload>(MEMORY_PANEL_EVENTS.close, (payload) => void finishClose(payload)),
      listenCurrentWindow<MemoryPanelActionAckPayload>(MEMORY_PANEL_EVENTS.actionAck, (payload) => {
        if (payload.ok && snapshotRef.current?.requestId === payload.requestId) void finishClose({ requestId: payload.requestId, reason: 'activation' });
      }),
    ]).then((items) => disposed ? items.forEach((unlisten) => unlisten()) : unlisteners.push(...items));

    let unlistenFocus: (() => void) | undefined;
    void import('@tauri-apps/api/window').then(({ getCurrentWindow }) => getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (focused || phaseRef.current !== 'open') return;
      const current = snapshotRef.current;
      if (current) void finishClose({ requestId: current.requestId, reason: 'blur' });
    })).then((unlisten) => { if (disposed) unlisten(); else unlistenFocus = unlisten; });
    return () => {
      disposed = true;
      unlisteners.forEach((unlisten) => unlisten());
      unlistenFocus?.();
      if (closeTimerRef.current) clearTimeout(closeTimerRef.current);
    };
  }, [finishClose]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const current = snapshotRef.current;
      if (event.key === 'Escape' && current) void finishClose({ requestId: current.requestId, reason: 'escape' });
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [finishClose]);

  if (!snapshot) return null;
  const activate = (suggestionId: string) => {
    const payload: MemoryPanelActivatePayload = { requestId: snapshot.requestId, ownerWindowLabel: snapshot.ownerWindowLabel, suggestionId };
    void emitToWindow(snapshot.ownerWindowLabel, MEMORY_PANEL_EVENTS.activate, payload);
  };

  return (
    <main className={`nomi-memory-panel nomi-memory-panel--${phase} nomi-memory-panel--${placement}`}>
      <section ref={cardRef} className='nomi-memory-panel__card' role='dialog' aria-label={t('nomi.tabs.suggestions')}>
        <div className='nomi-memory-panel__list'>
          {snapshot.suggestions.map((suggestion) => (
            <button key={suggestion.id} type='button' className='nomi-memory-panel__item' onClick={() => activate(suggestion.id)}>
              <span className='nomi-memory-panel__title'>{suggestion.title}</span>
              <span className='nomi-memory-panel__body'>{suggestion.body}</span>
            </button>
          ))}
        </div>
      </section>
    </main>
  );
};

export default MemoryPanelPage;
