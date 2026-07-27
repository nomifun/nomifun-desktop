/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { Alert, Button, Input, Select, Spin, Tag } from '@arco-design/web-react';
import { Left, Refresh, Right } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { ipcBridge } from '@/common';
import type {
  BrowserLaneControlState,
  IBrowserLane,
} from '@/common/browser/browserTypes';
import type { BrowserDisplayMode } from '@/common/browser/browserSettings';
import {
  createBackendWebSocket,
  redactSensitiveText,
} from '@/common/adapter/httpBridge';
import {
  mapBrowserViewerPoint,
  type BrowserViewerConnectionState,
  type BrowserFrameSize,
} from './browserViewerProtocol';
import {
  requestBrowserViewerTakeover,
  returnBrowserViewerControl,
} from './browserViewerActions';
import { BrowserViewerConnectionSession } from './browserViewerConnection';
import {
  browserViewerModifiersFor,
  BrowserViewerPointerController,
  BrowserViewerTextInputController,
  isBrowserViewerReadOnlyIdentity,
  sendBrowserViewerCommand,
  type BrowserViewerKeyInput,
  type BrowserViewerPointerInput,
} from './browserViewerInput';
import { createBrowserViewerInteractionHandlers } from './browserViewerHandlers';
import { BrowserViewerSocketState } from './browserViewerSocket';

interface EmbeddedBrowserViewerProps {
  lane: IBrowserLane;
  /**
   * The persisted presentation preference is resolved by the Browser page.
   * An omitted value keeps the component backwards-compatible for callers
   * which predate the display-mode contract; null means that configuration is
   * still loading and must not start a viewer.
   */
  displayMode?: BrowserDisplayMode | null;
  onInventoryRefresh: () => Promise<void>;
}

export const isEmbeddedBrowserViewerInteractionEnabled = (
  readOnly: boolean,
  connectionState: BrowserViewerConnectionState,
  controlState: BrowserLaneControlState
): boolean =>
  !readOnly && connectionState === 'streaming' && controlState === 'user';

export const isEmbeddedBrowserViewerInputEnabled = (
  readOnly: boolean,
  connectionState: BrowserViewerConnectionState,
  controlState: BrowserLaneControlState
): boolean =>
  !readOnly &&
  connectionState === 'streaming' &&
  (controlState === 'agent' || controlState === 'user' || controlState === 'idle');

export type BrowserViewerStreamPolicy =
  | 'unavailable'
  | 'automatic'
  | 'on_demand'
  | 'external';

/**
 * Resolves whether mounting a lane viewer is allowed to open a stream.
 * Embedded mode is eager, headless mode is explicitly opt-in, and external
 * mode never opens an embedded stream because the managed Primary Host owns
 * its separate window (crawl Hosts remain background-only).
 */
export const resolveBrowserViewerStreamPolicy = (
  displayMode: BrowserDisplayMode | null | undefined,
  lifecycleState: IBrowserLane['lifecycle_state'],
  explicitlyObserved: boolean
): BrowserViewerStreamPolicy => {
  if (!['starting', 'running', 'frozen'].includes(lifecycleState)) {
    return 'unavailable';
  }
  if (displayMode === undefined || displayMode === 'embedded') {
    return 'automatic';
  }
  if (displayMode === null) {
    return 'unavailable';
  }
  if (displayMode === 'headless') {
    return explicitlyObserved ? 'automatic' : 'on_demand';
  }
  return 'external';
};

/**
 * Only input events acquire control automatically. Toolbar navigation and tab
 * commands stay gated until the server confirms a user control lease.
 */
export const canSendEmbeddedBrowserViewerCommand = (
  messageType: unknown,
  controlState: BrowserLaneControlState
): boolean =>
  messageType === 'observe' ||
  messageType === 'takeover' ||
  controlState === 'user' ||
  messageType === 'input';

interface TrackedBrowserViewerKey {
  key: string;
  code: string;
  modifiers: BrowserViewerKeyInput['modifiers'];
}

/**
 * Tracks non-text key presses so a revoked/expired control lease cannot leave
 * a remote modifier or navigation key pressed. Text/IME state is owned by
 * BrowserViewerTextInputController and is reset alongside this tracker.
 */
