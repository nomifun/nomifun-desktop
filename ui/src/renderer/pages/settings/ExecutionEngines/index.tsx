import { ipcBridge } from '@/common';
import { Alert, Button, Spin, Tag } from '@arco-design/web-react';
import { CheckOne, Refresh, Shield } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { Link } from 'react-router-dom';
import useSWR from 'swr';
import SettingsPageWrapper from '../components/SettingsPageWrapper';

export default function ExecutionEngineSettings() {
  const { t } = useTranslation();
  const { data, error, isLoading, isValidating, mutate } = useSWR(
    'agent-runtime',
    () => ipcBridge.agentPlatform.agentRuntime.get.invoke()
  );
  const build = data;
  const stale = Boolean(error && data);

  return (
    <SettingsPageWrapper contentClassName='max-w-920px'>
      <header className='mb-18px flex items-start gap-16px'>
        <div className='min-w-0 flex-1'>
          <h1 className='m-0 text-20px font-600 leading-28px text-t-primary'>
            {t('settings.executionEngines.title')}
          </h1>
          <p className='m-0 mt-4px text-12px leading-18px text-t-secondary'>
            {t('settings.executionEngines.subtitle')}
          </p>
        </div>
        <Button
          icon={<Refresh />}
          loading={isValidating}
          aria-label={t('settings.executionEngines.refresh')}
          onClick={() => { void mutate().catch(() => undefined); }}
        >
          {t('settings.executionEngines.refresh')}
        </Button>
      </header>

      <div className='flex flex-col gap-14px' aria-busy={isLoading || isValidating}>
        <Alert
          type='info'
          content={(
            <div className='flex flex-col gap-4px'>
              <span>{t('settings.executionEngines.scope')}</span>
              <Link to='/settings/javascript-runtime'>{t('settings.executionEngines.javascriptLink')}</Link>
            </div>
          )}
        />

        {error && <Alert type='error' content={t('settings.executionEngines.loadError')} />}
        {isLoading && !data && (
          <div className='py-32px flex items-center justify-center gap-8px' role='status'>
            <Spin />
            <span>{t('settings.executionEngines.loading')}</span>
          </div>
        )}

        {data && (
          <section
            aria-label='Nomi Runtime'
            className='min-w-0 border border-solid border-[var(--color-border-2)] rd-12px bg-[var(--color-bg-2)] overflow-hidden'
          >
            <div className='p-20px flex items-start gap-14px'>
              <span className='size-38px rd-10px flex items-center justify-center bg-primary-1 text-primary-6 shrink-0'>
                <Shield theme='outline' size={20} />
              </span>
              <div className='min-w-0 flex-1'>
                <div className='flex flex-wrap items-center gap-8px'>
                  <h2 className='m-0 text-16px font-600 text-t-primary'>Nomi Runtime</h2>
                  <Tag color={stale ? 'orange' : build ? 'green' : 'gray'}>
                    {stale
                      ? t('settings.executionEngines.stale')
                      : build
                        ? t('settings.executionEngines.healthy')
                        : t('settings.executionEngines.unavailable')}
                  </Tag>
                  <Tag>{t('settings.executionEngines.singleRuntime')}</Tag>
                </div>
                <p className='mt-8px mb-0 text-13px leading-20px text-t-secondary'>
                  {t('settings.executionEngines.nomiDescription')}
                </p>
              </div>
            </div>

            {build ? (
              <div className='px-20px py-16px border-t border-t-solid border-t-[var(--color-border-2)] bg-fill-1'>
                <dl className='m-0 grid grid-cols-[auto_minmax(0,1fr)] gap-x-20px gap-y-9px text-13px leading-20px'>
                  <dt className='text-t-secondary'>{t('settings.executionEngines.build')}</dt>
                  <dd className='m-0 min-w-0 break-all text-t-primary'>{build.build_id}</dd>
                  <dt className='text-t-secondary'>{t('settings.executionEngines.health')}</dt>
                  <dd className='m-0 min-w-0 flex items-center gap-6px text-t-primary'>
                    <CheckOne theme='filled' size={14} className='text-green-6' />
                    {t('settings.executionEngines.healthReady')}
                  </dd>
                  <dt className='text-t-secondary'>{t('settings.executionEngines.recovery')}</dt>
                  <dd className='m-0 min-w-0 text-t-primary'>{t('settings.executionEngines.recoveryReady')}</dd>
                </dl>
                <details className='mt-14px text-12px text-t-secondary'>
                  <summary className='cursor-pointer'>{t('settings.executionEngines.details')}</summary>
                  <dl className='m-0 mt-9px grid grid-cols-[auto_minmax(0,1fr)] gap-x-20px gap-y-7px'>
                    <dt>{t('settings.executionEngines.digest')}</dt>
                    <dd className='m-0 font-mono break-all select-text'>{build.build_digest}</dd>
                    <dt>{t('settings.executionEngines.contractVersion')}</dt>
                    <dd className='m-0'>{build.host_contract_version}</dd>
                    <dt>{t('settings.executionEngines.runtimeModes')}</dt>
                    <dd className='m-0'>{t('settings.executionEngines.adaptiveMode')}</dd>
                  </dl>
                </details>
              </div>
            ) : (
              <div className='px-20px py-18px border-t border-t-solid border-t-[var(--color-border-2)] bg-fill-1'>
                <p className='m-0 text-13px leading-20px text-t-secondary'>
                  {t('settings.executionEngines.unavailableHint')}
                </p>
              </div>
            )}
          </section>
        )}

        <section className='p-20px rd-12px bg-fill-1'>
          <h2 className='m-0 text-14px font-600 text-t-primary'>
            {t('settings.executionEngines.configureTitle')}
          </h2>
          <p className='mt-8px mb-12px text-13px leading-20px text-t-secondary'>
            {t('settings.executionEngines.configureHint')}
          </p>
          <Link to='/agent'>{t('settings.executionEngines.configureLink')}</Link>
        </section>
      </div>
    </SettingsPageWrapper>
  );
}
