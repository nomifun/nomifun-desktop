/** NomiRelay configuration is installation transport, not a Mobile identity. */
import { Alert, Button, Input, Modal, Tag } from '@arco-design/web-react';
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ipcBridge } from '@/common';
import type { IRelayPairingStatus } from '@/common/adapter/ipcBridge';

interface RelayPairingPanelProps {
  onStatusChange: (status: IRelayPairingStatus) => void;
}

const EMPTY_STATUS: IRelayPairingStatus = { state: 'disconnected' };
type RelayAction = 'bootstrap' | 'restart' | 'stop' | 'disconnect' | 'forgetLocal';

const ACTION_ERROR_MESSAGES: Record<string, string> = {
  RELAY_INVITE_USED: 'settings.deviceConnections.relayErrorInviteUsed',
  RELAY_INVITE_EXPIRED: 'settings.deviceConnections.relayErrorInviteExpired',
  RELAY_ENROL_EXPIRED: 'settings.deviceConnections.relayErrorEnrolExpired',
  RELAY_CONNECTION_REVOKED: 'settings.deviceConnections.relayErrorConnectionRevoked',
  RELAY_BOOTSTRAP_REJECTED: 'settings.deviceConnections.relayErrorBootstrapRejected',
  RELAY_ENTRY_NOT_READY: 'settings.deviceConnections.relayErrorEntryNotReady',
  RELAY_AGENT_NOT_READY: 'settings.deviceConnections.relayErrorAgentNotReady',
  RELAY_REVOKE_UNCONFIRMED: 'settings.deviceConnections.relayErrorRevokeUnconfirmed',
  RELAY_STATE_UNREADABLE: 'settings.deviceConnections.relayErrorStateUnreadable',
  RELAY_ALREADY_PAIRED: 'settings.deviceConnections.relayErrorAlreadyPaired',
  RELAY_PENDING_EXISTS: 'settings.deviceConnections.relayErrorPendingExists',
  RELAY_ENVELOPE_INVALID: 'settings.deviceConnections.envelopeInvalid',
};

