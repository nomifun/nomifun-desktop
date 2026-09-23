import { useCallback, useEffect, useRef, useState } from 'react';
import { Alert, Button, Checkbox, Input, Modal, Spin, Switch } from '@arco-design/web-react';
import {
  ArrowLeft,
  ApiApp,
  CheckOne,
  Code,
  Data,
  Delete,
  Download,
  History,
  Info,
  PlayOne,
  Puzzle,
  Redo,
  SettingTwo,
  Shield,
} from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { useNavigate, useParams, useSearchParams } from 'react-router-dom';
import { pluginPlatform } from '@/common/adapter/pluginPlatformBridge';
import type {
  PluginDetail,
  PluginRestoreMode,
  PluginSurfaceDescriptor,
} from '@/common/types/pluginPlatform';
import { isDesktopShell } from '@/renderer/utils/platform';
import PluginConfigurationDialog from './PluginConfigurationDialog';
import { notifyPluginLibraryChanged } from './pluginLibraryState';
import { pluginNeedsAttention, pluginShape, requiresDataLossWarning } from './pluginPlatformModel';
import PluginWorkspace, { PluginVisual } from './PluginWorkspace';
import PluginSurfacePanel from './PluginSurfacePanel';
import styles from './PluginPlatform.module.css';

type Dialog = 'restore' | 'export_package' | 'export_backup' | 'trash' | 'delete' | 'command' | null;
type DetailTab = 'overview' | 'access' | 'maintenance';

