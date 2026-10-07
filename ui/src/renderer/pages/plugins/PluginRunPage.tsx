import { useCallback, useEffect, useRef, useState } from 'react';
import { Alert, Button, Modal, Spin, Switch } from '@arco-design/web-react';
import { ArrowLeft, ArrowRight, ApiApp, Code, Delete, Download, EditTwo, FolderClose, Refresh, SettingTwo } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { useNavigate, useParams, useSearchParams } from 'react-router-dom';
import { ipcBridge } from '@/common';
import { pluginPlatform } from '@/common/adapter/pluginPlatformBridge';
import type { PluginDetail, PluginSurfaceDescriptor } from '@/common/types/pluginPlatform';
import { isDesktopShell } from '@/renderer/utils/platform';
import PluginConfigurationDialog from './PluginConfigurationDialog';
import { notifyPluginLibraryChanged, subscribePluginLibraryChanges } from './pluginLibraryState';
import { pluginShape } from './pluginPlatformModel';
import PluginWorkspace, { PluginVisual } from './PluginWorkspace';
import PluginSurfacePanel from './PluginSurfacePanel';
import PluginParameterFields from './PluginParameterFields';
import { pluginParameterDefaults } from './pluginParameterModel';
import { launchPluginAuthoring } from './pluginAuthoringLaunch';
import styles from './PluginPlatform.module.css';

