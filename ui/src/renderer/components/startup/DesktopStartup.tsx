import type { DesktopStartupIO } from '@/common/adapter/tauriStartup';
import { Button, Progress } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import AppLoader from '../layout/AppLoader';
import { useDesktopStartup } from './useDesktopStartup';

const bytes = (value: number) => `${(Math.max(0, value) / 1_000_000).toFixed(1)} MB`;
const failureKeys: Record<string, string> = {
  COMPONENT_DOWNLOAD_FAILED: 'common.startup.errors.download',
  COMPONENT_CACHE_FAILED: 'common.startup.errors.cache',
  COMPONENT_ASSET_INVALID: 'common.startup.errors.asset',
  COMPONENT_VERIFICATION_FAILED: 'common.startup.errors.verification',
  COMPONENT_CHANGED: 'common.startup.errors.changed',
  COMPONENT_SIZE_LIMIT: 'common.startup.errors.size',
  COMPONENT_UNAVAILABLE: 'common.startup.errors.unavailable',
  COMPONENT_CLEANUP_UNPROVEN: 'common.startup.errors.cleanup',
};

export default function DesktopStartup({ io }: { io: DesktopStartupIO }) {
  const { t } = useTranslation();
  const { snapshot, connectionError, retrying, retry, quit } = useDesktopStartup(io);
  const stage = snapshot?.stage ?? 'preparing';
  const progress = snapshot?.preparation;
  const failed = stage === 'failed';
  const phase = progress?.phase ?? 'checking';
  const label = stage === 'quitting' ? t('common.startup.quitting')
    : stage === 'starting_backend' || stage === 'ready' ? t('common.startup.starting')
    : failed ? t('common.startup.failed')
    : phase === 'downloading' ? t('common.startup.downloading')
    : phase === 'extracting' ? t('common.startup.extracting')
    : phase === 'verifying' ? t('common.startup.verifying')
    : t('common.startup.preparing');
  const cleanupUnproven = progress?.error?.code === 'COMPONENT_CLEANUP_UNPROVEN';
  const errorLabel = t(failureKeys[progress?.error?.code ?? ''] ?? 'common.startup.failedHint');
  const downloading = stage === 'preparing' && phase === 'downloading';
  const knownTotal = typeof progress?.total_bytes === 'number' && progress.total_bytes > 0;
  const percent = downloading && knownTotal
    ? Math.min(100, Math.max(0, (progress?.received_bytes ?? 0) / progress!.total_bytes! * 100))
    : undefined;

  return (
    <AppLoader label={label} loading={!failed}>
      <strong className='text-24px text-t-primary'>NomiFun</strong>
      {stage === 'preparing' && <p className='m-0 text-13px text-t-secondary'>{t('common.startup.firstUseHint')}</p>}
      {downloading && (
        <div className='w-360px flex flex-col gap-8px'>
          {percent !== undefined && <Progress percent={percent} showText={false} />}
          <span className='text-12px text-t-secondary'>
            {knownTotal
              ? t('common.startup.downloadBytes', { received: bytes(progress?.received_bytes ?? 0), total: bytes(progress!.total_bytes!) })
              : t('common.startup.receivedBytes', { received: bytes(progress?.received_bytes ?? 0) })}
          </span>
        </div>
      )}
      {(failed || cleanupUnproven) && <p role='alert' className='m-0 text-13px text-danger max-w-480px text-center'>{errorLabel}</p>}
      {connectionError && <p role='alert' className='m-0 text-13px text-danger'>{t('common.startup.connectionFailed')}</p>}
      <div className='flex gap-12px'>
        {failed && progress?.error?.retryable && (
          <Button type='primary' loading={retrying} disabled={retrying} onClick={() => void retry()}>{t('common.retry')}</Button>
        )}
        {stage !== 'ready' && <Button disabled={stage === 'quitting'} onClick={() => void quit()}>{t('common.startup.quit')}</Button>}
      </div>
    </AppLoader>
  );
}