export class BrowserViewerPressedKeyTracker {
  private readonly pressed = new Map<string, TrackedBrowserViewerKey>();

  observe(input: Record<string, unknown>): void {
    if (input.kind !== 'key') return;
    const action = input.action;
    const key = typeof input.key === 'string' ? input.key : '';
    const code = typeof input.code === 'string' ? input.code : '';
    if (!key && !code) return;
    const id = `${code}\u0000${key}`;
    if (action === 'up') {
      this.pressed.delete(id);
      return;
    }
    if (action !== 'down') return;
    const modifiers =
      input.modifiers && typeof input.modifiers === 'object'
        ? (input.modifiers as BrowserViewerKeyInput['modifiers'])
        : { alt: false, ctrl: false, meta: false, shift: false };
    this.pressed.set(id, { key, code, modifiers: { ...modifiers } });
  }

  releaseAll(sendInput: (input: BrowserViewerKeyInput) => boolean): number {
    const pressed = [...this.pressed.values()];
    this.pressed.clear();
    let released = 0;
    for (const key of pressed) {
      if (
        sendInput({
          kind: 'key',
          action: 'up',
          key: key.key,
          code: key.code,
          modifiers: key.modifiers,
        })
      ) {
        released++;
      }
    }
    return released;
  }

  get pressedCount(): number {
    return this.pressed.size;
  }
}

