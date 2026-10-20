import React, { createContext, useContext, useEffect, useMemo, useRef, useState } from 'react';
import { addEventListener } from '@/renderer/utils/emitter';
import { useTranslation } from 'react-i18next';
import { toolDetailIdentityKey, toolDetailSnapshotKey, type ToolDetailIdentity, type ToolDetailPage, type ToolDetailSection } from '@/common/chat/toolDetails';
import { ToolDetailsCache } from '../toolDetailsCache';
import { ipcBridge } from '@/common';

const DetailCacheContext = createContext<{ cache: ToolDetailsCache; epoch: number } | null>(null);

/** Viewer-owned payloads expire on Session deletion, navigation and backend reconnect. */
export function ToolDetailsProvider({ scope, children }: { scope?: string; children: React.ReactNode }) {
  const [revision, setRevision] = useState(0);
  const cache = useMemo(() => new ToolDetailsCache(), [scope, revision]);
  useEffect(() => () => cache.clear(), [cache]);
  useEffect(() => {
    const invalidate = () => { cache.clear(); setRevision(value => value + 1); };
    const deleted = addEventListener('conversation.deleted', id => { if (id === scope) invalidate(); });
    const reconnected = ipcBridge.conversation.reconnected.on(invalidate);
    return () => { deleted(); reconnected(); };
  }, [cache, scope]);
  const value = useMemo(() => ({ cache, epoch: revision }), [cache, revision]);
  return <DetailCacheContext.Provider value={value}>{children}</DetailCacheContext.Provider>;
}

function ToolDetailSectionView({ source, section, cache }: {
  source: ToolDetailIdentity; section: ToolDetailSection; cache: ToolDetailsCache;
}) {
  const { t } = useTranslation();
  const [pages, setPages] = useState<ToolDetailPage[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(false);
  const [retry, setRetry] = useState(0);
  const snapshot = toolDetailSnapshotKey(source);
  const generation = useRef(0);
  useEffect(() => {
    generation.current += 1;
    let active = true;
    setPages([]); setLoading(true); setError(false);
    void cache.get({ ...source, section }).then(page => {
      if (active) setPages([page]);
    }).catch(() => { if (active) setError(true); }).finally(() => { if (active) setLoading(false); });
    return () => { active = false; generation.current += 1; };
  }, [cache, snapshot, section, retry]);
  const last = pages.at(-1);
  const content = pages.map(page => page.content).join('');
  let displayed = content;
  if (last?.format === 'json' && last.eof) {
    try { displayed = JSON.stringify(JSON.parse(content), null, 2); } catch { /* A byte page may start inside a JSON value. */ }
  }
  const loadMore = async () => {
    if (!last?.next_cursor || loading) return;
    const requestGeneration = generation.current;
    setLoading(true); setError(false);
    try {
      const page = await cache.get({ ...source, through_seq: last.through_seq,
        ...(last.result_seq !== undefined ? { expected_result_seq: last.result_seq } : {}), section, cursor: last.next_cursor });
      if (page.call_event_id !== last.call_event_id || page.result_seq !== last.result_seq
        || page.offset !== last.offset + new TextEncoder().encode(last.content).length) throw new Error('Tool detail page did not continue its snapshot');
      if (generation.current === requestGeneration) setPages(previous => [...previous, page].slice(-16));
    } catch { if (generation.current === requestGeneration) setError(true); }
    finally { if (generation.current === requestGeneration) setLoading(false); }
  };
  return <section className='turn-process-trace-detail__section' data-tool-detail-section={section}>
    <div className='turn-process-trace-detail__label'>{t(section === 'arguments' && last?.format === 'text' ? 'messages.toolDetails.command' : `messages.toolDetails.${section}`)}</div>
    {section === 'arguments' && last?.working_directory && <p className='turn-process-trace-detail__notice'>{t('messages.toolDetails.directory', { path: last.working_directory, interpolation: { escapeValue: false } })}</p>}
    {pages[0]?.offset > 0 && <p className='turn-process-trace-detail__notice'>{t('messages.toolDetails.previousPages')}</p>}
    {displayed && <pre className='turn-process-trace-detail__content'>{displayed}</pre>}
    {last?.availability === 'available' && !content && !loading && <p className='turn-process-trace-detail__notice'>{t('messages.toolDetails.empty')}</p>}
    {last?.availability === 'pending' && <p role='status' className='turn-process-trace-detail__notice'>{t('messages.toolDetails.pending')}</p>}
    {last?.availability === 'unconfirmed' && <p role='status' className='turn-process-trace-detail__notice'>{t('messages.toolResultUnconfirmed')}</p>}
    {section === 'output' && last?.retention && <div className='turn-process-trace-detail__notice'>
      <p>{t('messages.toolDetails.retained', { bytes: last.retention.retained_bytes })}</p>
      {last.retention.dropped_bytes > 0 && <p role='note'>{t('messages.toolDetails.dropped', { bytes: last.retention.dropped_bytes })}</p>}
      {last.retention.decode_errors > 0 && <p role='note'>{t('messages.toolDetails.decodeErrors', { count: last.retention.decode_errors })}</p>}
    </div>}
    {last && last.non_text_parts > 0 && <p className='turn-process-trace-detail__notice'>{t('messages.toolDetails.nonText', { count: last.non_text_parts })}</p>}
    {loading && <p role='status' className='turn-process-trace-detail__notice'>{t('messages.toolDetails.loading')}</p>}
    {error && <p role='alert' className='turn-process-trace-detail__notice'>{t('messages.toolDetails.failed')} <button type='button' onClick={() => last?.next_cursor ? void loadMore() : setRetry(value => value + 1)}>{t('messages.toolDetails.retry')}</button></p>}
    {last?.next_cursor && <button type='button' disabled={loading} className='turn-process-trace-detail__more' onClick={() => void loadMore()}>{t('messages.toolDetails.more')}</button>}
  </section>;
}

/** Mounted only while expanded; closing removes its request observers. */
export default function ToolDetails({ source }: { source: ToolDetailIdentity }) {
  const shared = useContext(DetailCacheContext);
  const local = useMemo(() => new ToolDetailsCache(), [source.conversation_id]);
  const cache = shared?.cache ?? local;
  const selectionKey = source.expected_result_seq !== undefined ? toolDetailIdentityKey(source) : toolDetailSnapshotKey(source);
  const [selection, setSelection] = useState(() => ({ key: selectionKey, cache, source: { ...source } }));
  const sameSelection = selection.key === selectionKey && selection.cache === cache;
  // A proven result pins the first read snapshot for this opened inspector.
  // A later Session head is unrelated activity, not a new result or page boundary.
  if (!sameSelection) setSelection({ key: selectionKey, cache, source: { ...source } });
  const readSource = sameSelection ? selection.source : source;
  useEffect(() => () => local.clear(), [local]);
  return <div className='turn-tool-details' key={`${toolDetailSnapshotKey(readSource)}:${shared?.epoch ?? 0}`}>
    <ToolDetailSectionView source={readSource} section='arguments' cache={cache} />
    <ToolDetailSectionView source={readSource} section='output' cache={cache} />
  </div>;
}