const RelayPairingPanel: React.FC<RelayPairingPanelProps> = ({ onStatusChange }) => {
  const { t, i18n } = useTranslation();
  const [consoleUrl, setConsoleUrl] = useState('');
  const [envelope, setEnvelope] = useState('');
  const [status, setStatus] = useState(EMPTY_STATUS);
  const [loading, setLoading] = useState(false);
  const [pendingAction, setPendingAction] = useState<RelayAction | null>(null);
  const [error, setError] = useState<string | null>(null);
  const actionEpoch = useRef(0);
  const busy = useRef(false);
  const mounted = useRef(true);

  const acceptStatus = useCallback((next: IRelayPairingStatus) => {
    setStatus(next);
    onStatusChange(next);
  }, [onStatusChange]);

  useEffect(() => {
    mounted.current = true;
    let polling = false;
    const poll = async () => {
      if (busy.current || polling) return;
      polling = true;
      const epoch = actionEpoch.current;
      try {
        const next = await ipcBridge.relayPairing.getStatus.invoke();
        if (mounted.current && epoch === actionEpoch.current) acceptStatus(next);
      } catch {
        if (mounted.current && epoch === actionEpoch.current) setError(t('settings.deviceConnections.relayStatusFailed'));
      } finally {
        polling = false;
      }
    };
    void poll();
    const timer = window.setInterval(() => void poll(), 2_000);
    return () => { mounted.current = false; window.clearInterval(timer); };
  }, [acceptStatus, t]);

  const runAction = async (action: RelayAction) => {
    if (busy.current && (action !== 'stop' || pendingAction === 'stop')) return;
    if (action === 'bootstrap' && !/^nomifun-relay-pair:v[12]:/.test(envelope.trim())) {
      setError(t('settings.deviceConnections.envelopeInvalid'));
      return;
    }
    busy.current = true;
    const epoch = ++actionEpoch.current;
    setLoading(true);
    setPendingAction(action);
    setError(null);
    if (action === 'bootstrap' || action === 'restart') acceptStatus({ ...status, state: 'connecting' });
    try {
      const next = action === 'bootstrap'
        ? await ipcBridge.relayPairing.bootstrap.invoke({ pairingEnvelope: envelope.trim() })
        : action === 'restart'
        ? await ipcBridge.relayPairing.restart.invoke()
        : await ipcBridge.relayPairing[action].invoke();
      if (!mounted.current || epoch !== actionEpoch.current) return;
      acceptStatus(next);
      if (action === 'bootstrap' && next.state !== 'error') setEnvelope('');
    } catch {
      if (!mounted.current || epoch !== actionEpoch.current) return;
      try {
        const fresh = await ipcBridge.relayPairing.getStatus.invoke();
        if (!mounted.current || epoch !== actionEpoch.current) return;
        acceptStatus(fresh);
        if (!fresh.actionErrorCode) setError(t('settings.deviceConnections.relayOperationFailed'));
      } catch {
        if (mounted.current && epoch === actionEpoch.current) setError(t('settings.deviceConnections.relayOperationFailed'));
      }
    } finally {
      if (epoch === actionEpoch.current) {
        busy.current = false;
        if (mounted.current) { setLoading(false); setPendingAction(null); }
      }
    }
  };

  const openConsole = () => {
    try {
      const url = new URL(consoleUrl.trim());
      if (!['https:', 'http:'].includes(url.protocol) || url.username || url.password) throw new Error();
      void ipcBridge.shell.openExternal.invoke(url.toString()).catch(() => setError(t('settings.deviceConnections.consoleOpenFailed')));
    } catch { setError(t('settings.deviceConnections.consoleUrlInvalid')); }
  };

  const disconnect = () => Modal.confirm({
    title: t('settings.deviceConnections.relayRemove'),
    content: t(status.protocolVersion === 1 ? 'settings.deviceConnections.relayRemoveLegacyConfirm' : 'settings.deviceConnections.relayRemoveConfirm'),
    okText: t('settings.deviceConnections.relayRemove'),
    cancelText: t('common.cancel'),
    okButtonProps: { status: 'danger' },
    onOk: () => runAction('disconnect'),
  });

  const forgetLocal = () => Modal.confirm({
    title: t('settings.deviceConnections.relayForgetLocal'),
    content: t('settings.deviceConnections.relayForgetLocalConfirm'),
    okText: t('settings.deviceConnections.relayForgetLocal'),
    cancelText: t('common.cancel'),
    okButtonProps: { status: 'danger' },
    onOk: () => runAction('forgetLocal'),
  });

  const actionMessage = status.actionErrorCode
    ? t(ACTION_ERROR_MESSAGES[status.actionErrorCode] ?? 'settings.deviceConnections.relayOperationFailed')
    : null;
  const stateUnreadable = status.errorCode === 'RELAY_STATE_UNREADABLE';
  const canBootstrap = !stateUnreadable && !status.relay && !status.pendingBootstrap;
  const forgettable = stateUnreadable
    || status.actionErrorCode === 'RELAY_REVOKE_UNCONFIRMED'
    || status.actionErrorCode === 'RELAY_STATE_UNREADABLE';
  const genericAlert = error
    || (status.state === 'error' && !status.errorCode && !status.actionErrorCode ? t('settings.deviceConnections.relayOperationFailed') : null);

  const statusLabel = {
    disconnected: 'settings.deviceConnections.relayDisconnected',
    paused: 'settings.deviceConnections.relayPaused',
    connecting: 'settings.deviceConnections.relayConnecting',
    reconnecting: 'settings.deviceConnections.relayReconnecting',
    connected: 'settings.deviceConnections.relayConnected',
    error: 'settings.deviceConnections.relayError',
  }[status.state];

  return (
    <section className='flex flex-col gap-12px' data-testid='relay-configuration'>
      <div className='flex items-center justify-between gap-12px'>
        <strong className='text-14px text-t-primary'>{t('settings.deviceConnections.relayTitle')}</strong>
        <Tag color={status.state === 'connected' ? 'green' : status.state === 'error' ? 'red' : 'gray'}>{t(statusLabel)}</Tag>
      </div>
      <p className='m-0 text-13px leading-20px text-t-secondary'>{t('settings.deviceConnections.relayIntro')}</p>
      <div className='flex gap-8px'>
        <Input aria-label={t('settings.deviceConnections.consoleUrl')} value={consoleUrl} onChange={setConsoleUrl} placeholder='https://relay.example.com/console' />
        <Button onClick={openConsole} disabled={!consoleUrl.trim()}>{t('settings.deviceConnections.consoleOpen')}</Button>
      </div>
      <div className='text-12px text-t-tertiary'>
        {t('settings.deviceConnections.relayDeploymentHint')}{' '}
        <Button type='text' size='mini' onClick={() => void ipcBridge.shell.openExternal.invoke(i18n.language.startsWith('zh') ? 'https://www.nomifun.com/zh/docs/guides/net-infra/' : 'https://www.nomifun.com/docs/guides/net-infra/')}>
          {t('settings.deviceConnections.relayDeploymentGuide')}
        </Button>
      </div>
      {canBootstrap && <label className='flex flex-col gap-6px text-13px text-t-secondary'>
        {t('settings.deviceConnections.envelopeLabel')}
        <Input.Password value={envelope} onChange={setEnvelope} placeholder='nomifun-relay-pair:v2:…' autoComplete='off' maxLength={8192} onPressEnter={() => void runAction('bootstrap')} />
      </label>}
      <div className='flex flex-wrap gap-8px'>
        {(pendingAction === 'bootstrap' || pendingAction === 'restart') && <Button onClick={() => void runAction('stop')}>{t('common.cancel')}</Button>}
        {canBootstrap && <Button type='primary' loading={loading} disabled={!envelope.trim()} onClick={() => void runAction('bootstrap')}>
          {t('settings.deviceConnections.relayConnect')}
        </Button>}
        {!stateUnreadable && (status.relay || status.pendingBootstrap) && <>
          <Button loading={loading} onClick={() => void runAction('restart')}>{t('settings.deviceConnections.relayReconnect')}</Button>
          {status.enabled !== false && status.state !== 'paused' && <Button loading={loading} onClick={() => void runAction('stop')}>{t('settings.deviceConnections.relayPause')}</Button>}
          <Button status='danger' loading={loading} onClick={disconnect}>{t('settings.deviceConnections.relayRemove')}</Button>
        </>}
        {forgettable && <Button status='danger' loading={loading} onClick={forgetLocal}>{t('settings.deviceConnections.relayForgetLocal')}</Button>}
      </div>
      {status.state === 'paused' && <Alert type='info' content={t('settings.deviceConnections.relayPauseHint')} />}
      {status.pendingBootstrap && <Alert type='warning' showIcon content={t('settings.deviceConnections.relayPendingHint')} />}
      {status.protocolVersion === 1 && <Alert type='warning' content={t('settings.deviceConnections.relayUpgradeHint')} />}
      {status.health && <div className='flex flex-wrap gap-12px text-12px text-t-secondary' aria-live='polite'>
        <span>{t(status.health.backendReachable ? 'settings.deviceConnections.backendReady' : 'settings.deviceConnections.backendUnavailable')}</span>
        <span>{t(status.health.tunnelReachable ? 'settings.deviceConnections.tunnelReady' : 'settings.deviceConnections.tunnelUnavailable')}</span>
      </div>}
      {actionMessage && <Alert type='error' showIcon content={actionMessage} />}
      {status.actionErrorCode === 'RELAY_ENTRY_NOT_READY' && !!status.relayReason &&
        <div className='text-12px text-t-tertiary'>{t('settings.deviceConnections.relayReasonDetail', { reason: status.relayReason })}</div>}
      {status.errorCode === 'RELAY_TUNNEL_UNREACHABLE' && <Alert type='warning' content={t('settings.deviceConnections.relayTunnelHint')} />}
      {status.errorCode === 'RELAY_AGENT_EXITED' && <Alert type='warning' content={t('settings.deviceConnections.relayErrorAgentExited')} />}
      {status.errorCode === 'RELAY_AGENT_CLEANUP_UNCONFIRMED' && <Alert type='error' content={t('settings.deviceConnections.relayErrorAgentCleanup')} />}
      {stateUnreadable && status.actionErrorCode !== 'RELAY_STATE_UNREADABLE' &&
        <Alert type='error' showIcon content={t('settings.deviceConnections.relayErrorStateUnreadable')} />}
      {genericAlert && <Alert type='error' showIcon content={genericAlert} />}
      {status.businessUrl && <div className='text-12px text-t-tertiary break-all'>{t('settings.deviceConnections.relayBusinessAddress')}: {status.businessUrl}</div>}
    </section>
  );
};

export default RelayPairingPanel;
