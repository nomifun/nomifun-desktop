import { useCallback, useEffect, useRef, useState } from 'react';
import { Alert, Button, Spin } from '@arco-design/web-react';
import { Code, Download, FileCode, PlayOne, Right, Save, Check } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { pluginPlatform } from '@/common/adapter/pluginPlatformBridge';
import type { PluginDraftDetail, PluginDraftSummary, PluginSurfaceDescriptor } from '@/common/types/pluginPlatform';
import { isDesktopShell } from '@/renderer/utils/platform';
import PluginSurfacePanel from './PluginSurfacePanel';
import styles from './PluginAuthoringArtifacts.module.css';

type Detail = Awaited<ReturnType<typeof pluginPlatform.authoring.details.invoke>>;
type AuthoringOperation = (operation: () => Promise<void>) => Promise<void>;
const errorMessage = (caught: unknown) => caught instanceof Error ? caught.message : String(caught);
const encodeText = (text: string) => {
  const bytes = new TextEncoder().encode(text);
  let binary = '';
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
};

export default function PluginAuthoringArtifacts({ conversationId, operationDisabled = false, visible = true, onAuthoringOperation }: {
  conversationId: string;
  operationDisabled?: boolean;
  visible?: boolean;
  onAuthoringOperation: AuthoringOperation;
}) {
  const { t } = useTranslation();
  const readOnly = !isDesktopShell();
  const [drafts, setDrafts] = useState<PluginDraftSummary[]>([]);
  const [selectedId, setSelectedId] = useState('');
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const generation = useRef(0);
  const refresh = useCallback(async () => {
    const requestGeneration = ++generation.current;
    try {
      const result = await pluginPlatform.drafts.list.invoke();
      if (requestGeneration !== generation.current) return;
      const visible = result.drafts.filter(draft => draft.source_conversation_id === conversationId);
      setDrafts(visible); setError('');
      setSelectedId(current => visible.some(draft => draft.draft_id === current) ? current : visible[0]?.draft_id ?? '');
    } catch (caught) {
      if (requestGeneration === generation.current) setError(errorMessage(caught));
    } finally {
      if (requestGeneration === generation.current) setLoading(false);
    }
  }, [conversationId]);
  useEffect(() => {
    setLoading(true); setDrafts([]); setSelectedId(''); setError('');
    if (!isDesktopShell()) { setLoading(false); return; }
    void refresh();
    const off = pluginPlatform.authoring.changed.on(event => {
      if (event.conversation_id === conversationId) void refresh();
    });
    return () => { generation.current += 1; off(); };
  }, [conversationId, refresh]);
  const selected = drafts.find(draft => draft.draft_id === selectedId);
  return <div className={styles.workspace}>
    {error && <div className={styles.error}><Alert type='error' content={error} />
      <Button onClick={() => void refresh()}>{t('pluginPlatform.authoring.retryLoad')}</Button></div>}
    {loading ? <div className={styles.empty}><Spin /><p>{t('pluginPlatform.authoring.loadingArtifacts')}</p></div>
      : selected ? <>
        {drafts.length > 1 && <div className={styles.artifactPicker} role='tablist' aria-label={t('pluginPlatform.authoring.artifacts')}>
          {drafts.map(draft => <button key={draft.draft_id} type='button' role='tab' aria-selected={selectedId === draft.draft_id}
            onClick={() => setSelectedId(draft.draft_id)}>{draft.display_name}</button>)}
        </div>}
        {drafts.map(draft => <div className={styles.artifactSlot} key={draft.draft_id} hidden={draft.draft_id !== selectedId}>
          <Artifact summary={draft} active={visible && draft.draft_id === selectedId} refresh={refresh} readOnly={readOnly} operationDisabled={operationDisabled} onAuthoringOperation={onAuthoringOperation} />
        </div>)}
      </> : <div className={styles.empty}>
        <span className={styles.emptyMark}><Code size={25} /></span>
        <h3>{t('pluginPlatform.authoring.artifactEmptyTitle')}</h3><p>{t('pluginPlatform.authoring.artifactEmptyBody')}</p>
      </div>}
  </div>;
}

