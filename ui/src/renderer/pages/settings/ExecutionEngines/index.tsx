import { ipcBridge } from '@/common';
import { Alert, Button, Spin } from '@arco-design/web-react';
import { CheckOne, Cpu, Refresh, Right, Shield } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { Link } from 'react-router-dom';
import useSWR from 'swr';
import SettingsPageWrapper from '../components/SettingsPageWrapper';
import { VisualListRow, VisualPanel, VisualStatus } from '../components/CodeVisualPrimitives';
import PageHeader from '@/renderer/components/layout/PageHeader';

export default function ExecutionEngineSettings() {
  const { t } = useTranslation();
  const { data, error, isLoading, isValidating, mutate } = useSWR('agent-runtime', () => ipcBridge.agentPlatform.runtime.get.invoke());
  const build = data;
  const stale = Boolean(error && data);

  return <SettingsPageWrapper>
    <PageHeader title={t('settings.executionEngines.title')}
      actions={<Button size='small' icon={<Refresh theme='outline' size={14} />} loading={isValidating}
        aria-label={t('settings.executionEngines.refresh')} onClick={() => { void mutate().catch(() => undefined); }}>
        {t('settings.executionEngines.refresh')}
      </Button>} />
    <div className='cv-stack' aria-busy={isLoading || isValidating}>
      {error && <Alert type='error' content={t('settings.executionEngines.loadError')} />}
      {isLoading && !data && <div className='py-32px flex items-center justify-center gap-8px' role='status'>
        <Spin /><span>{t('settings.executionEngines.loading')}</span>
      </div>}
      {data && <VisualPanel label='Nomi Runtime'>
        {build ? <>
          <VisualListRow icon={<Cpu theme='outline' size={17} />} title={t('settings.executionEngines.build')}
            description={<span className='cv-directory-path'>{build.build_id}</span>}
            action={<VisualStatus tone={stale ? 'warning' : 'success'}>{stale ? t('settings.executionEngines.stale') : t('settings.executionEngines.healthy')}</VisualStatus>} />
          <VisualListRow icon={<CheckOne theme='outline' size={17} />} title={t('settings.executionEngines.health')}
            description={t('settings.executionEngines.healthReady')} />
          <VisualListRow icon={<Shield theme='outline' size={17} />} title={t('settings.executionEngines.recovery')}
            description={t('settings.executionEngines.recoveryReady')} />
          <details className='cv-details'>
            <summary>{t('settings.executionEngines.details')}</summary>
            <dl>
              <dt>{t('settings.executionEngines.digest')}</dt><dd className='cv-directory-path'>{build.build_digest}</dd>
              <dt>{t('settings.executionEngines.contractVersion')}</dt><dd>{build.host_contract_version}</dd>
              <dt>{t('settings.executionEngines.runtimeModes')}</dt><dd>{t('settings.executionEngines.adaptiveMode')}</dd>
            </dl>
          </details>
        </> : <p className='cv-note'>{t('settings.executionEngines.unavailableHint')}</p>}
      </VisualPanel>}
      <VisualPanel title={t('settings.executionEngines.configureTitle')} description={t('settings.executionEngines.configureHint')}>
        <Link to='/agent' className='cv-link-row'>{t('settings.executionEngines.configureLink')}<Right theme='outline' size={15} /></Link>
      </VisualPanel>
      <p className='cv-note'>{t('settings.executionEngines.scope')}</p>
    </div>
  </SettingsPageWrapper>;
}
