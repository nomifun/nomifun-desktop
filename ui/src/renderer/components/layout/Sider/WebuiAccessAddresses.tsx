import React from 'react';
import { useTranslation } from 'react-i18next';
import { shell } from '@/common/adapter/ipcBridge';
import CopyIconButton from '@/renderer/components/base/CopyIconButton';
import { useWebuiServer } from '@/renderer/hooks/context/WebuiServerContext';
import ConnectionHint from './ConnectionHint';

const WebuiAccessAddresses: React.FC<{ className?: string }> = ({ className = '' }) => {
  const { t } = useTranslation();
  const { accessUrls, lifecycleSupported, running } = useWebuiServer();
  const urls = lifecycleSupported ? running ? accessUrls : [] : [window.location.origin];
  if (urls.length === 0) return null;

  return <section className={`flex flex-col gap-6px ${className}`} aria-label={t('settings.webui.accessUrl')} data-testid='webui-access-addresses'>
    <div className='flex items-center gap-4px'>
      <span className='text-12px font-500 text-t-tertiary'>{t('settings.webui.accessUrl')}</span>
      <ConnectionHint label={t('settings.webui.accessUrl')} content={t('settings.webui.cacheHint')} />
    </div>
    {urls.map((url) => <div key={url} className='flex min-w-0 items-center gap-8px'>
      {lifecycleSupported ? <button
        type='button'
        className='min-w-0 flex-1 cursor-pointer break-all border-none bg-transparent p-0 text-left font-mono text-12px leading-18px text-primary-6 hover:underline'
        onClick={() => void shell.openExternal.invoke(url).catch(console.error)}
      >
        {url}
      </button> : <code className='min-w-0 flex-1 break-all font-mono text-12px leading-18px text-t-primary'>{url}</code>}
      <CopyIconButton text={url} size={14} className='size-22px shrink-0' tooltip={`${t('common.copy')} ${url}`} />
    </div>)}
  </section>;
};

export default WebuiAccessAddresses;