function Artifact({ summary, active, refresh, readOnly, operationDisabled, onAuthoringOperation }: {
  summary: PluginDraftSummary;
  active: boolean;
  refresh: () => Promise<void>;
  readOnly: boolean;
  operationDisabled: boolean;
  onAuthoringOperation: AuthoringOperation;
}) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [detail, setDetail] = useState<Detail | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [loadError, setLoadError] = useState('');
  const [view, setView] = useState<'preview' | 'files'>('preview');
  const [previewAvailable, setPreviewAvailable] = useState(true);
  const [selectedPath, setSelectedPath] = useState('');
  const [edits, setEdits] = useState<Record<string, string>>({});
  const [installedSurface, setInstalledSurface] = useState<PluginSurfaceDescriptor | null>(null);
  const installedRef = useRef<PluginSurfaceDescriptor | null>(null);
  const previewRef = useRef<PluginSurfaceDescriptor | null>(null);
  const closedPreviews = useRef(new Set<string>());
  const openRequest = useRef(0);
  const mounted = useRef(false);
  const activeRef = useRef(active);
  activeRef.current = active;
  const [previewClosed, setPreviewClosed] = useState(false);
  const [reloadKey, setReloadKey] = useState(0);
  const loadGeneration = useRef(0);
  const closeSurface = useCallback(async (descriptor: PluginSurfaceDescriptor) => {
    if (descriptor.is_preview) closedPreviews.current.add(`${descriptor.surface_session_id}:${descriptor.surface_generation}`);
    await pluginPlatform.surface.close.invoke({
      plugin_id: descriptor.plugin_id, draft_id: descriptor.draft_id, is_preview: descriptor.is_preview,
      request: { surface_session_id: descriptor.surface_session_id, surface_generation: descriptor.surface_generation },
    }).catch(() => undefined);
  }, []);
  const load = useCallback(async () => {
    const generation = ++loadGeneration.current;
    const next = await pluginPlatform.authoring.details.invoke({ draft_id: summary.draft_id });
    if (generation === loadGeneration.current) {
      setDetail(next); setLoadError('');
      setSelectedPath(current => next.draft.files.some(file => file.path === current) ? current : next.draft.files[0]?.path ?? '');
    }
    return next;
  }, [summary.draft_id]);
  useEffect(() => {
    let active = true;
    void load().catch(caught => { if (active) setLoadError(errorMessage(caught)); });
    const off = pluginPlatform.authoring.changed.on(event => {
      if (active && event.draft_id === summary.draft_id) void load().catch(caught => { if (active) setLoadError(errorMessage(caught)); });
    });
    return () => { active = false; loadGeneration.current += 1; off(); };
  }, [summary.draft_id, summary.revision, load]);
  const closeInstalledSurface = useCallback(async () => {
    const descriptor = installedRef.current;
    installedRef.current = null;
    setInstalledSurface(null);
    if (descriptor) await closeSurface(descriptor);
  }, [closeSurface]);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false; openRequest.current += 1;
      if (installedRef.current) void closeSurface(installedRef.current);
      if (previewRef.current) void closeSurface(previewRef.current);
      installedRef.current = null; previewRef.current = null;
    };
  }, [closeSurface]);
  const delivered = Boolean(detail?.draft.summary.delivered_artifact_digest);
  const savedId = delivered ? detail?.draft.summary.plugin_id : undefined;
  const previewSurface = detail?.verification.surface as PluginSurfaceDescriptor | undefined;
  const descriptor = readOnly || !active ? undefined : savedId ? installedSurface : previewClosed ? undefined : previewSurface;
  const file = detail?.draft.files.find(item => item.path === selectedPath);
  const dirty = detail?.draft.files.some(item => edits[item.path] !== undefined && edits[item.path] !== item.text) ?? false;
  useEffect(() => {
    const previous = previewRef.current;
    const next = !readOnly && active && !savedId && previewSurface
      && !closedPreviews.current.has(`${previewSurface.surface_session_id}:${previewSurface.surface_generation}`) ? previewSurface : null;
    const changed = previous?.surface_session_id !== next?.surface_session_id || previous?.surface_generation !== next?.surface_generation;
    if (changed && previous) void closeSurface(previous);
    previewRef.current = next;
    if (next && changed) setPreviewClosed(false);
  }, [previewSurface?.surface_session_id, previewSurface?.surface_generation, readOnly, active, savedId, closeSurface]);
  useEffect(() => {
    if (active) return;
    openRequest.current += 1;
    setBusy(false); setPreviewClosed(true);
    void closeInstalledSurface();
  }, [active, closeInstalledSurface]);
  useEffect(() => { setPreviewAvailable(true); void closeInstalledSurface(); }, [summary.delivered_artifact_digest, closeInstalledSurface]);
  const command = detail?.commands[0];
  const openPreview = async () => {
    if (!detail || busy || readOnly || !active) return;
    const request = ++openRequest.current;
    setBusy(true); setError('');
    try {
      await closeInstalledSurface();
      if (!mounted.current || !activeRef.current || request !== openRequest.current) return;
      if (savedId) {
        const plugin = await pluginPlatform.plugins.get.invoke({ plugin_id: savedId });
        if (!mounted.current || !activeRef.current || request !== openRequest.current) return;
        if (!plugin.summary.has_ui) { setPreviewAvailable(false); return; }
        setPreviewAvailable(true);
        const current = await pluginPlatform.plugins.openSurface.invoke({
          plugin_id: savedId, request: { expected_revision: plugin.summary.revision },
        });
        if (!mounted.current || !activeRef.current || request !== openRequest.current) { await closeSurface(current); return; }
        installedRef.current = current; setInstalledSurface(current);
      } else {
        const existing = detail.draft.summary.plugin_id ? await pluginPlatform.plugins.get.invoke({ plugin_id: detail.draft.summary.plugin_id }) : null;
        if (!mounted.current || !activeRef.current || request !== openRequest.current) return;
        const current = await pluginPlatform.drafts.preview.invoke({
          draft_id: summary.draft_id, request: { expected_revision: detail.draft.summary.revision, config: existing?.config.values ?? {} },
        });
        if (!mounted.current || !activeRef.current || request !== openRequest.current) { await closeSurface(current.descriptor); return; }
        if (previewRef.current && previewRef.current.surface_session_id !== current.descriptor.surface_session_id) await closeSurface(previewRef.current);
        closedPreviews.current.delete(`${current.descriptor.surface_session_id}:${current.descriptor.surface_generation}`);
        previewRef.current = current.descriptor;
        setDetail(previous => previous ? { ...previous, verification: { ...previous.verification, surface: current.descriptor } } : previous);
        setPreviewClosed(false);
      }
      setView('preview');
    } catch (caught) { if (mounted.current && request === openRequest.current) setError(errorMessage(caught)); }
    finally { if (mounted.current && request === openRequest.current) setBusy(false); }
  };
  const save = async () => {
    if (!detail || readOnly || busy || operationDisabled) return;
    setBusy(true); setError('');
    const pending = Object.entries(edits).filter(([path, text]) => detail.draft.files.find(file => file.path === path)?.text !== text);
    try {
      await onAuthoringOperation(async () => {
        let draft: PluginDraftDetail = detail.draft;
        const existing = draft.summary.plugin_id ? await pluginPlatform.plugins.get.invoke({ plugin_id: draft.summary.plugin_id }) : null;
        const config = existing?.config.values ?? {};
        const credentialBindings = Object.fromEntries((existing?.credential_bindings ?? [])
          .filter(binding => binding.credential_id).map(binding => [binding.slot, binding.credential_id!]));
        for (const [path, text] of pending) {
          draft = await pluginPlatform.drafts.replaceFile.invoke({ draft_id: summary.draft_id,
            request: { expected_revision: draft.summary.revision, path, content_base64: encodeText(text) } });
          setDetail(previous => previous ? { ...previous, draft, verification: {}, commands: [] } : previous);
        }
        const saved = await pluginPlatform.drafts.save.invoke({ draft_id: summary.draft_id,
          request: { expected_revision: draft.summary.revision,
            ...(existing ? { expected_plugin_revision: existing.summary.revision } : {}), config, credential_bindings: credentialBindings } });
        setDetail(previous => previous ? { ...previous, draft: { ...previous.draft, summary: saved.draft } } : previous);
        setEdits({}); await closeInstalledSurface(); await load(); await refresh();
      });
    } catch (caught) {
      // An interrupted response can follow a successful save. Reconcile the
      // existing draft before allowing another install of its exact bytes.
      try {
        const current = await load(); await refresh();
        const persisted = Object.entries(edits).every(([path, text]) => current.draft.files.find(file => file.path === path)?.text === text);
        if (persisted && current.draft.summary.delivered_artifact_digest) setEdits({});
        else setError(errorMessage(caught));
      } catch { setError(errorMessage(caught)); }
    }
    finally { setBusy(false); }
  };
  const exportSource = () => {
    if (!detail) return;
    const files = detail.draft.files.map(file => edits[file.path] === undefined ? file : { ...file, text: edits[file.path] });
    const url = URL.createObjectURL(new Blob([JSON.stringify({ files }, null, 2)], { type: 'application/json' }));
    const link = document.createElement('a');
    link.href = url; link.download = `plugin-${summary.draft_id}.json`;
    link.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
  };
  return <section className={styles.artifact}>
    <div className={styles.artifactHeading}>
      <span className={styles.appIcon}><Code size={21} /></span>
      <div className={styles.appCopy}><h2>{summary.display_name}</h2><p>{summary.description || t('pluginPlatform.library.draftDescription')}</p></div>
      {savedId && <Button size='small' type='primary' onClick={() => navigate(`/plugins/run/${encodeURIComponent(savedId)}`)}>{t('pluginPlatform.actions.open')}<Right /></Button>}
    </div>
    <div className={styles.toolbar}>
      <div className={styles.viewTabs} role='tablist' aria-label={t('pluginPlatform.authoring.artifacts')}>
        <button type='button' role='tab' aria-selected={view === 'preview'} onClick={() => setView('preview')}><PlayOne />{t('pluginPlatform.authoring.preview')}</button>
        <button type='button' role='tab' aria-selected={view === 'files'} onClick={() => setView('files')}><FileCode />{t('pluginPlatform.authoring.files')}</button>
      </div>
      <button type='button' className={styles.iconButton} disabled={!detail} onClick={exportSource}
        title={t('pluginPlatform.authoring.exportSource')} aria-label={t('pluginPlatform.authoring.exportSource')}><Download /></button>
    </div>
    {(error || loadError) && <div className={styles.error}><Alert type='error' content={error || loadError} />{loadError && <Button onClick={() => void load().catch(caught => setLoadError(errorMessage(caught)))}>{t('pluginPlatform.authoring.retryLoad')}</Button>}</div>}
    {!detail ? <div className={styles.empty}><Spin /></div> : <>
      <div className={styles.content} hidden={view !== 'preview'}>
        {descriptor?.entrypoint ? <div className={styles.preview}>
          <PluginSurfacePanel key={`${descriptor.surface_session_id}:${reloadKey}`} descriptor={descriptor} title={summary.display_name}
            onReload={() => savedId ? void openPreview() : setReloadKey(previous => previous + 1)}
            onClose={async () => {
              await closeSurface(descriptor);
              openRequest.current += 1;
              setInstalledSurface(null); installedRef.current = null; setPreviewClosed(true);
              previewRef.current = null;
            }} verification={command && !savedId ? { command,
              onComplete: async (observations, failure) => {
                await pluginPlatform.authoring.uiResults.invoke({ draft_id: summary.draft_id,
                  request: { test_token: command.test_token, descriptor: command.descriptor, observations, ...(failure ? { error: failure } : {}) } });
              },
            } : undefined} />
        </div> : <div className={styles.empty}>
          <span className={styles.emptyMark}><PlayOne size={24} /></span><h3>{t('pluginPlatform.authoring.preview')}</h3>
          <p>{t(previewAvailable ? 'pluginPlatform.authoring.artifactEmptyBody' : 'pluginPlatform.authoring.noPreview')}</p>
          {!readOnly && previewAvailable && <Button icon={<PlayOne />} loading={busy} disabled={operationDisabled} onClick={() => void openPreview()}>{t('pluginPlatform.authoring.preview')}</Button>}
        </div>}
      </div>
      <div className={styles.source} hidden={view !== 'files'}>
        <div className={styles.fileSelector}>
          <label htmlFor={`plugin-source-${summary.draft_id}`}><FileCode />{t('pluginPlatform.authoring.fileCount', { count: detail.draft.files.length })}</label>
          <select id={`plugin-source-${summary.draft_id}`} aria-label={t('pluginPlatform.authoring.files')} value={selectedPath} onChange={event => setSelectedPath(event.target.value)}>
            {detail.draft.files.map(file => <option key={file.path} value={file.path}>{file.path}</option>)}
          </select>
        </div>
        {file?.text !== undefined ? <textarea className={styles.editor} aria-label={file.path} value={edits[file.path] ?? file.text}
          readOnly={readOnly || busy || operationDisabled} spellCheck={false} onChange={event => setEdits(current => ({ ...current, [file.path]: event.target.value }))}
          onKeyDown={event => {
            if (event.key === 's' && (event.ctrlKey || event.metaKey)) { event.preventDefault(); void save(); }
          }} /> : <div className={styles.empty}><FileCode size={28} /><p>{t('pluginPlatform.authoring.readOnlyFile')}</p></div>}
      </div>
      <footer className={styles.footer}>
        <span className={styles.saveStatus} data-dirty={dirty || undefined}>{dirty ? t('pluginPlatform.authoring.sourceChanged')
          : savedId ? <><Check />{t('pluginPlatform.authoring.sourceSaved')}</> : t('pluginPlatform.authoring.inProgress')}</span>
        {!readOnly && <Button type='primary' size='small' icon={<Save />} loading={busy} disabled={operationDisabled || (!dirty && Boolean(savedId))} onClick={() => void save()}>{t('pluginPlatform.authoring.saveAndUse')}</Button>}
      </footer>
    </>}
  </section>;
}
