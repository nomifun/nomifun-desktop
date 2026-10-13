import { Button, Select, Spin } from '@arco-design/web-react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { attachedChromeClient, type AttachedChromeClient, type AttachedChromeSnapshot } from './attachedChromeClient';
import styles from './AgentResourcePicker.module.css';

type Props = {
  value?: string;
  onChange: (resourceId: 'managed-browser' | 'attached-chrome') => void;
  onReadinessChange: (ready: boolean) => void;
  disabled?: boolean;
  client?: AttachedChromeClient;
};

export default function BrowserResourceField({ value = 'managed-browser', onChange, onReadinessChange, disabled = false, client = attachedChromeClient }: Props) {
  const { t } = useTranslation();
  const [snapshot, setSnapshot] = useState<AttachedChromeSnapshot | null>(null);
  const [pending, setPending] = useState<'read' | 'connect' | 'disconnect' | null>('read');
  const [error, setError] = useState(false);
  const generation = useRef(0);
  const connected = snapshot?.state === 'connected';
  const ready = value === 'managed-browser'
    ? pending !== 'connect' && pending !== 'disconnect'
    : !pending && value === 'attached-chrome' && connected && !error;

  useEffect(() => { onReadinessChange(ready); }, [onReadinessChange, ready]);
  useEffect(() => {
    const current = ++generation.current;
    setPending('read');
    setError(false);
    void client.snapshot().then((next) => {
      if (generation.current === current) setSnapshot(next);
    }).catch(() => {
      if (generation.current === current) setError(true);
    }).finally(() => {
      if (generation.current === current) setPending(null);
    });
    return () => { generation.current += 1; };
  }, [client]);

  const act = async (operation: 'read' | 'connect' | 'disconnect') => {
    if (disabled || pending) return;
    const current = ++generation.current;
    setPending(operation);
    setError(false);
    try {
      const next = operation === 'connect'
        ? await client.connect()
        : operation === 'disconnect' && snapshot
          ? await client.disconnect(snapshot.incarnation).then(() => null)
          : await client.snapshot();
      if (generation.current === current) setSnapshot(next);
    } catch {
      // A stale incarnation or busy disconnect leaves the observed connection
      // intact; never claim disconnect success or replay a mutating request.
      if (generation.current === current) setError(true);
    } finally {
      if (generation.current === current) setPending(null);
    }
  };

  return <div className={styles.field}>
    <span className={styles.fieldLabel}>{t('agentSettings.resources.kinds.browser')}</span>
    <div className={styles.fieldControl}>
      <Select value={value} disabled={disabled || Boolean(pending)}
        aria-label={t('agentSettings.resources.selectAria', { resource: t('agentSettings.resources.kinds.browser') })}
        onChange={(next: string) => { if (next === 'managed-browser' || next === 'attached-chrome' && connected) onChange(next); }}>
        <Select.Option value='managed-browser'>{t('agentSettings.resources.browser.managed')}</Select.Option>
        <Select.Option value='attached-chrome' disabled={!connected}>{t('agentSettings.resources.browser.attached')}</Select.Option>
      </Select>
    </div>
    <span className={styles.fieldHint}>{t('agentSettings.resources.browser.scope')}</span>
    <div className={styles.fieldControl}>
      <span className={`${styles.fieldHint} ${error ? styles.fieldError : ''}`} role='status'>
        {pending ? <Spin size={10} /> : error ? t('agentSettings.resources.browser.failed') : t(`agentSettings.resources.browser.states.${snapshot?.state ?? 'disconnected'}`)}
      </span>
      <Button size='mini' disabled={disabled || Boolean(pending)} onClick={() => void act('read')}>{t('agentSettings.resources.browser.refresh')}</Button>
      <Button size='mini' disabled={disabled || Boolean(pending)} onClick={() => void act(snapshot ? 'disconnect' : 'connect')}>
        {t(snapshot ? 'agentSettings.resources.browser.disconnect' : 'agentSettings.resources.browser.connect')}
      </Button>
    </div>
    {!snapshot && <span className={styles.fieldHint}>{t('agentSettings.resources.browser.setup')}</span>}
    {value === 'attached-chrome' && !connected && !pending && <span className={`${styles.fieldHint} ${styles.fieldError}`}>{t('agentSettings.resources.browser.selectedUnavailable')}</span>}
  </div>;
}
