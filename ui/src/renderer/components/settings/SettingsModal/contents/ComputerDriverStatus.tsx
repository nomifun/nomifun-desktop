import { computer } from '@/common/adapter/ipcBridge';
import { VisualPanel, VisualRow, VisualStatus } from '@/renderer/pages/settings/components/CodeVisualPrimitives';
import { Alert, Button } from '@arco-design/web-react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import useSWR from 'swr';

/** A read-only process snapshot; viewing settings never starts a desktop worker. */
const ComputerDriverStatus: React.FC = () => {
  const { t } = useTranslation();
  const { data, error, isLoading, mutate } = useSWR('computer.status', () => computer.status.invoke(), {
    revalidateOnFocus: true,
  });
  const unknown = t('settings.capabilityPermissions.states.unknown');
  const available = (value: boolean | null) => value === null ? unknown : t(value
    ? 'settings.capabilityPermissions.computer.driver.available'
    : 'settings.capabilityPermissions.computer.driver.unavailable');
  return <VisualPanel title={t('settings.capabilityPermissions.computer.driver.title')}
    description={t('settings.capabilityPermissions.computer.driver.description')}>
    <VisualRow label={t('settings.capabilityPermissions.computer.driver.process')}>
      <VisualStatus tone={data?.running ? 'success' : 'neutral'}>{!data ? unknown : t(data.running
        ? 'settings.capabilityPermissions.computer.driver.running'
        : 'settings.capabilityPermissions.computer.driver.stopped')}</VisualStatus>
      <Button size='small' loading={isLoading} onClick={() => void mutate()}>{t('settings.capabilityPermissions.refresh')}</Button>
    </VisualRow>
    {data && <>
      <VisualRow label={t('settings.capabilityPermissions.computer.driver.version')}>{data.worker_version ?? unknown}</VisualRow>
      <VisualRow label={t('settings.capabilityPermissions.computer.driver.platform')}>{data.platform}</VisualRow>
      <VisualRow label={t('settings.capabilityPermissions.computer.driver.backend')}>{data.backend ?? unknown}</VisualRow>
      <VisualRow label={t('settings.capabilityPermissions.computer.driver.sessionType')}>{data.session_type ?? unknown}</VisualRow>
      {data.platform === 'linux' && <>
        <VisualRow label='Wayland'>{available(data.wayland)}</VisualRow>
        <VisualRow label='AT-SPI'>{available(data.atspi)}</VisualRow>
      </>}
      {data.last_error && <Alert type='warning' showIcon content={data.last_error} />}
    </>}
    {error && <Alert type='error' showIcon content={t('settings.capabilityPermissions.computer.driver.loadFailed')} />}
  </VisualPanel>;
};

export default ComputerDriverStatus;
