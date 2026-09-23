import { useCallback, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { Button, Spin } from '@arco-design/web-react';
import { CloseOne, Refresh } from '@icon-park/react';
import { pluginPlatform } from '@/common/adapter/pluginPlatformBridge';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import type {
  PluginBridgeRequest,
  PluginBridgeResult,
  PluginSurfaceDescriptor,
} from '@/common/types/pluginPlatform';
import { isDesktopShell, resolveBackendAssetUrl } from '@/renderer/utils/platform';
import { pluginSurfaceAssetPath } from './pluginPlatformModel';
import styles from './PluginPlatform.module.css';

const BRIDGE_VERSION = '1.0.0';
const BRIDGE_CHALLENGE_EVENT = 'nomifun-plugin-bridge-challenge-v1';
const BRIDGE_HANDSHAKE_EVENT = 'nomifun-plugin-bridge-handshake-v1';
const BRIDGE_CONNECT_EVENT = 'nomifun-plugin-bridge-connect-v1';
const BRIDGE_RESULT_EVENT = 'nomifun-plugin-bridge-result-v1';
const HANDSHAKE_TIMEOUT_MS = 10_000;

const asObject = (value: unknown): Record<string, unknown> | null =>
  value !== null && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;

export function parsePluginBridgeRequest(value: unknown): PluginBridgeRequest | null {
  const request = asObject(value);
  const callId = request?.call_id;
  const target = asObject(request?.target);
  if (
    typeof callId !== 'string' ||
    !callId.trim() ||
    callId.length > 256 ||
    !target ||
    !['kv', 'db', 'files', 'cache', 'actions', 'host', 'config'].includes(
      String(target.target),
    )
  ) return null;
  return request as unknown as PluginBridgeRequest;
}

function bridgeFailure(callId: string, error: unknown): PluginBridgeResult {
  if (isBackendHttpError(error)) {
    return {
      outcome: 'failure',
      call_id: callId,
      error: {
        code: error.code || 'PLUGIN_BRIDGE_FAILED',
        message: error.backendMessage || error.message,
        outcome_unknown: error.status >= 500,
      },
    };
  }
  return {
    outcome: 'failure',
    call_id: callId,
    error: {
      code: 'PLUGIN_BRIDGE_FAILED',
      message: error instanceof Error ? error.message : String(error),
      outcome_unknown: false,
    },
  };
}

function nonce(): string {
  const bytes = new Uint8Array(32);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('');
}

interface PluginSurfacePanelProps {
  descriptor: PluginSurfaceDescriptor;
  title: string;
  closing?: boolean;
  onReload: () => void;
  onClose: () => void | Promise<void>;
}

export default function PluginSurfacePanel({
  descriptor,
  title,
  closing = false,
  onReload,
  onClose,
}: PluginSurfacePanelProps) {
  const frame = useRef<HTMLIFrameElement | null>(null);
  const port = useRef<MessagePort | null>(null);
  const cleanupHandshake = useRef<(() => void) | null>(null);
  const inFlight = useRef(new Set<string>());
  const [loading, setLoading] = useState(true);
  const [failed, setFailed] = useState(false);
  const [generation, setGeneration] = useState(0);
  const desktopShell = isDesktopShell();
  const assetPath = useMemo(() => pluginSurfaceAssetPath(descriptor), [descriptor]);
  const source = desktopShell ? resolveBackendAssetUrl(assetPath ?? undefined) : undefined;
  const descriptorKey = JSON.stringify(descriptor);

  const closePort = useCallback(() => {
    cleanupHandshake.current?.();
    cleanupHandshake.current = null;
    if (port.current) {
      port.current.onmessage = null;
      port.current.close();
      port.current = null;
    }
    inFlight.current.clear();
  }, []);

  const connect = useCallback((target: Window, challenge: string) => {
    if (!desktopShell) return;
    closePort();
    const channel = new MessageChannel();
    const hostPort = channel.port1;
    port.current = hostPort;
    hostPort.onmessage = (event: MessageEvent<unknown>) => {
      const request = parsePluginBridgeRequest(event.data);
      const raw = asObject(event.data);
      const callId = typeof raw?.call_id === 'string' ? raw.call_id : 'invalid-call';
      if (!request) {
        hostPort.postMessage({
          type: BRIDGE_RESULT_EVENT,
          ...bridgeFailure(callId, new Error('Plugin Bridge request is invalid')),
        });
        return;
      }
      if (inFlight.current.has(request.call_id)) {
        hostPort.postMessage({
          type: BRIDGE_RESULT_EVENT,
          outcome: 'failure',
          call_id: request.call_id,
          error: {
            code: 'PLUGIN_BRIDGE_CALL_DUPLICATE',
            message: 'Plugin Bridge call_id is already in flight',
            outcome_unknown: false,
          },
        });
        return;
      }
      inFlight.current.add(request.call_id);
      void pluginPlatform.surface.bridge.invoke({
        plugin_id: descriptor.plugin_id,
        draft_id: descriptor.draft_id,
        artifact_digest: descriptor.artifact_digest,
        surface_session_id: descriptor.surface_session_id,
        surface_generation: descriptor.surface_generation,
        is_preview: descriptor.is_preview,
        request,
      }).catch((error: unknown) => bridgeFailure(request.call_id, error))
        .then((result) => {
          if (port.current === hostPort) {
            hostPort.postMessage({ type: BRIDGE_RESULT_EVENT, ...result });
          }
        }).finally(() => inFlight.current.delete(request.call_id));
    };
    hostPort.start();
    target.postMessage(
      {
        type: BRIDGE_CONNECT_EVENT,
        version: BRIDGE_VERSION,
        nonce: challenge,
        preview: descriptor.is_preview,
      },
      '*',
      [channel.port2],
    );
  }, [closePort, descriptor, desktopShell]);

  const beginHandshake = useCallback(() => {
    if (!desktopShell) return;
    const target = frame.current?.contentWindow;
    if (!target) return;
    closePort();
    const challenge = nonce();
    let connected = false;
    const receive = (event: MessageEvent<unknown>) => {
      const message = asObject(event.data);
      if (
        connected ||
        event.source !== target ||
        event.origin !== 'null' ||
        message?.type !== BRIDGE_HANDSHAKE_EVENT ||
        message.version !== BRIDGE_VERSION ||
        message.nonce !== challenge
      ) return;
      connected = true;
      cleanupHandshake.current?.();
      cleanupHandshake.current = null;
      connect(target, challenge);
    };
    window.addEventListener('message', receive);
    const timeout = window.setTimeout(() => {
      if (!connected) {
        cleanupHandshake.current?.();
        cleanupHandshake.current = null;
        setFailed(true);
      }
    }, HANDSHAKE_TIMEOUT_MS);
    cleanupHandshake.current = () => {
      window.removeEventListener('message', receive);
      window.clearTimeout(timeout);
    };
    target.postMessage(
      { type: BRIDGE_CHALLENGE_EVENT, version: BRIDGE_VERSION, nonce: challenge },
      '*',
    );
  }, [closePort, connect, desktopShell]);

  useLayoutEffect(() => {
    closePort();
    setLoading(true);
    setFailed(false);
    return closePort;
  }, [closePort, descriptorKey]);

  useLayoutEffect(() => {
    if (closing) closePort();
  }, [closing, closePort]);

  const reload = () => {
    closePort();
    setLoading(true);
    setFailed(false);
    setGeneration((value) => value + 1);
    onReload();
  };

  return (
    <section className={styles.surface} aria-busy={loading || closing || undefined}>
      <header className={styles.surfaceHeader}>
        <div>
          <strong>{title}</strong>
          <div className={styles.muted}>
            {descriptor.is_preview ? 'Temporary preview data' : 'Active Plugin data'}
          </div>
        </div>
        <div className={styles.actions}>
          <Button icon={<Refresh />} disabled={closing} onClick={reload}>Reload</Button>
          <Button icon={<CloseOne />} disabled={closing} onClick={() => void onClose()}>Close</Button>
        </div>
      </header>
      <div className={styles.surfaceViewport}>
        {!source || failed ? (
          <div className={styles.surfaceState} role='alert'>
            <strong>Plugin surface could not be opened</strong>
            <Button onClick={reload}>Retry</Button>
          </div>
        ) : (
          <>
            {loading && <div className={styles.surfaceState}><Spin /><span>Loading Plugin…</span></div>}
            <iframe
              ref={frame}
              key={`${descriptorKey}:${generation}`}
              className={styles.surfaceFrame}
              src={source}
              sandbox='allow-scripts'
              referrerPolicy='no-referrer'
              title={title}
              onLoad={() => {
                setLoading(false);
                beginHandshake();
              }}
              onError={() => {
                closePort();
                setLoading(false);
                setFailed(true);
              }}
            />
          </>
        )}
      </div>
    </section>
  );
}
