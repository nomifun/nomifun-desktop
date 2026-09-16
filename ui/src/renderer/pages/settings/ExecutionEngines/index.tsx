import { ipcBridge } from '@/common';
import type { RuntimeEngineDescriptor } from '@/common/types/agentPlatform';
import { Alert, Button, Spin, Tag } from '@arco-design/web-react';
import { Refresh } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { Link } from 'react-router-dom';
import useSWR from 'swr';
import SettingsPageWrapper from '../components/SettingsPageWrapper';

const BUILTIN_FAMILIES = ['nomifun.nomi', 'nomifun.coding'];

export default function ExecutionEngineSettings() {
  const { t } = useTranslation();
  const { data, error, isLoading, isValidating, mutate } = useSWR(
    'runtime-engines',
    () => ipcBridge.agentPlatform.runtimeEngines.list.invoke()
  );
  const families = [...new Set([...BUILTIN_FAMILIES, ...(data ?? []).map(engine => engine.family_id)])];
  const name = (family: string, builds: RuntimeEngineDescriptor[]) => {
    if (family === 'nomifun.nomi') return 'Nomi Runtime';
    if (family === 'nomifun.coding') return 'Coding Runtime';
    return builds[0]?.display_name ?? family;
  };

  return (
    <SettingsPageWrapper contentClassName='max-w-1200px'>
      <header className='mb-18px flex items-start gap-16px'>
        <div className='min-w-0 flex-1'>
          <h1 className='m-0 text-20px font-600 leading-28px text-t-primary'>{t('settings.executionEngines.title')}</h1>
          <p className='m-0 mt-4px text-12px leading-18px text-t-secondary'>{t('settings.executionEngines.subtitle')}</p>
        </div>
        <Button icon={<Refresh />} loading={isValidating} onClick={() => { void mutate().catch(() => undefined); }}>
          {t('settings.executionEngines.refresh')}
        </Button>
      </header>
      <div className='flex flex-col gap-16px' aria-busy={isLoading}>
        <Alert type='info' content={(
          <div className='flex flex-col gap-4px'>
            <span>{t('settings.executionEngines.scope')}</span>
            <Link to='/settings/javascript-runtime'>{t('settings.executionEngines.javascriptLink')}</Link>
          </div>
        )} />
        {error && <Alert type='error' content={t('settings.executionEngines.loadError')} />}
        {isLoading && !data && <div className='py-24px flex items-center justify-center gap-8px'><Spin /><span>{t('settings.executionEngines.loading')}</span></div>}
        {data && families.map(family => {
          const builds = data.filter(engine => engine.family_id === family);
          const title = name(family, builds);
          return (
            <section key={family} aria-label={title} className='min-w-0 border border-solid border-[var(--color-border-2)] rd-12px p-20px bg-[var(--color-bg-2)]'>
              <div className='flex flex-wrap items-center gap-8px'>
                <h2 className='m-0 text-16px font-600 text-t-primary'>{title}</h2>
                <Tag color={error ? 'orange' : builds.length ? 'green' : 'gray'}>
                  {error ? t('settings.executionEngines.stale') : builds.length ? t('settings.executionEngines.available') : t('settings.executionEngines.unavailable')}
                </Tag>
              </div>
              <p className='mt-8px mb-0 text-13px leading-20px text-t-secondary'>
                {family === 'nomifun.nomi' ? t('settings.executionEngines.nomiDescription') : family === 'nomifun.coding' ? t('settings.executionEngines.codingDescription') : t('settings.executionEngines.customDescription')}
              </p>
              <div className='mt-4px text-12px font-mono text-t-tertiary break-all'>{family}</div>
              {builds.length === 0 && <p className='mb-0 text-12px leading-18px text-t-secondary'>{t('settings.executionEngines.unavailableHint')}</p>}
              {builds.map(engine => (
                <div key={`${engine.build_id}:${engine.build_digest}`} className='mt-16px pt-16px border-t border-t-solid border-t-[var(--color-border-2)]'>
                  <dl className='m-0 grid grid-cols-[auto_1fr] gap-x-20px gap-y-8px text-13px leading-20px'>
                    <dt className='text-t-secondary'>{t('settings.executionEngines.build')}</dt>
                    <dd className='m-0 min-w-0 break-all text-t-primary'>{engine.build_id}</dd>
                    <dt className='text-t-secondary'>{t('settings.executionEngines.profiles')}</dt>
                    <dd className='m-0 min-w-0 flex flex-wrap gap-6px'>{engine.supported_profiles.map(profile => <Tag key={profile}>{profile}</Tag>)}</dd>
                  </dl>
                  <details className='mt-12px text-12px text-t-secondary'>
                    <summary className='cursor-pointer'>{t('settings.executionEngines.details')}</summary>
                    <dl className='m-0 mt-8px flex flex-col gap-4px'>
                      <dt>{t('settings.executionEngines.digest')}</dt>
                      <dd className='m-0 font-mono break-all select-text'>{engine.build_digest}</dd>
                      <dt>{t('settings.executionEngines.contractVersion')}</dt>
                      <dd className='m-0'>{engine.host_contract_version}</dd>
                    </dl>
                  </details>
                </div>
              ))}
            </section>
          );
        })}
        <section className='p-20px rd-12px bg-fill-1'>
          <h2 className='m-0 text-14px font-600 text-t-primary'>{t('settings.executionEngines.configureTitle')}</h2>
          <p className='mt-8px mb-12px text-13px leading-20px text-t-secondary'>{t('settings.executionEngines.configureHint')}</p>
          <Link to='/agent'>{t('settings.executionEngines.configureLink')}</Link>
        </section>
      </div>
    </SettingsPageWrapper>
  );
}
