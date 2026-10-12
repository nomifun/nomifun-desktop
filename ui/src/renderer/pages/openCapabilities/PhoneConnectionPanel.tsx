import React, { Suspense, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Alert, Button, Modal, Radio, Select, Spin, Tag } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import { ipcBridge } from '@/common';
import type { IRelayPairingStatus } from '@/common/adapter/ipcBridge';
import { buildMobilePairingUrl, connections, type ConnectionDevice, type ConnectionInfo, type ConnectionPairing, type ConnectionPairingStatus } from '@/common/adapter/connections';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import RelayPairingPanel from '@/renderer/components/layout/Sider/RelayPairingPanel';
import CopyIconButton from '@/renderer/components/base/CopyIconButton';
import { getWebuiQrBaseUrls } from '@/renderer/components/layout/Sider/webuiQrLinks';
import { useWebuiServer } from '@/renderer/hooks/context/WebuiServerContext';

const QRCode = React.lazy(async () => ({ default: (await import('qrcode.react')).QRCodeSVG }));
const PANEL = 'rd-12px border border-solid border-arco-2 bg-fill-0 p-16px';
const DEVICE_LIST_POLL_MS = 15_000;

const PhoneConnectionPanel: React.FC = () => {
  const { t } = useTranslation();
  const { status, running, starting, lifecycleSupported, accessUrls, start } = useWebuiServer();
  const [path, setPath] = useState<'lan' | 'relay'>('lan');
  const [info, setInfo] = useState<ConnectionInfo | null>(null);
  const [devices, setDevices] = useState<ConnectionDevice[]>([]);
  const [deviceError, setDeviceError] = useState(false);
  const [managementAllowed, setManagementAllowed] = useState<boolean | null>(null);
  const [infoError, setInfoError] = useState(false);
  const [baseUrl, setBaseUrl] = useState('');
  const [pairing, setPairing] = useState<ConnectionPairing | null>(null);
  const [pairStatus, setPairStatus] = useState<ConnectionPairingStatus | null>(null);
  const [pairStatusError, setPairStatusError] = useState<string | null>(null);
  const [pairInvalidated, setPairInvalidated] = useState(false);
  const [relay, setRelay] = useState<IRelayPairingStatus>({ state: 'disconnected' });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [now, setNow] = useState(Date.now());
  const actionEpoch = useRef(0);
  const mounted = useRef(true);
  const browserBaseUrl = typeof window !== 'undefined' ? window.location.origin : '';

  const urls = useMemo(() => !running || !status?.allowRemote ? [] :
    getWebuiQrBaseUrls(status, accessUrls, status.port).filter((candidate) => {
      const host = new URL(candidate).hostname;
      return host !== 'localhost' && host !== '::1' && host !== '[::1]' && !host.startsWith('127.');
    }), [running, status, accessUrls]);
  useEffect(() => { setBaseUrl((current) => urls.includes(current) ? current : urls[0] ?? ''); }, [urls]);

  const refreshDevices = useCallback(async () => {
    try {
      const next = await connections.devices.invoke();
      if (mounted.current) { setDevices(next); setDeviceError(false); setManagementAllowed(true); }
    } catch (cause) { if (mounted.current) {
      setDeviceError(true);
      if (isBackendHttpError(cause) && cause.status === 403) setManagementAllowed(false);
    } }
  }, []);

  useEffect(() => {
    mounted.current = true;
    void connections.info.invoke().then((next) => {
      if (mounted.current) { setInfo(next); setInfoError(false); }
    }).catch(() => { if (mounted.current) setInfoError(true); });
    void refreshDevices();
    let polling = false;
    const clock = window.setInterval(() => setNow(Date.now()), 1_000);
    const timer = window.setInterval(async () => {
      if (polling) return;
      polling = true;
      try { await refreshDevices(); } finally { polling = false; }
    }, DEVICE_LIST_POLL_MS);
    return () => { mounted.current = false; window.clearInterval(timer); window.clearInterval(clock); };
  }, [lifecycleSupported, refreshDevices]);

  const ticketId = path === 'lan' ? pairing?.ticket_id : relay.ticketId;
  const ticketExpiresAt = path === 'lan' ? pairing?.expires_at_ms : relay.qrExpiresAtMs;
  useEffect(() => {
    setPairStatus(null);
    setPairStatusError(null);
    setPairInvalidated(false);
    if (!ticketId) return;
    let active = true;
    let polling = false;
    let terminal = false;
    let timer: number | undefined;
    const finish = () => { terminal = true; if (timer !== undefined) window.clearInterval(timer); };
    const expire = () => {
      setPairStatus({ ticket_id: ticketId, state: 'expired', device_id: null, expires_at_ms: ticketExpiresAt ?? Date.now() });
      setPairStatusError(null);
      finish();
    };
    const poll = async () => {
      if (polling || terminal || !active) return;
      if (ticketExpiresAt && Date.now() >= ticketExpiresAt) { expire(); return; }
      polling = true;
      try {
        const next = await connections.pairingStatus.invoke(ticketId);
        if (active) {
          setPairStatus(next);
          setPairStatusError(null);
          if (next.state === 'used' || next.state === 'failed' || next.state === 'expired') finish();
          if (next.state === 'used' && next.device_id) void refreshDevices();
        }
      } catch (cause) { if (active) {
        if (isBackendHttpError(cause) && cause.status === 404) {
          if (ticketExpiresAt && Date.now() >= ticketExpiresAt) expire();
          else { setPairInvalidated(true); setPairStatusError(null); finish(); }
        } else setPairStatusError(t('settings.deviceConnections.pairStatusFailed'));
      } }
      finally { polling = false; }
    };
    void poll();
    if (!terminal) timer = window.setInterval(() => void poll(), 2_000);
    return () => { active = false; if (timer !== undefined) window.clearInterval(timer); };
  }, [ticketId, ticketExpiresAt, refreshDevices, t]);

  useEffect(() => {
    if (lifecycleSupported && !running) { setPairing(null); if (path === 'lan') setPairStatus(null); }
  }, [running, path, lifecycleSupported]);

  const generate = async () => {
    if (busy || managementAllowed !== true || !info) return;
    const epoch = ++actionEpoch.current;
    setBusy(true);
    setError(null);
    try {
      if (path === 'lan') {
        if (lifecycleSupported && (!running || !baseUrl)) return;
        const next = await connections.pairing.invoke({});
        if (mounted.current && epoch === actionEpoch.current) { setPairing(next); setPairStatus(null); }
      } else {
        const next = await ipcBridge.relayPairing.refreshQr.invoke();
        if (mounted.current && epoch === actionEpoch.current) {
          setRelay(next);
          setPairStatus(null);
          if (next.state === 'error') setError(t('settings.deviceConnections.pairGenerateFailed'));
        }
      }
    } catch { if (mounted.current && epoch === actionEpoch.current) setError(t('settings.deviceConnections.pairGenerateFailed')); }
    finally { if (mounted.current && epoch === actionEpoch.current) setBusy(false); }
  };

  const selectPath = (next: string | number | boolean) => {
    actionEpoch.current += 1;
    setBusy(false);
    setPath(next as 'lan' | 'relay');
    setError(null);
    setPairStatus(null);
  };

  const revoke = (device: ConnectionDevice) => Modal.confirm({
    title: t('settings.deviceConnections.deviceRemove'),
    content: t('settings.deviceConnections.deviceRemoveConfirm', { name: device.device_name }),
    okText: t('settings.deviceConnections.deviceRemove'),
    cancelText: t('common.cancel'),
    okButtonProps: { status: 'danger' },
    onOk: async () => {
      try { await connections.revoke.invoke(device.device_id); await refreshDevices(); }
      catch { setError(t('settings.deviceConnections.deviceRemoveFailed')); }
    },
  });

  const expiresAt = ticketExpiresAt;
  const seconds = expiresAt ? Math.max(0, Math.ceil((expiresAt - now) / 1000)) : 0;
  const used = Boolean(ticketId && pairStatus?.state === 'used' && pairStatus.device_id);
  const failed = Boolean(ticketId && (pairStatus?.state === 'failed' || (pairStatus?.state === 'used' && !pairStatus.device_id)));
  const expired = Boolean(ticketId && !failed && (seconds === 0 || pairStatus?.state === 'expired'));
  const pairingBaseUrl = lifecycleSupported ? baseUrl : browserBaseUrl;
  const qrUrl = path === 'lan' ? pairing && pairingBaseUrl ? buildMobilePairingUrl(pairingBaseUrl, pairing.qr_token) : null : relay.pairUrl ?? null;
  const ready = managementAllowed === true && (path === 'lan' ? lifecycleSupported ? running && Boolean(baseUrl) : Boolean(browserBaseUrl) : relay.state === 'connected' && relay.protocolVersion === 2 && relay.health?.tunnelReachable === true);
  const activeDevices = devices.filter((device) => device.revoked_at_ms === null);
  const pairedDevice = devices.find((device) => device.device_id === pairStatus?.device_id);
  const receiptRemoved = used && Boolean(pairedDevice && pairedDevice.revoked_at_ms !== null);

  return (
    <div className='flex flex-col gap-16px' data-testid='phone-connection-panel'>
      <section className={PANEL}>
        <div className='flex items-center justify-between gap-12px'>
          <div>
            <div className='text-12px text-t-tertiary'>{t('settings.deviceConnections.thisDesktop')}</div>
            <h2 className='m-0 mt-4px text-17px font-600 text-t-primary'>{info?.desktop_name || t('settings.deviceConnections.desktopLoading')}</h2>
          </div>
          <Tag>{t('settings.deviceConnections.desktopAuthority')}</Tag>
        </div>
        <p className='mb-0 text-13px leading-20px text-t-secondary'>{t('settings.deviceConnections.phoneIntro')}</p>
        {infoError && <Alert className='mt-12px' type='error' content={t('settings.deviceConnections.infoFailed')} />}
        {!lifecycleSupported && <Alert className='mt-12px' type='info' content={t('settings.deviceConnections.browserOwnerHint')} />}
        {managementAllowed === false && <Alert className='mt-12px' type='warning' content={t('settings.deviceConnections.ownerRequired')} />}
      </section>

      <>
        <section className={PANEL}>
          <h2 className='m-0 mb-12px text-15px font-600 text-t-primary'>{t(lifecycleSupported ? 'settings.deviceConnections.choosePath' : 'settings.deviceConnections.confirmAddress')}</h2>
          {lifecycleSupported && <Radio.Group type='button' value={path} onChange={selectPath}>
            <Radio value='lan'>{t('settings.deviceConnections.sameNetwork')}</Radio>
            <Radio value='relay'>{t('settings.deviceConnections.crossNetwork')}</Radio>
          </Radio.Group>}
          {lifecycleSupported && <p className='mb-0 text-13px leading-20px text-t-secondary'>{t(path === 'lan' ? 'settings.deviceConnections.lanHint' : 'settings.deviceConnections.crossHint')}</p>}
          <div className='mt-16px'>
            {!lifecycleSupported ? <>
              <code className='block break-all text-13px text-t-primary' data-testid='browser-pairing-address'>{browserBaseUrl}</code>
              <p className='mb-0 text-12px leading-18px text-t-tertiary'>{t('settings.deviceConnections.browserAddressHint')}</p>
            </> : path === 'lan' ? <>
              {!running && <Alert type='info' content={t('settings.deviceConnections.lanOff')} action={<Button type='primary' loading={starting} onClick={() => void start()}>{t('settings.deviceConnections.lanEnable')}</Button>} />}
              {running && urls.length === 0 && <Alert type='warning' content={t('settings.deviceConnections.noLanAddress')} />}
              {running && urls.length > 0 && <label className='flex flex-col gap-6px text-13px text-t-secondary'>
                {t('settings.deviceConnections.addressLabel')}
                <Select value={baseUrl} onChange={(value) => { setBaseUrl(value); setPairing(null); setError(null); }} options={urls.map((url) => ({ value: url, label: url }))} />
                <span className='text-12px text-t-tertiary'>{t('settings.deviceConnections.addressHint')}</span>
              </label>}
            </> : <RelayPairingPanel onStatusChange={setRelay} />}
          </div>
        </section>

        <section className={PANEL} data-testid='mobile-pairing'>
          <div className='flex flex-wrap items-center justify-between gap-12px'>
            <h2 className='m-0 text-15px font-600 text-t-primary'>{t('settings.deviceConnections.scanStep')}</h2>
            <Button data-testid='generate-phone-pairing' type='primary' disabled={!ready || !info} loading={busy} onClick={() => void generate()}>{t(qrUrl ? 'settings.deviceConnections.pairRefresh' : 'settings.deviceConnections.pairGenerate')}</Button>
          </div>
          <p className='text-13px leading-20px text-t-secondary'>{t('settings.deviceConnections.scanHint')}</p>
          <p className='text-12px leading-18px text-t-tertiary'>{t('settings.deviceConnections.phonePermissions')}</p>
          {receiptRemoved ? <div data-testid='phone-pairing-revoked'><Alert type='warning' showIcon content={t('settings.deviceConnections.pairRemoved')} /></div>
            : used ? <div data-testid='phone-pairing-completed'><Alert type='success' showIcon content={t('settings.deviceConnections.pairUsed', { name: pairedDevice?.device_name || t('settings.deviceConnections.phoneName') })} /></div>
            : pairInvalidated ? <Alert type='warning' showIcon content={t('settings.deviceConnections.pairInvalidated')} />
            : failed ? <Alert type='warning' showIcon content={t(pairStatus?.failure_code === 'DEVICE_LIMIT_REACHED' ? 'settings.deviceConnections.pairFailedLimit' : 'settings.deviceConnections.pairFailed')} />
            : expired ? <Alert type='warning' showIcon content={t('settings.deviceConnections.pairExpired')} />
              : ready && qrUrl && ticketId ? <div className='flex items-start gap-20px'>
                <div className='shrink-0 rd-10px bg-white p-10px' data-testid='phone-pairing-qr'><Suspense fallback={<Spin />}><QRCode value={qrUrl} size={172} level='M' /></Suspense></div>
                <div className='flex min-w-0 flex-col gap-10px text-13px text-t-secondary' aria-live='polite'>
                  <strong>{t('settings.deviceConnections.pairWaiting')}</strong>
                  <span>{t('settings.deviceConnections.pairCountdown', { time: `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}` })}</span>
                  <span className='text-12px leading-18px text-t-tertiary'>{t('settings.deviceConnections.pairSingleUse')}</span>
                  <span className='inline-flex items-center gap-6px'><CopyIconButton text={qrUrl} size={16} tooltip={t('settings.deviceConnections.pairCopy')} />{t('settings.deviceConnections.pairCopy')}</span>
                  <details className='w-full'>
                    <summary className='cursor-pointer text-12px text-primary-6'>{t('settings.deviceConnections.pairManualCopy')}</summary>
                    <label className='mt-8px flex flex-col gap-6px text-12px'>
                      {t('settings.deviceConnections.pairLinkLabel')}
                      <textarea data-testid='phone-pairing-link' aria-label={t('settings.deviceConnections.pairLinkLabel')} readOnly value={qrUrl} rows={3} className='w-full box-border rd-6px border border-solid border-arco-2 bg-fill-1 px-8px py-6px font-mono text-12px text-t-secondary resize-none' />
                    </label>
                  </details>
                </div>
              </div> : <Alert type='info' content={t(ready ? 'settings.deviceConnections.pairReady' : 'settings.deviceConnections.pairNotReady')} />}
          {error && <Alert className='mt-12px' type='error' showIcon content={error} />}
          {pairStatusError && <Alert className='mt-12px' type='warning' showIcon content={pairStatusError} />}
        </section>
      </>

      <section className={PANEL} data-testid='connection-devices'>
        <div className='flex items-center justify-between gap-12px'>
          <h2 className='m-0 text-15px font-600 text-t-primary'>{t('settings.deviceConnections.devicesTitle', { count: activeDevices.length })}</h2>
          <Button size='small' onClick={() => void refreshDevices()}>{t('settings.deviceConnections.devicesRefresh')}</Button>
        </div>
        <p className='text-12px leading-18px text-t-tertiary'>{t('settings.deviceConnections.devicesHint')}</p>
        {deviceError ? <Alert type={managementAllowed === false ? 'warning' : 'error'} content={t(managementAllowed === false ? 'settings.deviceConnections.ownerRequired' : 'settings.deviceConnections.devicesFailed')} /> : activeDevices.length === 0
          ? <p className='text-13px text-t-secondary'>{t('settings.deviceConnections.devicesEmpty')}</p>
          : <div className='flex flex-col gap-8px'>{activeDevices.map((device) => <div key={device.device_id} data-testid='paired-phone' data-device-id={device.device_id} className='flex items-center justify-between gap-12px rd-8px bg-fill-1 px-12px py-10px'>
            <div className='min-w-0'>
              <div className='truncate text-13px font-500 text-t-primary'>{device.device_name}</div>
              <div className='mt-3px text-12px text-t-tertiary'>{t('settings.deviceConnections.deviceLastSeen', { time: new Date(device.last_seen_at_ms).toLocaleString() })}</div>
            </div>
            <Button data-testid='remove-phone' status='danger' size='small' onClick={() => revoke(device)}>{t('settings.deviceConnections.deviceRemove')}</Button>
          </div>)}</div>}
      </section>
    </div>
  );
};

export default PhoneConnectionPanel;
