import React, { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { openExternalUrl } from '@/renderer/utils/platform';
import type { TurnDisclosureProcessState } from '../turnDisclosureModel';
import { localSearchPayload, localSearchResult } from './localSearchResultModel';

type Props = { input?: string; output?: string; state: TurnDisclosureProcessState };

export default function LocalWebSearchResult({ input, output, state }: Props) {
  const { t } = useTranslation();
  const [openFailed, setOpenFailed] = useState(false);
  useEffect(() => { setOpenFailed(false); }, [input, output, state]);
  const result = useMemo(() => state === 'completed' ? localSearchResult(output) : null, [output, state]);
  const inputQuery = localSearchPayload(input)?.query;
  const query = result?.query ?? (typeof inputQuery === 'string' ? inputQuery.slice(0, 4096) : '');
  const code = localSearchPayload(output)?.code;
  const status = state === 'running' ? t('browserWorkspace.search.searching')
    : state === 'canceled' ? t('browserWorkspace.search.canceled')
    : state === 'failed' ? code === 'NOMI_LOCAL_WEBSEARCH_CHALLENGE' ? t('browserWorkspace.search.challenge')
      : code === 'NOMI_LOCAL_WEBSEARCH_TIMEOUT' ? t('browserWorkspace.search.timeout')
      : code === 'NOMI_LOCAL_WEBSEARCH_BUSY' ? t('browserWorkspace.search.busy') : t('browserWorkspace.search.failed')
    : !result ? t('browserWorkspace.search.invalid')
    : result.sources.length ? t('browserWorkspace.search.count', { count: result.sources.length }) : t('browserWorkspace.search.empty');

  const openSource = (event: React.MouseEvent<HTMLAnchorElement>, url: string) => {
    event.preventDefault();
    if (event.type === 'auxclick' && event.button !== 1) return;
    setOpenFailed(false);
    void openExternalUrl(url).catch(() => setOpenFailed(true));
  };

  return (
    <section className='min-w-0 text-13px' aria-label={t('browserWorkspace.search.title')} aria-busy={state === 'running'}>
      <div className='flex flex-wrap items-center gap-x-8px gap-y-2px'>
        <span className='font-medium text-t-primary'>{t('browserWorkspace.search.title')}</span>
        <span className='text-t-secondary' role='status'>{status}</span>
      </div>
      {query && <div className='text-t-secondary break-words mt-2px'>{query}</div>}
      {result && result.sources.length > 0 && (
        <ol className='my-8px pl-20px flex flex-col gap-8px'>
          {result.sources.map(source => (
            <li key={source.citationId} className='min-w-0' data-citation-id={source.citationId}>
              <a href={source.url} target='_blank' rel='noopener noreferrer' className='text-brand break-words hover:underline focus-visible:underline'
                title={t('browserWorkspace.search.openSource', { url: source.url })}
                onClick={event => openSource(event, source.url)} onAuxClick={event => openSource(event, source.url)}>{source.title}</a>
              <div className='text-12px text-t-secondary break-all'>{source.host}</div>
              {source.snippet && <div className='text-t-secondary break-words line-clamp-2'>{source.snippet}</div>}
            </li>
          ))}
        </ol>
      )}
      {openFailed && <div role='alert' className='text-t-secondary'>{t('browserWorkspace.search.openFailed')}</div>}
      <div className='text-12px text-t-secondary mt-4px'>{t('browserWorkspace.search.isolated')}</div>
    </section>
  );
}
