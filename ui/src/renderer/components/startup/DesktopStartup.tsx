import type { DesktopStartupIO } from '@/common/adapter/tauriStartup';
import { Button } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import AppLoader from '../layout/AppLoader';
import { useDesktopStartup } from './useDesktopStartup';

export default function DesktopStartup({ io }: { io: DesktopStartupIO }) {
  const { t } = useTranslation();
  const { snapshot, connectionError, quit } = useDesktopStartup(io);
  const stage = snapshot?.stage ?? 'starting_backend';
  const label = stage === 'quitting' ? t('common.startup.quitting')
    : t('common.startup.starting');

  return (
    <AppLoader label={label}>
      <strong className='text-24px text-t-primary'>NomiFun</strong>
      {connectionError && <p role='alert' className='m-0 text-13px text-danger'>{t('common.startup.connectionFailed')}</p>}
      <div className='flex gap-12px'>
        {stage !== 'ready' && <Button disabled={stage === 'quitting'} onClick={() => void quit()}>{t('common.startup.quit')}</Button>}
      </div>
    </AppLoader>
  );
}