export default function PluginRunPage() {
  const { t, i18n } = useTranslation();
  const navigate = useNavigate();
  const { id = '' } = useParams<{ id: string }>();
  const [searchParams] = useSearchParams();
  const [detail, setDetail] = useState<PluginDetail | null>(null);
  const [surface, setSurface] = useState<PluginSurfaceDescriptor | null>(null);
  const surfaceRef = useRef<PluginSurfaceDescriptor | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [configureVisible, setConfigureVisible] = useState(false);
  const [dialog, setDialog] = useState<Dialog>(null);
  const [destination, setDestination] = useState('');
  const [restoreMode, setRestoreMode] = useState<PluginRestoreMode>('previous_code');
  const [acknowledgeLoss, setAcknowledgeLoss] = useState(false);
  const [commandActionId, setCommandActionId] = useState('');
  const [commandInput, setCommandInput] = useState('{}');
  const [commandOutput, setCommandOutput] = useState('');
  const [activeTab, setActiveTab] = useState<DetailTab>('overview');
  const desktopShell = isDesktopShell();

  const closeSurface = useCallback(async () => {
    if (!desktopShell) return;
    const current = surfaceRef.current;
    surfaceRef.current = null;
    setSurface(null);
    if (!current) return;
    try {
      await pluginPlatform.surface.close.invoke({
        plugin_id: current.plugin_id,
        draft_id: current.draft_id,
        is_preview: current.is_preview,
        request: {
          surface_session_id: current.surface_session_id,
          surface_generation: current.surface_generation,
        },
      });
    } catch {
      // Lifecycle changes revoke the same authority, so an expired close is harmless.
    }
  }, [desktopShell]);

  const load = useCallback(async () => {
    setLoading(true);
    setError('');
    try {
      await closeSurface();
      const next = await pluginPlatform.plugins.get.invoke({ plugin_id: id });
      setDetail(next);
      if (
        desktopShell &&
        next.summary.enabled &&
        next.summary.has_ui &&
        next.summary.trashed_at_ms === undefined
      ) {
        const descriptor = await pluginPlatform.plugins.openSurface.invoke({
          plugin_id: id,
          request: { expected_revision: next.summary.revision },
        });
        surfaceRef.current = descriptor;
        setSurface(descriptor);
      }
    } catch (caught) {
      console.error('[pluginPlatform] Plugin load failed', caught);
      setError(t('pluginPlatform.detail.loadFailed'));
    } finally {
      setLoading(false);
    }
  }, [closeSurface, desktopShell, id, t]);

  useEffect(() => {
    void load();
    return () => {
      const current = surfaceRef.current;
      surfaceRef.current = null;
      if (desktopShell && current) void pluginPlatform.surface.close.invoke({
        plugin_id: current.plugin_id,
        draft_id: current.draft_id,
        is_preview: current.is_preview,
        request: {
          surface_session_id: current.surface_session_id,
          surface_generation: current.surface_generation,
        },
      }).catch(() => undefined);
    };
  }, [desktopShell, load]);

  const mutate = async (operation: (current: PluginDetail) => Promise<unknown>) => {
    if (!desktopShell || !detail || busy) return;
    setBusy(true);
    setError('');
    try {
      await closeSurface();
      const current = await pluginPlatform.plugins.get.invoke({ plugin_id: detail.summary.plugin_id });
      await operation(current);
      setDialog(null);
      notifyPluginLibraryChanged();
      await load();
    } catch (caught) {
      console.error('[pluginPlatform] Plugin mutation failed', caught);
      setError(t('pluginPlatform.detail.operationFailed'));
    } finally {
      setBusy(false);
    }
  };

  const setEnabled = (enabled: boolean) => mutate((current) =>
    pluginPlatform.plugins.setEnabled.invoke({
      plugin_id: current.summary.plugin_id,
      request: { expected_revision: current.summary.revision, enabled },
    }));

  const beginEditing = async () => {
    if (!desktopShell || !detail || busy) return;
    setBusy(true);
    try {
      const draft = await pluginPlatform.drafts.create.invoke({
        plugin_id: detail.summary.plugin_id,
        expected_plugin_revision: detail.summary.revision,
      });
      navigate(`/plugins/create/${encodeURIComponent(draft.summary.draft_id)}`);
    } catch {
      setError(t('pluginPlatform.detail.operationFailed'));
    } finally {
      setBusy(false);
    }
  };

  const configure = async (request: Parameters<typeof pluginPlatform.plugins.configure.invoke>[0]['request']) => {
    if (!desktopShell || !detail) return;
    setBusy(true);
    try {
      const next = await pluginPlatform.plugins.configure.invoke({
        plugin_id: detail.summary.plugin_id,
        request,
      });
      setDetail(next);
      setConfigureVisible(false);
      notifyPluginLibraryChanged();
    } catch {
      setError(t('pluginPlatform.detail.operationFailed'));
    } finally {
      setBusy(false);
    }
  };

  const submitDialog = () => {
    if (!desktopShell || !detail) return;
    if (dialog === 'restore') {
      void mutate((current) => pluginPlatform.plugins.restore.invoke({
        plugin_id: current.summary.plugin_id,
        request: {
          expected_revision: current.summary.revision,
          mode: restoreMode,
          acknowledge_data_loss: acknowledgeLoss,
        },
      }));
    } else if (dialog === 'trash') {
      void mutate((current) => pluginPlatform.plugins.trash.invoke({
        plugin_id: current.summary.plugin_id,
        request: { expected_revision: current.summary.revision },
      }));
    } else if (dialog === 'delete') {
      setBusy(true);
      setError('');
      void closeSurface().then(async () => {
        const current = await pluginPlatform.plugins.get.invoke({ plugin_id: detail.summary.plugin_id });
        await pluginPlatform.plugins.delete.invoke({
          plugin_id: current.summary.plugin_id,
          request: {
            expected_revision: current.summary.revision,
            acknowledge_permanent_delete: true,
          },
        });
        notifyPluginLibraryChanged();
        navigate('/plugins');
      }).catch((caught) => {
        console.error('[pluginPlatform] permanent delete failed', caught);
        setError(t('pluginPlatform.detail.operationFailed'));
      }).finally(() => setBusy(false));
    } else if (dialog === 'export_package') {
      void mutate((current) => pluginPlatform.plugins.exportPackage.invoke({
        plugin_id: current.summary.plugin_id,
        request: {
          expected_revision: current.summary.revision,
          destination_path: destination.trim(),
          include_source: true,
        },
      }));
    } else if (dialog === 'export_backup') {
      void mutate((current) => pluginPlatform.plugins.exportBackup.invoke({
        plugin_id: current.summary.plugin_id,
        request: {
          expected_revision: current.summary.revision,
          destination_path: destination.trim(),
        },
      }));
    } else if (dialog === 'command') {
      let input: unknown;
      try {
        input = JSON.parse(commandInput);
      } catch {
        setError(t('pluginPlatform.command.invalidJson'));
        return;
      }
      setBusy(true);
      setError('');
      void pluginPlatform.desktop.invoke.invoke({ action_id: commandActionId, input })
        .then((output) => {
          setCommandOutput(JSON.stringify(output, null, 2));
          setDialog(null);
        })
        .catch((caught) => {
          console.error('[pluginPlatform] Desktop command failed', caught);
          setError(t('pluginPlatform.command.failed'));
        })
        .finally(() => setBusy(false));
    }
  };

  if (loading) return (
    <PluginWorkspace>
      <main className={styles.page}><div className={styles.emptyState}><Spin /></div></main>
    </PluginWorkspace>
  );
  if (!detail) return (
    <PluginWorkspace>
      <main className={styles.page}><Alert type='error' content={error || t('pluginPlatform.detail.notFound')} showIcon /></main>
    </PluginWorkspace>
  );

  const { summary, manifest } = detail;
  const trashed = summary.trashed_at_ms !== undefined;
  const fullDataRestore = restoreMode === 'previous_code_and_data';
  const needsLossAck = fullDataRestore && requiresDataLossWarning(summary);
  const shape = pluginShape(summary);
  const attention = pluginNeedsAttention(summary) || !detail.config.valid;
  const navView = trashed ? 'trash' : attention ? 'attention' : summary.enabled ? 'enabled' : 'disabled';

  return (
    <PluginWorkspace activeView={navView}>
      <main className={styles.page}>
        <div className={styles.breadcrumb}>
          <button type='button' onClick={() => navigate('/plugins')}>
            <ArrowLeft />{t('pluginPlatform.workspace.title')}
          </button>
          <span>/</span>
          <span>{summary.display_name}</span>
        </div>

        <header className={styles.pluginDetailHeader}>
          <div className={styles.pluginHeroIdentity}>
            <PluginVisual shape={shape} large />
            <div className={styles.headerCopy}>
              <div className={styles.pluginTitleLine}>
                <h1>{summary.display_name}</h1>
                <span className={styles.statusPill} data-tone={trashed ? 'trash' : summary.enabled ? 'enabled' : 'disabled'}>
                  {trashed
                    ? t('pluginPlatform.library.trashed')
                    : summary.enabled
                      ? t('pluginPlatform.actions.enabled')
                      : t('pluginPlatform.actions.disabled')}
                </span>
              </div>
              <p>{summary.description}</p>
              <span className={styles.pluginPackageLine}>{summary.package_id} · v{summary.active.package_version}</span>
            </div>
          </div>
          <div className={styles.headerActions}>
            {desktopShell && summary.has_ui && !trashed && (
              <Button
                type='primary'
                icon={<PlayOne />}
                loading={busy}
                onClick={() => summary.enabled ? void load() : void setEnabled(true)}
              >
                {summary.enabled ? t('pluginPlatform.actions.open') : t('pluginPlatform.actions.enable')}
              </Button>
            )}
            {desktopShell && !trashed && <Switch
              checked={summary.enabled}
              loading={busy}
              checkedText={t('pluginPlatform.actions.enabled')}
              uncheckedText={t('pluginPlatform.actions.disabled')}
              onChange={(enabled) => void setEnabled(enabled)}
            />}
            {desktopShell && !trashed && <Button icon={<Code />} disabled={busy} onClick={() => void beginEditing()}>
              {t('pluginPlatform.actions.edit')}
            </Button>}
            {desktopShell && !trashed && <Button icon={<SettingTwo />} disabled={busy} onClick={() => setConfigureVisible(true)}>
              {t('pluginPlatform.actions.configure')}
            </Button>}
          </div>
        </header>

        {searchParams.has('saved') && <Alert type='success' content={t('pluginPlatform.detail.saved')} showIcon />}
        {error && <Alert type='error' content={error} showIcon />}
        {!desktopShell && <Alert type='info' content={t('pluginPlatform.readOnly.body')} showIcon />}
        {summary.last_error && <Alert type='warning' content={summary.last_error} showIcon />}

        <div className={styles.detailTabs} role='tablist' aria-label={t('pluginPlatform.detail.tabs.label')}>
          {(['overview', 'access', 'maintenance'] as const).map((tab) => (
            <button
              type='button'
              role='tab'
              key={tab}
              aria-selected={activeTab === tab}
              className={activeTab === tab ? styles.detailTabActive : ''}
              onClick={() => setActiveTab(tab)}
            >
              {t(`pluginPlatform.detail.tabs.${tab}`)}
            </button>
          ))}
        </div>

        {activeTab === 'overview' && (
          <div className={styles.detailOverviewGrid}>
            <div className={styles.detailPrimaryColumn}>
              {surface && (
                <section className={styles.surfaceSection}>
                  <PluginSurfacePanel
                    descriptor={surface}
                    title={summary.display_name}
                    closing={busy}
                    onReload={() => void load()}
                    onClose={() => void closeSurface()}
                  />
                </section>
              )}
              {!surface && summary.has_ui && (!summary.enabled || trashed) && (
                <div className={styles.friendlyEmpty}>
                  <PluginVisual shape={shape} />
                  <h2>{trashed ? t('pluginPlatform.library.trashed') : t('pluginPlatform.detail.disabledTitle')}</h2>
                  <p>{t('pluginPlatform.detail.disabledBody')}</p>
                  {desktopShell && !trashed && <Button type='primary' onClick={() => void setEnabled(true)}>
                    {t('pluginPlatform.actions.enable')}
                  </Button>}
                </div>
              )}
              {!summary.has_ui && (
                <section className={`${styles.section} ${styles.capabilityCallout}`}>
                  <div className={styles.sectionHeadingIcon}><ApiApp /></div>
                  <div>
                    <h2>{t('pluginPlatform.detail.headlessTitle')}</h2>
                    <p>{t('pluginPlatform.detail.headlessBody')}</p>
                  </div>
                  <span className={styles.statusPill} data-tone={summary.runtime.state === 'failed' ? 'attention' : 'enabled'}>
                    {t(`pluginPlatform.service.${summary.runtime.state}`)}
                  </span>
                </section>
              )}

              <div className={styles.capabilityGrid}>
                <section className={styles.section}>
                  <div className={styles.sectionTitleRow}>
                    <div><h2>{t('pluginPlatform.detail.actions')}</h2><p>{t('pluginPlatform.detail.actionsHelp')}</p></div>
                    <span className={styles.sectionCount}>{manifest.actions.length}</span>
                  </div>
                  <div className={styles.bindingList}>
                    {manifest.actions.map((action) => (
                      <div key={action.action_id} className={styles.capabilityRow}>
                        <span className={styles.capabilityRowIcon}><Code /></span>
                        <span className={styles.capabilityRowCopy}>
                          <strong>{action.name}</strong><small>{action.description}</small>
                        </span>
                        <span className={styles.capabilityRowMeta}>
                          <code>{action.stable_id ?? action.action_id}</code>
                          {desktopShell && action.stable_id && summary.enabled && !trashed && manifest.bindings.some(
                            (binding) => binding.point === 'desktop.command' && binding.action_id === action.action_id,
                          ) && <Button type='text' size='mini' onClick={() => {
                            setCommandActionId(action.stable_id!);
                            setCommandInput('{}');
                            setCommandOutput('');
                            setDialog('command');
                          }}>
                            {t('pluginPlatform.command.run')}
                          </Button>}
                        </span>
                      </div>
                    ))}
                    {!manifest.actions.length && <p className={styles.muted}>{t('pluginPlatform.detail.noActions')}</p>}
                  </div>
                </section>
                <section className={styles.section}>
                  <div className={styles.sectionTitleRow}>
                    <div><h2>{t('pluginPlatform.detail.bindings')}</h2><p>{t('pluginPlatform.detail.bindingsHelp')}</p></div>
                    <span className={styles.sectionCount}>{manifest.bindings.length}</span>
                  </div>
                  <div className={styles.bindingList}>
                    {manifest.bindings.map((binding) => (
                      <div key={`${binding.point}:${binding.action_id}`} className={styles.capabilityRow}>
                        <span className={styles.capabilityRowIcon}><Puzzle /></span>
                        <span className={styles.capabilityRowCopy}>
                          <strong>{binding.point}</strong><small>{binding.action_id}</small>
                        </span>
                        <span className={styles.statusPill} data-tone={binding.supported ? 'enabled' : 'attention'}>
                          {binding.supported ? t('pluginPlatform.detail.available') : binding.unavailable_reason}
                        </span>
                      </div>
                    ))}
                    {!manifest.bindings.length && <p className={styles.muted}>{t('pluginPlatform.detail.noBindings')}</p>}
                  </div>
                </section>
              </div>
            </div>

            <aside className={styles.detailRail}>
              <section className={styles.section}>
                <div className={styles.sectionTitleRow}><h2>{t('pluginPlatform.detail.runtimeStatus')}</h2></div>
                <div className={styles.statusList}>
                  <div><CheckOne /><span>{t('pluginPlatform.detail.recentCheck')}</span><strong>{attention ? t('pluginPlatform.detail.needsReview') : t('pluginPlatform.detail.passed')}</strong></div>
                  <div><History /><span>{t('pluginPlatform.detail.currentVersion')}</span><strong>v{summary.active.package_version}</strong></div>
                  <div><Info /><span>{t('pluginPlatform.detail.runtime')}</span><strong>{t(`pluginPlatform.service.${summary.runtime.state}`)}</strong></div>
                  <div><SettingTwo /><span>{t('pluginPlatform.detail.configuration')}</span><strong>{t(detail.config.valid ? 'pluginPlatform.detail.configurationValid' : 'pluginPlatform.detail.configurationInvalid')}</strong></div>
                </div>
              </section>
              <section className={styles.section}>
                <div className={styles.sectionTitleRow}><h2>{t('pluginPlatform.detail.summary')}</h2></div>
                <div className={styles.summaryMetrics}>
                  <div><strong>{manifest.actions.length}</strong><span>{t('pluginPlatform.detail.actions')}</span></div>
                  <div><strong>{manifest.bindings.length}</strong><span>{t('pluginPlatform.detail.bindings')}</span></div>
                  <div><strong>{detail.grants.filter((grant) => grant.granted).length}</strong><span>{t('pluginPlatform.detail.permissions')}</span></div>
                </div>
                <p className={styles.updatedSummary}>{t('pluginPlatform.detail.updatedAt', { date: new Date(summary.updated_at_ms).toLocaleString(i18n.language) })}</p>
              </section>
            </aside>
          </div>
        )}

        {activeTab === 'access' && (
          <div className={styles.detailGrid}>
            <section className={styles.section}>
              <div className={styles.sectionTitleRow}>
                <div><h2>{t('pluginPlatform.detail.configuration')}</h2><p>{t('pluginPlatform.detail.configurationHelp')}</p></div>
                <span className={styles.statusPill} data-tone={detail.config.valid ? 'enabled' : 'attention'}>
                  {t(detail.config.valid ? 'pluginPlatform.detail.configurationValid' : 'pluginPlatform.detail.configurationInvalid')}
                </span>
              </div>
              <details className={styles.codeDisclosure}>
                <summary>{t('pluginPlatform.detail.showConfiguration')}</summary>
                <pre>{JSON.stringify(detail.config.values, null, 2)}</pre>
              </details>
              {!detail.config.valid && detail.config.validation_errors.length > 0 && (
                <Alert type='warning' content={detail.config.validation_errors.join('\n')} showIcon />
              )}
              {desktopShell && !trashed && <Button icon={<SettingTwo />} onClick={() => setConfigureVisible(true)}>
                {t('pluginPlatform.actions.configure')}
              </Button>}
            </section>
            <section className={styles.section}>
              <div className={styles.sectionTitleRow}>
                <div><h2>{t('pluginPlatform.detail.permissions')}</h2><p>{t('pluginPlatform.detail.permissionsHelp')}</p></div>
                <Shield />
              </div>
              <div className={styles.bindingList}>
                {detail.grants.map((grant) => <div key={grant.permission} className={styles.permissionRow}>
                  <span><Shield /><code>{grant.permission}</code></span>
                  <span className={styles.statusPill} data-tone={grant.granted ? 'enabled' : 'disabled'}>
                    {grant.granted ? t('pluginPlatform.detail.granted') : t('pluginPlatform.detail.denied')}
                  </span>
                </div>)}
                {!detail.grants.length && <p className={styles.muted}>{t('pluginPlatform.detail.noPermissions')}</p>}
              </div>
            </section>
            <section className={styles.section}>
              <div className={styles.sectionTitleRow}><div><h2>{t('pluginPlatform.detail.storage')}</h2><p>{t('pluginPlatform.detail.storageHelp')}</p></div><Data /></div>
              <div className={styles.facts}>
                <div className={styles.fact}><span>{t('pluginPlatform.detail.artifact')}</span><code>{summary.active.artifact_digest.slice(0, 16)}…</code></div>
                <div className={styles.fact}><span>{t('pluginPlatform.detail.dataGeneration')}</span><code>{summary.active.data_generation}</code></div>
                <div className={styles.fact}><span>{t('pluginPlatform.detail.dataVersion')}</span><code>{summary.active.data_version}</code></div>
                <div className={styles.fact}><span>{t('pluginPlatform.detail.credentials')}</span><code>{detail.credential_bindings.length}</code></div>
              </div>
            </section>
          </div>
        )}

        {activeTab === 'maintenance' && (
          <div className={styles.maintenanceGrid}>
            <section className={styles.section}>
              <div className={styles.sectionTitleRow}>
                <div><h2>{t('pluginPlatform.detail.versionAndData')}</h2><p>{t('pluginPlatform.detail.versionAndDataHelp')}</p></div>
                <History />
              </div>
              <div className={styles.facts}>
                <div className={styles.fact}><span>{t('pluginPlatform.detail.currentVersion')}</span><code>v{summary.active.package_version}</code></div>
                <div className={styles.fact}><span>{t('pluginPlatform.detail.packageId')}</span><code>{summary.package_id}</code></div>
                <div className={styles.fact}><span>{t('pluginPlatform.detail.revision')}</span><code>{summary.revision}</code></div>
              </div>
            </section>
            {desktopShell && <section className={styles.section}>
              <div className={styles.sectionTitleRow}><div><h2>{t('pluginPlatform.detail.maintenanceActions')}</h2><p>{t('pluginPlatform.detail.maintenanceHelp')}</p></div></div>
              <div className={styles.maintenanceActions}>
                {!trashed && summary.previous && <Button icon={<Redo />} disabled={busy} onClick={() => {
                  setRestoreMode('previous_code'); setAcknowledgeLoss(false); setDialog('restore');
                }}>{t('pluginPlatform.actions.restore')}</Button>}
                {trashed && <Button type='primary' icon={<Redo />} disabled={busy} onClick={() => void mutate((current) => pluginPlatform.plugins.restore.invoke({
                  plugin_id: current.summary.plugin_id,
                  request: { expected_revision: current.summary.revision, mode: 'from_trash' },
                }))}>{t('pluginPlatform.actions.restoreTrash')}</Button>}
                {!trashed && <Button icon={<Download />} disabled={busy} onClick={() => { setDestination(''); setDialog('export_package'); }}>
                  {t('pluginPlatform.actions.exportPackage')}
                </Button>}
                {!trashed && <Button icon={<Download />} disabled={busy} onClick={() => { setDestination(''); setDialog('export_backup'); }}>
                  {t('pluginPlatform.actions.exportBackup')}
                </Button>}
                {!trashed ? <Button status='danger' icon={<Delete />} disabled={busy} onClick={() => setDialog('trash')}>
                  {t('pluginPlatform.actions.trash')}
                </Button> : <Button status='danger' icon={<Delete />} disabled={busy} onClick={() => setDialog('delete')}>
                  {t('pluginPlatform.actions.delete')}
                </Button>}
              </div>
            </section>}
            {commandOutput && <section className={styles.section}>
              <h2>{t('pluginPlatform.command.result')}</h2>
              <pre>{commandOutput}</pre>
            </section>}
          </div>
        )}
      </main>

      {desktopShell && <PluginConfigurationDialog
        detail={detail}
        visible={configureVisible}
        loading={busy}
        onCancel={() => setConfigureVisible(false)}
        onSubmit={configure}
      />}
      <Modal
        visible={desktopShell && dialog !== null}
        title={dialog ? t(`pluginPlatform.dialog.${dialog}.title`) : ''}
        confirmLoading={busy}
        okButtonProps={{
          status: dialog === 'delete' || dialog === 'trash' ? 'danger' : undefined,
          disabled: (dialog === 'export_package' || dialog === 'export_backup') && !destination.trim()
            || dialog === 'restore' && needsLossAck && !acknowledgeLoss,
        }}
        onCancel={busy ? undefined : () => setDialog(null)}
        onOk={submitDialog}
        unmountOnExit
      >
        <div className={styles.modalBody}>
          {(dialog === 'export_package' || dialog === 'export_backup') && <>
            <Alert type='info' content={t(dialog === 'export_package'
              ? 'pluginPlatform.dialog.export_package.disclosure'
              : 'pluginPlatform.dialog.export_backup.disclosure')} showIcon />
            <Input value={destination} onChange={setDestination} placeholder={t('pluginPlatform.dialog.destination')} />
          </>}
          {dialog === 'restore' && <>
            <Checkbox checked={restoreMode === 'previous_code_and_data'} onChange={(checked) => {
              setRestoreMode(checked ? 'previous_code_and_data' : 'previous_code');
              setAcknowledgeLoss(false);
            }}>{t('pluginPlatform.dialog.restore.withData')}</Checkbox>
            {needsLossAck && <Alert type='warning' content={t('pluginPlatform.dialog.restore.dataLoss')} showIcon />}
            {needsLossAck && <Checkbox checked={acknowledgeLoss} onChange={setAcknowledgeLoss}>
              {t('pluginPlatform.dialog.restore.acknowledge')}
            </Checkbox>}
          </>}
          {dialog === 'trash' && <Alert type='warning' content={t('pluginPlatform.dialog.trash.body')} showIcon />}
          {dialog === 'delete' && <Alert type='error' content={t('pluginPlatform.dialog.delete.body')} showIcon />}
          {dialog === 'command' && <>
            <p><code>{commandActionId}</code></p>
            <Input.TextArea
              value={commandInput}
              onChange={setCommandInput}
              spellCheck={false}
              autoSize={{ minRows: 4, maxRows: 12 }}
              aria-label={t('pluginPlatform.command.input')}
            />
          </>}
        </div>
      </Modal>
    </PluginWorkspace>
  );
}
