/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import type {
  MiniAppBridgeKvRequest,
  MiniAppBridgeRequest,
  MiniAppSurfaceLaunchDescriptor,
} from '@/common/types/miniAppPlatform';
import { resolveBackendAssetUrl } from '@/renderer/utils/platform';
import { Button, Spin } from '@arco-design/web-react';
import { CloseOne, Refresh } from '@icon-park/react';
import React, {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { useTranslation } from 'react-i18next';
import { miniAppSurfaceAssetPath, shortMiniAppIdentity } from './model';
import styles from './MiniAppSurface.module.css';

interface MiniAppSurfacePanelProps {
  compact?: boolean;
  descriptor: MiniAppSurfaceLaunchDescriptor;
  displayName: string;
  reloading: boolean;
  closing: boolean;
  onReload: () => void;
  onClose: () => void | Promise<void>;
}

interface BridgeLoadState {
  descriptorKey: string;
  portTransferred: boolean;
  handshakeNonce: string | null;
}

const BRIDGE_VERSION = '1.0.0';
const BRIDGE_CHALLENGE_EVENT = 'nomifun-miniapp-bridge-challenge-v1';
const BRIDGE_HANDSHAKE_EVENT = 'nomifun-miniapp-bridge-handshake-v1';
const BRIDGE_CONNECT_EVENT = 'nomifun-miniapp-bridge-connect-v1';
const BRIDGE_RESULT_EVENT = 'nomifun-miniapp-bridge-result-v1';
const BRIDGE_HANDSHAKE_TIMEOUT_MS = 10_000;

function createBridgeNonce(): string {
  const cryptoObject = globalThis.crypto;
  if (!cryptoObject || typeof cryptoObject.getRandomValues !== 'function') {
    throw new Error('WebCrypto getRandomValues is required for MiniApp Bridge handshake');
  }
  const bytes = new Uint8Array(32);
  cryptoObject.getRandomValues(bytes);
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('');
}

function asObject(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function parseKvRequest(value: unknown): MiniAppBridgeKvRequest | null {
  const request = asObject(value);
  const operation = request?.operation;
  const key = request?.key;
  if (typeof key !== 'string' || !key || key.length > 256) return null;
  if (operation === 'get' || operation === 'delete') {
    return { operation, key };
  }
  if (operation === 'set' && Object.hasOwn(request, 'value')) {
    return { operation, key, value: request.value };
  }
  if (operation !== 'compare_and_swap') return null;
  const expectedRevision = request.expected_revision;
  if (
    expectedRevision !== undefined &&
    (!Number.isSafeInteger(expectedRevision) || Number(expectedRevision) < 1)
  ) {
    return null;
  }
  return {
    operation,
    key,
    ...(expectedRevision === undefined
      ? {}
      : { expected_revision: Number(expectedRevision) }),
    ...(Object.hasOwn(request, 'value') ? { value: request.value } : {}),
  };
}

function parseBridgeRequest(value: unknown): MiniAppBridgeRequest | null {
  const request = asObject(value);
  const callId = request?.call_id;
  const target = asObject(request?.target);
  if (typeof callId !== 'string' || !callId.trim() || callId.length > 256) {
    return null;
  }
  if (target?.target === 'service') {
    const method = target.method;
    const payload = asObject(target.payload);
    if (
      typeof method !== 'string' ||
      !method.trim() ||
      method.length > 256 ||
      !payload
    ) {
      return null;
    }
    return {
      call_id: callId,
      target: { target: 'service', method, payload },
    };
  }
  if (target?.target !== 'host_kv') return null;
  const kv = parseKvRequest(target.request);
  return kv
    ? {
        call_id: callId,
        target: { target: 'host_kv', request: kv },
      }
    : null;
}

function bridgeFailure(error: unknown): { code: string; message: string } {
  if (isBackendHttpError(error)) {
    return {
      code: error.code || 'MINIAPP_BRIDGE_FAILED',
      message: error.backendMessage || error.message,
    };
  }
  return {
    code: 'MINIAPP_BRIDGE_FAILED',
    message: error instanceof Error ? error.message : String(error),
  };
}

const MiniAppSurfacePanel: React.FC<MiniAppSurfacePanelProps> = ({
  compact = false,
  descriptor,
  displayName,
  reloading,
  closing,
  onReload,
  onClose,
}) => {
  const { t } = useTranslation();
  const [frameLoading, setFrameLoading] = useState(true);
  const [frameFailed, setFrameFailed] = useState(false);
  const [frameGeneration, setFrameGeneration] = useState(0);
  const iframeRef = useRef<HTMLIFrameElement | null>(null);
  const bridgePortRef = useRef<MessagePort | null>(null);
  const inFlightCallsRef = useRef<Set<string> | null>(null);
  const handshakeCleanupRef = useRef<(() => void) | null>(null);
  const closingRef = useRef(closing);
  const assetPath = useMemo(
    () => miniAppSurfaceAssetPath(descriptor),
    [descriptor]
  );
  const source = resolveBackendAssetUrl(assetPath ?? undefined);
  const bridgeDescriptorKey = JSON.stringify([
    descriptor.miniapp_id,
    descriptor.product_revision,
    descriptor.release_id,
    descriptor.expected_release_digest,
    descriptor.active_release_epoch,
    descriptor.surface_session_id,
    descriptor.surface_generation,
    descriptor.surface_capability,
    descriptor.ui_entrypoint,
    descriptor.kind,
  ]);
  const bridgeLoadRef = useRef<BridgeLoadState>({
    descriptorKey: bridgeDescriptorKey,
    portTransferred: false,
    handshakeNonce: null,
  });

  const clearHandshake = useCallback(() => {
    const cleanup = handshakeCleanupRef.current;
    handshakeCleanupRef.current = null;
    cleanup?.();
  }, []);

  const closeBridge = useCallback(() => {
    clearHandshake();
    const port = bridgePortRef.current;
    bridgePortRef.current = null;
    if (port) {
      port.onmessage = null;
      port.close();
    }
    inFlightCallsRef.current?.clear();
    inFlightCallsRef.current = null;
  }, [clearHandshake]);

  const openBridge = useCallback(
    (frame: Window, nonce: string) => {
      if (
        !bridgeLoadRef.current.portTransferred ||
        bridgeLoadRef.current.handshakeNonce !== null
      ) {
        return;
      }
      if (typeof MessageChannel === 'undefined') return;
      const channel = new MessageChannel();
      const hostPort = channel.port1;
      const inFlightCalls = new Set<string>();
      bridgePortRef.current = hostPort;
      inFlightCallsRef.current = inFlightCalls;
      hostPort.onmessage = (event: MessageEvent<unknown>) => {
        if (bridgePortRef.current !== hostPort) return;
        const raw = asObject(event.data);
        const request = parseBridgeRequest(event.data);
        const rawCallId =
          typeof raw?.call_id === 'string' ? raw.call_id : 'invalid-call';
        if (!request) {
          hostPort.postMessage({
            type: BRIDGE_RESULT_EVENT,
            call_id: rawCallId,
            ok: false,
            error: {
              code: 'MINIAPP_BRIDGE_REQUEST_INVALID',
              message: 'MiniApp Bridge request is invalid',
            },
          });
          return;
        }
        if (closingRef.current) {
          hostPort.postMessage({
            type: BRIDGE_RESULT_EVENT,
            call_id: request.call_id,
            ok: false,
            error: {
              code: 'MINIAPP_BRIDGE_CLOSING',
              message: 'MiniApp Surface is closing',
            },
          });
          return;
        }
        if (inFlightCalls.has(request.call_id)) {
          hostPort.postMessage({
            type: BRIDGE_RESULT_EVENT,
            call_id: request.call_id,
            ok: false,
            error: {
              code: 'MINIAPP_BRIDGE_CALL_DUPLICATE',
              message: 'MiniApp Bridge call_id is already in flight',
            },
          });
          return;
        }
        inFlightCalls.add(request.call_id);
        void ipcBridge.miniapps.bridge
          .invoke({
            miniapp_id: descriptor.miniapp_id,
            surface_capability: descriptor.surface_capability,
            active_release_epoch: descriptor.active_release_epoch,
            expected_release_digest: descriptor.expected_release_digest,
            request,
          })
          .then((result) => {
            if (bridgePortRef.current === hostPort) {
              hostPort.postMessage({
                type: BRIDGE_RESULT_EVENT,
                call_id: request.call_id,
                ok: true,
                result,
              });
            }
          })
          .catch((error: unknown) => {
            if (bridgePortRef.current === hostPort) {
              hostPort.postMessage({
                type: BRIDGE_RESULT_EVENT,
                call_id: request.call_id,
                ok: false,
                error: bridgeFailure(error),
              });
            }
          })
          .finally(() => {
            inFlightCalls.delete(request.call_id);
          });
      };
      hostPort.start();
      try {
        frame.postMessage(
          {
            type: BRIDGE_CONNECT_EVENT,
            version: BRIDGE_VERSION,
            nonce,
          },
          '*',
          [channel.port2]
        );
      } catch {
        channel.port2.close();
        closeBridge();
      }
    },
    [closeBridge, descriptor]
  );

  const beginBridgeHandshake = useCallback(() => {
    closeBridge();
    const frame = iframeRef.current?.contentWindow;
    if (!frame || typeof window === 'undefined') return;

    let nonce: string;
    try {
      nonce = createBridgeNonce();
    } catch (error) {
      console.error('[miniapps] failed to create Surface Bridge nonce', error);
      setFrameFailed(true);
      return;
    }

    const expectedDescriptorKey = bridgeDescriptorKey;
    bridgeLoadRef.current.handshakeNonce = nonce;
    const handleHandshake = (event: MessageEvent<unknown>) => {
      if (
        bridgeLoadRef.current.descriptorKey !== expectedDescriptorKey ||
        bridgeLoadRef.current.portTransferred ||
        bridgeLoadRef.current.handshakeNonce !== nonce ||
        event.source !== frame ||
        // The sandboxed iframe has an opaque "null" origin. Do not accept a
        // same-window message from any other origin.
        event.origin !== 'null'
      ) {
        return;
      }
      const data = asObject(event.data);
      if (
        data?.type !== BRIDGE_HANDSHAKE_EVENT ||
        data.version !== BRIDGE_VERSION ||
        data.nonce !== nonce
      ) {
        return;
      }
      bridgeLoadRef.current.portTransferred = true;
      bridgeLoadRef.current.handshakeNonce = null;
      clearHandshake();
      openBridge(frame, nonce);
    };

    window.addEventListener('message', handleHandshake);
    const timeout = window.setTimeout(() => {
      if (
        bridgeLoadRef.current.descriptorKey === expectedDescriptorKey &&
        bridgeLoadRef.current.handshakeNonce === nonce &&
        !bridgeLoadRef.current.portTransferred
      ) {
        bridgeLoadRef.current.handshakeNonce = null;
        clearHandshake();
        setFrameFailed(true);
      }
    }, BRIDGE_HANDSHAKE_TIMEOUT_MS);
    handshakeCleanupRef.current = () => {
      window.removeEventListener('message', handleHandshake);
      window.clearTimeout(timeout);
    };

    try {
      frame.postMessage(
        {
          type: BRIDGE_CHALLENGE_EVENT,
          version: BRIDGE_VERSION,
          nonce,
        },
        '*'
      );
    } catch (error) {
      console.error('[miniapps] failed to start Surface Bridge handshake', error);
      clearHandshake();
      bridgeLoadRef.current.handshakeNonce = null;
    }
  }, [
    bridgeDescriptorKey,
    clearHandshake,
    closeBridge,
    openBridge,
  ]);

  const revokeBridge = useCallback(() => {
    bridgeLoadRef.current.portTransferred = true;
    bridgeLoadRef.current.handshakeNonce = null;
    closeBridge();
  }, [closeBridge]);

  useLayoutEffect(() => {
    closeBridge();
    bridgeLoadRef.current = {
      descriptorKey: bridgeDescriptorKey,
      portTransferred: false,
      handshakeNonce: null,
    };
    setFrameLoading(true);
    setFrameFailed(false);
    return closeBridge;
  }, [bridgeDescriptorKey, closeBridge]);

  useEffect(() => {
    closingRef.current = closing;
  }, [closing]);

  const handleFrameLoad = useCallback(() => {
    setFrameLoading(false);
    if (bridgeLoadRef.current.descriptorKey !== bridgeDescriptorKey) {
      closeBridge();
      bridgeLoadRef.current = {
        descriptorKey: bridgeDescriptorKey,
        portTransferred: false,
        handshakeNonce: null,
      };
    }
    if (bridgeLoadRef.current.portTransferred) {
      closeBridge();
      return;
    }
    beginBridgeHandshake();
  }, [beginBridgeHandshake, bridgeDescriptorKey, closeBridge]);

  const handleReload = useCallback(() => {
    revokeBridge();
    setFrameLoading(true);
    setFrameFailed(false);
    setFrameGeneration((value) => value + 1);
    onReload();
  }, [onReload, revokeBridge]);

  const handleClose = useCallback(() => {
    void onClose();
  }, [onClose]);

  return (
    <section
      className={styles.surfaceSection}
      aria-labelledby={compact ? undefined : 'miniapp-surface-title'}
      aria-label={compact ? displayName : undefined}
      aria-describedby={compact ? undefined : 'miniapp-surface-hint'}
      aria-busy={reloading || closing || frameLoading || undefined}
    >
      {!compact && <header className={styles.surfaceHeader}>
        <div className={styles.surfaceHeading}>
          <h3 id='miniapp-surface-title' className={styles.sectionTitle}>
            {t('miniApps.surface.title')}
          </h3>
          <p id='miniapp-surface-hint' className={styles.sectionHint}>
            {t('miniApps.surface.fence', {
              epoch: descriptor.active_release_epoch,
              digest: shortMiniAppIdentity(
                descriptor.expected_release_digest,
                10
              ),
            })}
          </p>
        </div>
        <div className={styles.surfaceActions}>
          <Button
            size='small'
            icon={<Refresh theme='outline' size='14' />}
            aria-label={`${t(compact ? 'miniApps.product.retry' : 'miniApps.actions.reloadSurface')}: ${displayName}`}
            loading={reloading}
            disabled={reloading || closing}
            onClick={handleReload}
          >
            {t(compact ? 'miniApps.product.retry' : 'miniApps.actions.reloadSurface')}
          </Button>
          <Button
            size='small'
            icon={<CloseOne theme='outline' size='14' />}
            aria-label={`${t('miniApps.actions.closeSurface')}: ${displayName}`}
            loading={closing}
            disabled={reloading || closing}
            onClick={handleClose}
          >
            {t('miniApps.actions.closeSurface')}
          </Button>
        </div>
      </header>}

      <div className={styles.surfaceViewport}>
        {!source || frameFailed ? (
          <div
            className={styles.surfaceState}
            role='alert'
            aria-live='assertive'
          >
            <span className={styles.stateTitle}>
              {t(compact ? 'miniApps.product.openFailed' : 'miniApps.errors.surfaceFrameTitle')}
            </span>
            <span className={styles.stateBody}>
              {t(compact ? 'miniApps.product.operationFailed' : 'miniApps.errors.surfaceFrameBody')}
            </span>
            <Button
              size='small'
              icon={<Refresh theme='outline' size='14' />}
              aria-label={`${t(compact ? 'miniApps.product.retry' : 'miniApps.actions.reloadSurface')}: ${displayName}`}
              loading={reloading}
              disabled={reloading || closing}
              onClick={handleReload}
            >
              {t(compact ? 'miniApps.product.retry' : 'miniApps.actions.reloadSurface')}
            </Button>
          </div>
        ) : (
          <>
            {frameLoading && (
              <div className={styles.surfaceLoading} role='status'>
                <Spin size={24} />
                <span>{t('miniApps.states.loadingSurface')}</span>
              </div>
            )}
            <iframe
              ref={iframeRef}
              key={`${bridgeDescriptorKey}:${frameGeneration}`}
              className={styles.surfaceFrame}
              style={compact ? { minHeight: 'calc(100dvh - 190px)' } : undefined}
              src={source}
              sandbox='allow-scripts allow-forms'
              referrerPolicy='no-referrer'
              title={t('miniApps.surface.frameTitle', { name: displayName })}
              onLoad={handleFrameLoad}
              onError={() => {
                revokeBridge();
                setFrameLoading(false);
                setFrameFailed(true);
              }}
            />
          </>
        )}
      </div>
    </section>
  );
};

export default MiniAppSurfacePanel;