const EmbeddedBrowserViewer: React.FC<EmbeddedBrowserViewerProps> = ({
  lane,
  displayMode,
  onInventoryRefresh,
}) => {
  const { t } = useTranslation();
  const [connectionState, setConnectionState] =
    useState<BrowserViewerConnectionState>('idle');
  const [frameUrl, setFrameUrl] = useState<string | null>(null);
  const [frameSize, setFrameSize] = useState<BrowserFrameSize | null>(null);
  const [address, setAddress] = useState(lane.url || lane.tabs[0]?.url || '');
  const [activeTabId, setActiveTabId] = useState(
    lane.active_tab_id || lane.tabs.find((tab) => tab.active)?.tab_id || lane.tabs[0]?.tab_id || ''
  );
  const [controlState, setControlState] = useState(lane.control_state);
  const [viewerError, setViewerError] = useState<string | null>(null);
  const [retryKey, setRetryKey] = useState(0);
  const [explicitlyObservedLaneId, setExplicitlyObservedLaneId] = useState<string | null>(null);
  const [returningControl, setReturningControl] = useState(false);
  const authoritativeAddressRef = useRef(address);
  const authoritativeActiveTabIdRef = useRef(activeTabId);
  const socketRef = useRef<WebSocket | null>(null);
  const frameUrlRef = useRef<string | null>(null);
  const imageRef = useRef<HTMLImageElement | null>(null);
  const surfaceRef = useRef<HTMLTextAreaElement | null>(null);
  const focusedRef = useRef(false);
  const frameSizeRef = useRef<BrowserFrameSize | null>(null);
  const socketStateRef = useRef<BrowserViewerSocketState | null>(null);
  const connectionStateRef = useRef(connectionState);
  const controlStateRef = useRef(controlState);
  const pressedKeysRef = useRef<BrowserViewerPressedKeyTracker | null>(null);
  if (!pressedKeysRef.current) {
    pressedKeysRef.current = new BrowserViewerPressedKeyTracker();
  }
  const readOnly = isBrowserViewerReadOnlyIdentity(lane.identity?.mode);
  const readOnlyIdentityLabel =
    lane.identity?.mode === 'anonymous'
      ? t('browser.state.identity.anonymous')
      : lane.identity?.mode === 'authenticated_replica'
        ? t('browser.state.identity.authenticatedReplica')
        : t('browser.viewer.unknownReadOnlyIdentity');
  const readOnlyRef = useRef(readOnly);
  const translationRef = useRef(t);
  translationRef.current = t;

  const streamPolicy = resolveBrowserViewerStreamPolicy(
    displayMode,
    lane.lifecycle_state,
    explicitlyObservedLaneId === lane.lane_id
  );
  const canStream = streamPolicy === 'automatic';
  const interactionEnabled = isEmbeddedBrowserViewerInteractionEnabled(
    readOnly,
    connectionState,
    controlState
  );
  const interactionEnabledRef = useRef(interactionEnabled);
  interactionEnabledRef.current = interactionEnabled;
  const inputEnabled = isEmbeddedBrowserViewerInputEnabled(
    readOnly,
    connectionState,
    controlState
  );
  const inputEnabledRef = useRef(inputEnabled);
  inputEnabledRef.current = inputEnabled;
  const activeTab = useMemo(
    () =>
      lane.tabs.find((tab) => tab.tab_id === activeTabId) ||
      lane.tabs.find((tab) => tab.active) ||
      lane.tabs[0],
    [activeTabId, lane.tabs]
  );

  const send = useCallback((message: Record<string, unknown>): boolean => {
    const socket = socketRef.current;
    if (!socket || socket.readyState !== WebSocket.OPEN) return false;
    socket.send(JSON.stringify(message));
    return true;
  }, []);

  const sendCommand = useCallback(
    (message: Record<string, unknown>): boolean => {
      if (!canSendEmbeddedBrowserViewerCommand(message.type, controlStateRef.current)) {
        return false;
      }
      return sendBrowserViewerCommand(
        send,
        readOnlyRef.current,
        message,
        connectionStateRef.current === 'streaming'
      );
    },
    [send]
  );

  const sendInput = useCallback(
    (input: Record<string, unknown>): boolean => {
      const sent = sendCommand({
        type: 'input',
        input: socketStateRef.current?.bindInput(input) ?? input,
      });
      if (sent) pressedKeysRef.current?.observe(input);
      return sent;
    },
    [sendCommand]
  );

  const focusSurface = useCallback(() => {
    focusedRef.current = true;
    surfaceRef.current?.focus({ preventScroll: true });
  }, []);

  const pointerControllerRef = useRef<BrowserViewerPointerController | null>(null);
  if (!pointerControllerRef.current) {
    pointerControllerRef.current = new BrowserViewerPointerController({
      sendInput: (input: BrowserViewerPointerInput) => sendInput(input),
      getFrame: () =>
        frameSizeRef.current ||
        (imageRef.current?.naturalWidth && imageRef.current?.naturalHeight
          ? {
              width: imageRef.current.naturalWidth,
              height: imageRef.current.naturalHeight,
            }
          : null),
      onEngage: focusSurface,
    });
  }

  const textInputControllerRef = useRef<BrowserViewerTextInputController | null>(null);
  if (!textInputControllerRef.current) {
    textInputControllerRef.current = new BrowserViewerTextInputController(sendCommand);
  }

  const releasePressedInputState = useCallback(() => {
    pointerControllerRef.current?.releaseAll();
    pressedKeysRef.current?.releaseAll((input) => sendInput(input));
    textInputControllerRef.current?.reset();
    focusedRef.current = false;
    if (surfaceRef.current) surfaceRef.current.value = '';
  }, [sendInput]);

  const relinquishLocalInputState = useCallback(() => {
    releasePressedInputState();
    if (surfaceRef.current) surfaceRef.current.blur();
    setAddress(authoritativeAddressRef.current);
    setActiveTabId(authoritativeActiveTabIdRef.current);
  }, [releasePressedInputState]);

  const applyControlState = useCallback(
    (nextControlState: BrowserLaneControlState) => {
      if (
        nextControlState !== 'user' &&
        controlStateRef.current === 'user'
      ) {
        // Release while the old lease is still authoritative. Updating the ref
        // first would make sendCommand reject the final key/pointer-up events.
        relinquishLocalInputState();
      }
      controlStateRef.current = nextControlState;
      setControlState(nextControlState);
    },
    [relinquishLocalInputState]
  );

  const applyConnectionState = useCallback(
    (nextConnectionState: BrowserViewerConnectionState) => {
      if (
        nextConnectionState !== 'streaming' &&
        connectionStateRef.current === 'streaming'
      ) {
        // Clear all local input before a disconnect makes final release
        // commands unsendable.
        relinquishLocalInputState();
      }
      connectionStateRef.current = nextConnectionState;
      setConnectionState(nextConnectionState);
    },
    [relinquishLocalInputState]
  );

  const interactionHandlersRef = useRef<ReturnType<
    typeof createBrowserViewerInteractionHandlers
  > | null>(null);
  if (!interactionHandlersRef.current) {
    interactionHandlersRef.current = createBrowserViewerInteractionHandlers({
      readOnly: () => readOnlyRef.current,
      interactionEnabled: () => inputEnabledRef.current,
      sendInput: (input) => sendInput(input),
      textInput: textInputControllerRef.current,
      pointerInput: pointerControllerRef.current,
    });
  }
  const interactionHandlers = interactionHandlersRef.current;

  frameSizeRef.current = frameSize;

  useEffect(() => {
    const nextAddress = activeTab?.url || lane.url || '';
    authoritativeAddressRef.current = nextAddress;
    setAddress(nextAddress);
  }, [activeTab?.url, lane.url]);

  useEffect(() => {
    const nextActiveTabId =
      lane.active_tab_id ||
      lane.tabs.find((tab) => tab.active)?.tab_id ||
      lane.tabs[0]?.tab_id ||
      '';
    authoritativeActiveTabIdRef.current = nextActiveTabId;
    setActiveTabId(nextActiveTabId);
    applyControlState(lane.control_state);
  }, [
    applyControlState,
    lane.active_tab_id,
    lane.control_state,
    lane.lane_id,
    lane.tabs,
  ]);

  useEffect(() => {
    if (readOnlyRef.current === readOnly) return;
    if (readOnly) {
      // Cleanup must observe the prior writable policy so remote pressed
      // inputs receive their final up before this identity becomes inert.
      readOnlyRef.current = false;
      relinquishLocalInputState();
    }
    readOnlyRef.current = readOnly;
  }, [readOnly, relinquishLocalInputState]);

  useEffect(
    () => () => {
      releasePressedInputState();
    },
    [canStream, lane.lane_id, releasePressedInputState, retryKey]
  );

  useEffect(() => {
    if (!canStream) {
      socketRef.current = null;
      applyConnectionState('idle');
      setViewerError(null);
      return;
    }

    let disposed = false;
    let socket: WebSocket | null = null;
    let socketState: BrowserViewerSocketState | null = null;
    const connection = new BrowserViewerConnectionSession(lane.lane_id, {
      mintViewerToken: (request) => ipcBridge.browserSession.viewerToken.invoke(request),
      createSocket: createBackendWebSocket,
    });
    applyConnectionState('connecting');
    setViewerError(null);
    setFrameSize(null);
    socketStateRef.current = null;

    void connection.connect().then(
      (connectedSocket) => {
        if (disposed || !connectedSocket) return;
        try {
          socket = connectedSocket;
          socketRef.current = socket;
          socket.binaryType = 'arraybuffer';
          socketState = new BrowserViewerSocketState({
            initialControlState: lane.control_state,
            redactError: redactSensitiveText,
            streamFailureMessage: () =>
              translationRef.current('browser.viewer.errors.streamFailed'),
            onConnectionState: applyConnectionState,
            onViewerError: setViewerError,
            onFrameBinding: () => undefined,
            onFrameSize: setFrameSize,
            onAddress: (nextAddress) => {
              authoritativeAddressRef.current = nextAddress;
              setAddress(nextAddress);
            },
            onActiveTabId: (nextActiveTabId) => {
              authoritativeActiveTabIdRef.current = nextActiveTabId;
              setActiveTabId(nextActiveTabId);
            },
            onControlState: applyControlState,
            onInventoryRefresh: () => {
              void onInventoryRefresh().catch(() => undefined);
            },
            onJpegFrame: (bytes) => {
              const blob = new Blob([bytes], { type: 'image/jpeg' });
              const nextUrl = URL.createObjectURL(blob);
              const previousUrl = frameUrlRef.current;
              frameUrlRef.current = nextUrl;
              setFrameUrl(nextUrl);
              if (previousUrl) URL.revokeObjectURL(previousUrl);
            },
          });
          socketStateRef.current = socketState;
          socket.addEventListener('open', () => {
            if (disposed) return;
            socketState?.opened();
            socket?.send(JSON.stringify({ type: 'observe', lane_id: lane.lane_id }));
          });
          socket.addEventListener('message', (event) => {
            if (disposed) return;
            socketState?.received(event.data);
          });
          socket.addEventListener('error', () => {
            if (disposed) return;
            applyConnectionState('failed');
            setViewerError(translationRef.current('browser.viewer.errors.connectionFailed'));
          });
          socket.addEventListener('close', (event) => {
            if (disposed || event.code === 1000) return;
            applyConnectionState('failed');
            setViewerError(
              event.reason
                ? redactSensitiveText(event.reason)
                : translationRef.current('browser.viewer.errors.disconnected')
            );
          });
        } catch (openError) {
          applyConnectionState('failed');
          setViewerError(
            redactSensitiveText(openError instanceof Error ? openError.message : String(openError))
          );
        }
      },
      (tokenError) => {
        if (disposed) return;
        applyConnectionState('failed');
        setViewerError(
          redactSensitiveText(
            translationRef.current('browser.viewer.errors.startFailed', {
              error:
                tokenError instanceof Error
                  ? tokenError.message
                  : String(tokenError),
            })
          )
        );
      }
    );

    return () => {
      disposed = true;
      if (socketRef.current === socket) socketRef.current = null;
      if (socketStateRef.current === socketState) socketStateRef.current = null;
      connection.close();
      const currentUrl = frameUrlRef.current;
      frameUrlRef.current = null;
      if (currentUrl) URL.revokeObjectURL(currentUrl);
      setFrameUrl(null);
    };
  }, [
    applyConnectionState,
    applyControlState,
    canStream,
    lane.lane_id,
    onInventoryRefresh,
    retryKey,
  ]);

  useEffect(() => {
    if (readOnly || connectionState !== 'streaming' || controlState !== 'user') return;
    const heartbeat = setInterval(() => {
      if (focusedRef.current) send({ type: 'heartbeat', lane_id: lane.lane_id });
    }, 10_000);
    return () => clearInterval(heartbeat);
  }, [connectionState, controlState, lane.lane_id, readOnly, send]);

  const handleWheel = useCallback(
    (event: React.WheelEvent<HTMLImageElement>) => {
      if (!inputEnabledRef.current) return;
      const image = imageRef.current;
      const effectiveFrame =
        frameSize ||
        (image?.naturalWidth && image?.naturalHeight
          ? { width: image.naturalWidth, height: image.naturalHeight }
          : null);
      if (!image || !effectiveFrame) return;
      const point = mapBrowserViewerPoint(
        image.getBoundingClientRect(),
        effectiveFrame,
        event.clientX,
        event.clientY
      );
      if (!point) return;
      event.preventDefault();
      sendInput({
        kind: 'wheel',
        x: point.x,
        y: point.y,
        delta_x: event.deltaX,
        delta_y: event.deltaY,
        modifiers: browserViewerModifiersFor(event),
      });
    },
    [frameSize, sendInput]
  );

  const handleReturnControl = useCallback(async () => {
    await returnBrowserViewerControl(lane.lane_id, {
      invoke: (request) => ipcBridge.browserSession.returnControl.invoke(request),
      send,
      refresh: onInventoryRefresh,
      setControlState: applyControlState,
      setReturningControl,
      setViewerError,
      formatError: (error) =>
        redactSensitiveText(error instanceof Error ? error.message : String(error)),
    });
  }, [applyControlState, lane.lane_id, onInventoryRefresh, send]);

  const handleTakeControl = useCallback(() => {
    if (readOnly || connectionState !== 'streaming') return;
    if (!requestBrowserViewerTakeover(sendCommand)) {
      setViewerError(translationRef.current('browser.viewer.errors.disconnected'));
    }
  }, [connectionState, readOnly, sendCommand]);

  const navigate = useCallback(() => {
    if (
      readOnly ||
      connectionState !== 'streaming' ||
      controlStateRef.current !== 'user'
    )
      return;
    const url = address.trim();
    if (!url) return;
    sendCommand({ type: 'navigate', url });
  }, [address, connectionState, readOnly, sendCommand]);

  if (streamPolicy === 'on_demand') {
    return (
      <section
        className='h-300px flex flex-col items-center justify-center gap-10px border border-solid border-[var(--color-border-2)] rd-10px bg-fill-1 px-24px text-center'
        data-browser-viewer-policy='on-demand'
      >
        <div className='text-13px text-t-secondary'>
          {t('browser.viewer.headlessOnDemand')}
        </div>
        <Button onClick={() => setExplicitlyObservedLaneId(lane.lane_id)}>
          {t('browser.viewer.observeNow')}
        </Button>
      </section>
    );
  }

  if (streamPolicy === 'external') {
    return (
      <section
        className='h-300px flex flex-col items-center justify-center gap-8px border border-solid border-[var(--color-border-2)] rd-10px bg-fill-1 px-24px text-center'
        data-browser-viewer-policy='external'
      >
        <div className='text-13px text-t-secondary'>
          {lane.identity?.mode === 'primary'
            ? t('browser.viewer.externalPrimary')
            : t('browser.viewer.externalBackground')}
        </div>
      </section>
    );
  }

  if (streamPolicy === 'unavailable') {
    return (
      <section
        className='h-300px flex items-center justify-center border border-solid border-[var(--color-border-2)] rd-10px bg-fill-1 text-13px text-t-secondary'
        data-browser-viewer-policy='unavailable'
      >
        {displayMode === null
          ? t('browser.viewer.preferenceLoading')
          : lane.lifecycle_state === 'queued'
          ? t('browser.viewer.queuedUnavailable')
          : t('browser.viewer.stateUnavailable')}
      </section>
    );
  }

  const viewerAspectRatio = frameSize
    ? `${frameSize.width} / ${frameSize.height}`
    : '16 / 10';

  return (
    <section className='overflow-hidden rd-14px border border-solid border-[var(--color-border-2)] bg-bg-1 shadow-[0_12px_36px_rgba(15,23,42,0.10)]'>
      <div className='flex flex-col gap-7px p-8px bg-fill-1'>
        <div className='flex min-w-0 items-center gap-6px'>
          <div className='flex shrink-0 items-center gap-1px rd-9px bg-bg-1 p-2px shadow-[0_1px_4px_rgba(15,23,42,0.08)]'>
            <Button
              size='mini'
              type='text'
              aria-label={t('browser.viewer.back')}
              icon={<Left theme='outline' size='13' />}
              disabled={!interactionEnabled}
              onClick={() => sendCommand({ type: 'back' })}
            />
            <Button
              size='mini'
              type='text'
              aria-label={t('browser.viewer.forward')}
              icon={<Right theme='outline' size='13' />}
              disabled={!interactionEnabled}
              onClick={() => sendCommand({ type: 'forward' })}
            />
            <Button
              size='mini'
              type='text'
              aria-label={t('browser.viewer.reload')}
              icon={<Refresh theme='outline' size='13' />}
              disabled={!interactionEnabled}
              onClick={() => sendCommand({ type: 'reload' })}
            />
          </div>
          <Input
            size='mini'
            className='min-w-0 flex-1'
            value={address}
            aria-label={t('browser.viewer.address')}
            disabled={!interactionEnabled}
            onChange={setAddress}
            onPressEnter={navigate}
          />
        </div>
        <div className='flex min-w-0 flex-wrap items-center gap-6px'>
          {lane.tabs.length > 0 && (
            <Select
              size='mini'
              className='min-w-180px max-w-320px flex-1'
              value={activeTabId}
              aria-label={t('browser.viewer.tab')}
              disabled={!interactionEnabled}
              onChange={(tabId) => {
                setActiveTabId(tabId);
                sendCommand({ type: 'select_tab', tab_id: tabId });
              }}
            >
              {lane.tabs.map((tab) => (
                <Select.Option key={tab.tab_id} value={tab.tab_id}>
                  {tab.title || tab.url || tab.tab_id}
                  {tab.crashed ? ` (${t('browser.viewer.crashed')})` : ''}
                </Select.Option>
              ))}
            </Select>
          )}
          <div className='ml-auto flex min-w-0 items-center gap-6px'>
            <Tag color={!readOnly && controlState === 'user' ? 'orange' : 'gray'}>
              {readOnly
                ? `🔒 ${readOnlyIdentityLabel} · ${t('browser.viewer.readOnly')}`
                : controlState === 'user'
                  ? t('browser.viewer.userControl')
                  : t('browser.viewer.agentControl')}
            </Tag>
            {!readOnly && controlState === 'user' ? (
              <Button size='mini' loading={returningControl} onClick={() => void handleReturnControl()}>
                {t('browser.viewer.returnToAgent')}
              </Button>
            ) : !readOnly ? (
              <Button
                size='mini'
                disabled={connectionState !== 'streaming'}
                onClick={handleTakeControl}
              >
                {t('browser.viewer.takeControl')}
              </Button>
            ) : null}
          </div>
        </div>
      </div>

      {viewerError && (
        <Alert
          type='error'
          showIcon
          content={viewerError}
          action={
            <Button
              size='mini'
              onClick={() => {
                releasePressedInputState();
                socketRef.current = null;
                setRetryKey((value) => value + 1);
                setViewerError(null);
              }}
            >
              {t('browser.viewer.retry')}
            </Button>
          }
        />
      )}

      <div className='bg-fill-2 p-8px'>
        <div
          className='relative mx-auto w-full min-h-300px max-h-[min(68vh,760px)] outline-none flex items-center justify-center overflow-hidden bg-[#0b0d12] shadow-[0_10px_30px_rgba(0,0,0,0.28)]'
          style={{ aspectRatio: viewerAspectRatio }}
          role='application'
          aria-label={t('browser.viewer.surfaceAria')}
        >
          <textarea
            ref={surfaceRef}
            defaultValue=''
            readOnly={!inputEnabled}
            disabled={!inputEnabled}
            tabIndex={inputEnabled ? 0 : -1}
            aria-label={t('browser.viewer.surfaceAria')}
            aria-readonly={!inputEnabled}
            className='absolute inset-0 size-full opacity-0 resize-none bg-transparent text-transparent caret-transparent outline-none z-1'
            onFocus={() => {
              if (inputEnabledRef.current) focusedRef.current = true;
            }}
            onBlur={() => {
              releasePressedInputState();
            }}
            onCompositionStart={interactionHandlers.onCompositionStart}
            onCompositionEnd={interactionHandlers.onCompositionEnd}
            onBeforeInput={interactionHandlers.onBeforeInput}
            onPaste={interactionHandlers.onPaste}
            onKeyDown={interactionHandlers.onKeyDown}
            onKeyUp={interactionHandlers.onKeyUp}
          />
          {frameUrl ? (
            <img
              ref={imageRef}
              src={frameUrl}
              alt={t('browser.viewer.frameAlt')}
              draggable={false}
              className={`relative size-full object-contain select-none ${
                !inputEnabled ? 'pointer-events-none' : 'z-2'
              }`}
              onLoad={(event) => {
                if (!frameSize && event.currentTarget.naturalWidth && event.currentTarget.naturalHeight) {
                  setFrameSize({
                    width: event.currentTarget.naturalWidth,
                    height: event.currentTarget.naturalHeight,
                  });
                }
              }}
              onPointerMove={interactionHandlers.onPointerMove}
              onPointerDown={interactionHandlers.onPointerDown}
              onPointerUp={interactionHandlers.onPointerUp}
              onPointerCancel={interactionHandlers.onPointerCancel}
              onLostPointerCapture={interactionHandlers.onLostPointerCapture}
              onWheel={handleWheel}
              onContextMenu={(event) => event.preventDefault()}
            />
          ) : connectionState === 'connecting' || connectionState === 'streaming' ? (
            <Spin
              dot
              tip={
                connectionState === 'connecting'
                  ? t('browser.viewer.connecting')
                  : t('browser.viewer.waitingFrame')
              }
            />
          ) : (
            <div className='text-13px text-white/65'>{t('browser.viewer.idle')}</div>
          )}
        </div>
      </div>
      <div className='flex items-center justify-end bg-fill-1 px-10px py-6px text-10px text-t-tertiary'>
        {t('browser.viewer.frameStatus', {
          frame: frameSize
            ? `${frameSize.width}×${frameSize.height}`
            : t('browser.viewer.jpegLiveView'),
          mode: t('browser.viewer.latestFrameOnly'),
        })}
      </div>
    </section>
  );
};

export default EmbeddedBrowserViewer;