type Dialog = 'trash' | 'delete' | 'command' | null;
export default function PluginRunPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { id = '' } = useParams<{ id: string }>();
  const [searchParams] = useSearchParams();
  const [detail, setDetail] = useState<PluginDetail | null>(null);
  const detailRef = useRef<PluginDetail | null>(null);
  const [customName, setCustomName] = useState('');
  const [surface, setSurface] = useState<PluginSurfaceDescriptor | null>(null);
  const surfaceRef = useRef<PluginSurfaceDescriptor | null>(null);
  const sequence = useRef(0);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [feedback, setFeedback] = useState('');
  const [configureVisible, setConfigureVisible] = useState(false);
  const [dialog, setDialog] = useState<Dialog>(null);
  const [commandActionId, setCommandActionId] = useState('');
  const [commandName, setCommandName] = useState('');
  const [commandSchema, setCommandSchema] = useState<Record<string, unknown>>({});
  const [commandInput, setCommandInput] = useState('{}');
  const [commandOutput, setCommandOutput] = useState('');
  const [activeTab, setActiveTab] = useState<'use' | 'manage'>('use');
  const desktopShell = isDesktopShell();

  const release = useCallback(async (current: PluginSurfaceDescriptor | null) => {
    if (!desktopShell || !current) return;
    await pluginPlatform.surface.close.invoke({ plugin_id: current.plugin_id, draft_id: current.draft_id, is_preview: current.is_preview,
      request: { surface_session_id: current.surface_session_id, surface_generation: current.surface_generation },
    }).catch(() => undefined);
  }, [desktopShell]);
  const closeSurface = useCallback(async () => {
    const current = surfaceRef.current;
    surfaceRef.current = null; setSurface(null); await release(current);
  }, [release]);
  const load = useCallback(async () => {
    const request = ++sequence.current;
    setLoading(true); setError('');
    try {
      await closeSurface();
      const next = await pluginPlatform.plugins.get.invoke({ plugin_id: id });
      if (request !== sequence.current) return;
      detailRef.current = next; setDetail(next);
      void pluginPlatform.libraryState.get.invoke().then(state => {
        if (request === sequence.current) setCustomName(state.items.find(item => item.plugin_id === id)?.custom_name ?? '');
      }).catch(() => undefined);
      if (desktopShell && next.summary.enabled && next.summary.has_ui && next.summary.trashed_at_ms === undefined) {
        try {
          const descriptor = await pluginPlatform.plugins.openSurface.invoke({ plugin_id: id, request: { expected_revision: next.summary.revision } });
          if (request !== sequence.current) { await release(descriptor); return; }
          surfaceRef.current = descriptor; setSurface(descriptor);
        } catch { if (request === sequence.current) setError(t('pluginPlatform.preview.surfaceFailed')); }
      }
    } catch { if (request === sequence.current) setError(t('pluginPlatform.detail.loadFailed')); }
    finally { if (request === sequence.current) setLoading(false); }
  }, [closeSurface, desktopShell, id, release, t]);
  useEffect(() => {
    detailRef.current = null; setDetail(null); setCustomName(''); setFeedback(''); setCommandOutput(''); setActiveTab('use');
  }, [id]);
  useEffect(() => {
    void load();
    return () => { sequence.current += 1; const current = surfaceRef.current; surfaceRef.current = null; void release(current); };
  }, [load, release]);
  useEffect(() => {
    let active = true;
    let request = 0;
    const off = subscribePluginLibraryChanges(() => {
      const currentRequest = ++request;
      void Promise.all([pluginPlatform.plugins.get.invoke({ plugin_id: id }), pluginPlatform.libraryState.get.invoke().catch(() => null)]).then(([next, state]) => {
        if (!active || currentRequest !== request) return;
        if (state) setCustomName(state.items.find(item => item.plugin_id === id)?.custom_name ?? '');
        const previous = detailRef.current?.summary;
        if (previous && (previous.revision !== next.summary.revision || previous.active.artifact_digest !== next.summary.active.artifact_digest ||
          previous.enabled !== next.summary.enabled || previous.trashed_at_ms !== next.summary.trashed_at_ms)) void load();
        else { detailRef.current = next; setDetail(next); }
      }).catch(() => undefined);
    });
    return () => { active = false; off(); };
  }, [id, load]);

  const mutate = async (operation: (current: PluginDetail) => Promise<unknown>) => {
    if (!desktopShell || !detail || busy || loading) return false;
    setBusy(true); setError('');
    try {
      await closeSurface();
      const current = await pluginPlatform.plugins.get.invoke({ plugin_id: id });
      await operation(current); setDialog(null); await load(); notifyPluginLibraryChanged(); return true;
    } catch { setError(t('pluginPlatform.detail.operationFailed')); return false; }
    finally { setBusy(false); }
  };
  const setEnabled = (enabled: boolean) => mutate(current => pluginPlatform.plugins.setEnabled.invoke({
    plugin_id: id, request: { expected_revision: current.summary.revision, enabled },
  }));
  const beginEditing = async () => {
    if (!desktopShell || !detail || busy || loading) return;
    setBusy(true);
    try { await launchPluginAuthoring(navigate, { plugin_id: id, expected_plugin_revision: detail.summary.revision }); }
    catch { setError(t('pluginPlatform.detail.operationFailed')); }
    finally { setBusy(false); }
  };
  const configure = async (request: Parameters<typeof pluginPlatform.plugins.configure.invoke>[0]['request']) => {
    if (!desktopShell || !detail || busy) return;
    const saved = await mutate(() => pluginPlatform.plugins.configure.invoke({ plugin_id: id, request }));
    if (saved) setConfigureVisible(false);
    else throw new Error(t('pluginPlatform.detail.operationFailed'));
  };
  const exportPlugin = async (backup: boolean) => {
    if (!desktopShell || !detail || busy) return;
    setBusy(true); setError('');
    try {
      const paths = await ipcBridge.dialog.showOpen.invoke({ properties: ['openDirectory'] });
      if (!paths?.[0]) return;
      const current = await pluginPlatform.plugins.get.invoke({ plugin_id: id });
      const filename = current.summary.package_id.replace(/[^a-zA-Z0-9._-]/g, '-') + (backup ? '-backup-' : '-') + Date.now() + '.zip';
      const destination = paths[0].replace(/[\\/]+$/, '') + '/' + filename;
      const result = backup
        ? await pluginPlatform.plugins.exportBackup.invoke({ plugin_id: id, request: { expected_revision: current.summary.revision, destination_path: destination } })
        : await pluginPlatform.plugins.exportPackage.invoke({ plugin_id: id, request: { expected_revision: current.summary.revision, destination_path: destination, include_source: true } });
      setFeedback(t('pluginPlatform.detail.exported', { path: result.destination_path }));
    } catch { setError(t('pluginPlatform.detail.operationFailed')); }
    finally { setBusy(false); }
  };
  const submitDialog = async () => {
    if (!desktopShell || !detail || busy) return;
    if (dialog === 'trash') { await mutate(current => pluginPlatform.plugins.trash.invoke({ plugin_id: id, request: { expected_revision: current.summary.revision } })); return; }
    if (dialog === 'delete') {
      setBusy(true); setError('');
      try {
        await closeSurface();
        const current = await pluginPlatform.plugins.get.invoke({ plugin_id: id });
        await pluginPlatform.plugins.delete.invoke({ plugin_id: id, request: { expected_revision: current.summary.revision, acknowledge_permanent_delete: true } });
        notifyPluginLibraryChanged(); navigate('/plugins?view=trash');
      } catch { setError(t('pluginPlatform.detail.operationFailed')); }
      finally { setBusy(false); }
      return;
    }
    if (dialog === 'command') {
      let input: unknown;
      try { input = JSON.parse(commandInput); } catch { setError(t('pluginPlatform.command.invalidJson')); return; }
      setBusy(true); setError('');
      try { const output = await pluginPlatform.desktop.invoke.invoke({ action_id: commandActionId, input }); setCommandOutput(JSON.stringify(output, null, 2)); setDialog(null); }
      catch { setError(t('pluginPlatform.command.failed')); }
      finally { setBusy(false); }
    }
  };

  if (loading && !detail) return <PluginWorkspace><main className={styles.page}><div className={styles.emptyState}><Spin /></div></main></PluginWorkspace>;
  if (!detail) return <PluginWorkspace><main className={styles.page}><Alert type='error' content={error || t('pluginPlatform.detail.notFound')} action={<Button onClick={() => void load()}>{t('pluginPlatform.actions.refresh')}</Button>} /></main></PluginWorkspace>;
  const { summary, manifest } = detail;
  const trashed = summary.trashed_at_ms !== undefined;
  const name = customName || summary.display_name;
  const shape = pluginShape(summary);

  return <PluginWorkspace activeView={trashed ? 'trash' : 'all'}>
    <main className={styles.page}>
      <div className={styles.breadcrumb}><button type='button' onClick={() => navigate('/plugins')}><ArrowLeft />{t('pluginPlatform.actions.back')}</button></div>
      <header className={styles.pluginDetailHeader}>
        <div className={styles.pluginHeroIdentity}><PluginVisual shape={shape} large /><div className={styles.headerCopy}>
          <div className={styles.pluginTitleLine}><h1>{name}</h1><span className={styles.kindBadge}>{t('pluginPlatform.shape.' + shape)}</span></div>
          <p>{summary.description}</p>
        </div></div>
        {desktopShell && !trashed && <div className={styles.headerActions}>
          <Button icon={<EditTwo />} loading={busy} onClick={() => void beginEditing()}>{t('pluginPlatform.actions.edit')}</Button>
          <Button type='text' icon={<Refresh />} aria-label={t('pluginPlatform.actions.refresh')} title={t('pluginPlatform.actions.refresh')} loading={loading} onClick={() => void load()} />
        </div>}
      </header>
      {searchParams.has('saved') && <Alert type='success' content={t('pluginPlatform.detail.saved')} showIcon />}
      {feedback && <Alert type='success' content={feedback} closable onClose={() => setFeedback('')} showIcon />}
      {error && <Alert type='error' content={error} showIcon />}
      {!desktopShell && <Alert type='info' content={t('pluginPlatform.readOnly.body')} showIcon />}
      {summary.last_error && <Alert type='warning' content={summary.last_error} showIcon />}
      <div className={styles.detailTabs} role='tablist' aria-label={t('pluginPlatform.detail.tabs.label')}>
        {(['use', 'manage'] as const).map(tab => <button key={tab} type='button' role='tab' id={'plugin-tab-' + tab} aria-controls={'plugin-panel-' + tab}
          aria-selected={activeTab === tab} className={activeTab === tab ? styles.detailTabActive : ''} onClick={() => setActiveTab(tab)}>
          {t('pluginPlatform.detail.tabs.' + tab)}
        </button>)}
      </div>
      {activeTab === 'use' ? <div role='tabpanel' id='plugin-panel-use' aria-labelledby='plugin-tab-use' className={styles.detailPrimaryColumn}>
        {surface && <section className={styles.surfaceSection}><PluginSurfacePanel descriptor={surface} title={name} closing={busy} onReload={() => void load()} onClose={closeSurface} /></section>}
        {!surface && summary.has_ui && <div className={styles.friendlyEmpty}><PluginVisual shape={shape} large />
          <h2>{t(trashed ? 'pluginPlatform.library.trashed' : !summary.enabled ? 'pluginPlatform.detail.disabledTitle' : 'pluginPlatform.detail.openTitle')}</h2>
          <p>{t(trashed ? 'pluginPlatform.detail.trashedBody' : !summary.enabled ? 'pluginPlatform.detail.disabledBody' : 'pluginPlatform.detail.openBody')}</p>
          {desktopShell && !trashed && <Button type='primary' loading={busy || loading} onClick={() => summary.enabled ? void load() : void setEnabled(true)}>
            {t(summary.enabled ? 'pluginPlatform.actions.open' : 'pluginPlatform.actions.enable')}
          </Button>}
          {desktopShell && trashed && <Button type='primary' loading={busy} onClick={() => void mutate(current => pluginPlatform.plugins.restore.invoke({ plugin_id: id, request: { expected_revision: current.summary.revision } }))}>{t('pluginPlatform.actions.restoreTrash')}</Button>}
        </div>}
        {!summary.has_ui && <section className={styles.capabilityCallout}><span className={styles.sectionHeadingIcon}><ApiApp /></span>
          <div><h2>{t('pluginPlatform.detail.headlessTitle')}</h2><p>{t('pluginPlatform.detail.headlessBody')}</p></div>
          <span className={styles.kindBadge}>{t(summary.enabled ? 'pluginPlatform.actions.enabled' : 'pluginPlatform.actions.disabled')}</span>
        </section>}
        {!!manifest.actions.length && <section className={styles.section}>
          <div className={styles.sectionTitleRow}><div><h2>{t('pluginPlatform.detail.actions')}</h2><p>{t('pluginPlatform.detail.actionsHelp')}</p></div><span className={styles.sectionCount}>{manifest.actions.length}</span></div>
          <div className={styles.bindingList}>{manifest.actions.map(action => <div key={action.action_id} className={styles.capabilityRow}>
            <span className={styles.capabilityRowIcon}><Code /></span><div className={styles.capabilityRowCopy}><strong>{action.name}</strong><small>{action.description}</small></div>
            {desktopShell && action.stable_id && summary.enabled && !trashed && manifest.bindings.some(binding => binding.point === 'desktop.command' && binding.action_id === action.action_id) &&
              <Button onClick={() => { setError(''); setCommandActionId(action.stable_id!); setCommandName(action.name); setCommandSchema(action.input_schema); setCommandInput(JSON.stringify(pluginParameterDefaults(action.input_schema), null, 2)); setDialog('command'); }}>{t('pluginPlatform.command.run')}<ArrowRight /></Button>}
          </div>)}</div>
        </section>}
        {!!manifest.bindings.length && <details className={styles.section}><summary className={styles.sectionSummary}>{t('pluginPlatform.detail.bindings')}</summary>
          <div className={styles.bindingList}>{manifest.bindings.map(binding => <div key={binding.point + ':' + binding.action_id} className={styles.capabilityRow}>
            <div className={styles.capabilityRowCopy}><strong>{t('pluginPlatform.detail.bindingNames.' + binding.point.replaceAll('.', '_'), { defaultValue: binding.point })}</strong>
              <small>{manifest.actions.find(action => action.action_id === binding.action_id)?.name ?? binding.action_id}</small></div>
            {!binding.supported && <span className={styles.muted}>{binding.unavailable_reason}</span>}
          </div>)}</div>
        </details>}
        {commandOutput && <section className={styles.section}><h2>{t('pluginPlatform.command.result')}</h2><pre className={styles.commandResult}>{commandOutput}</pre></section>}
      </div> : <div role='tabpanel' id='plugin-panel-manage' aria-labelledby='plugin-tab-manage' className={styles.management}>
        <section className={styles.section}>
          <div className={styles.manageRow}><span className={styles.sectionHeadingIcon}><SettingTwo /></span><div><h2>{t('pluginPlatform.detail.enabledTitle')}</h2><p>{t('pluginPlatform.detail.enabledHelp')}</p></div>
            {!trashed && <Switch checked={summary.enabled} loading={busy} disabled={!desktopShell} aria-label={t('pluginPlatform.library.toggle', { name })} onChange={enabled => void setEnabled(enabled)} />}
          </div>
          {!trashed && <div className={styles.manageRow}><span className={styles.sectionHeadingIcon}><Code /></span><div><h2>{t('pluginPlatform.actions.configure')}</h2><p>{t('pluginPlatform.detail.configurationHelp')}</p></div>
            {desktopShell && <Button disabled={busy} onClick={() => setConfigureVisible(true)}>{t('pluginPlatform.actions.configure')}</Button>}
          </div>}
          {desktopShell && !trashed && <div className={styles.manageRow}><span className={styles.sectionHeadingIcon}><Download /></span><div><h2>{t('pluginPlatform.detail.exportTitle')}</h2><p>{t('pluginPlatform.detail.exportHelp')}</p></div>
            <div className={styles.actions}><Button disabled={busy} onClick={() => void exportPlugin(false)}>{t('pluginPlatform.actions.exportPackage')}</Button><Button disabled={busy} onClick={() => void exportPlugin(true)}>{t('pluginPlatform.actions.exportBackup')}</Button></div>
          </div>}
          {desktopShell && <div className={styles.manageRow}><span className={styles.sectionHeadingIcon}><FolderClose /></span><div><h2>{t(trashed ? 'pluginPlatform.detail.restoreTitle' : 'pluginPlatform.detail.removeTitle')}</h2><p>{t(trashed ? 'pluginPlatform.detail.trashedBody' : 'pluginPlatform.dialog.trash.body')}</p></div>
            <div className={styles.actions}>{trashed && <Button loading={busy} onClick={() => void mutate(current => pluginPlatform.plugins.restore.invoke({ plugin_id: id, request: { expected_revision: current.summary.revision } }))}>{t('pluginPlatform.actions.restoreTrash')}</Button>}
              <Button status='danger' icon={<Delete />} disabled={busy} onClick={() => setDialog(trashed ? 'delete' : 'trash')}>{t(trashed ? 'pluginPlatform.actions.delete' : 'pluginPlatform.actions.trash')}</Button></div>
          </div>}
        </section>
      </div>}
    </main>
    {desktopShell && <PluginConfigurationDialog detail={detail} visible={configureVisible} loading={busy} onCancel={() => setConfigureVisible(false)} onSubmit={configure} />}
    <Modal visible={desktopShell && !!dialog} title={dialog === 'command' ? commandName : dialog ? t('pluginPlatform.dialog.' + dialog + '.title') : ''}
      okText={t(dialog === 'command' ? 'pluginPlatform.command.run' : dialog === 'delete' ? 'pluginPlatform.actions.delete' : 'pluginPlatform.actions.trash')} cancelText={t('pluginPlatform.actions.cancel')}
      confirmLoading={busy} okButtonProps={{ status: dialog === 'delete' || dialog === 'trash' ? 'danger' : undefined }} onCancel={busy ? undefined : () => setDialog(null)} onOk={() => void submitDialog()} unmountOnExit>
      <div className={styles.modalBody}>{dialog === 'trash' && <p>{t('pluginPlatform.dialog.trash.body')}</p>}
        {dialog === 'delete' && <p>{t('pluginPlatform.dialog.delete.body')}</p>}
        {dialog === 'command' && <><PluginParameterFields schema={commandSchema} value={commandInput} onChange={value => { setCommandInput(value); setError(''); }} disabled={busy} jsonLabel={t('pluginPlatform.command.input')} />
          {error && <Alert type='error' content={error} showIcon />}</>}
      </div>
    </Modal>
  </PluginWorkspace>;
}
