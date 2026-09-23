import { useCallback, useEffect, useRef, useState } from 'react';
import { Alert, Button, Checkbox, Input, Modal, Spin, Switch, Tag } from '@arco-design/web-react';
import { ArrowLeft, Code, Delete, Download, Redo, SettingTwo } from '@icon-park/react';
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
import { pluginShape, requiresDataLossWarning } from './pluginPlatformModel';
import PluginSurfacePanel from './PluginSurfacePanel';
import styles from './PluginPlatform.module.css';

type Dialog = 'restore' | 'export_package' | 'export_backup' | 'trash' | 'delete' | 'command' | null;

export default function PluginRunPage() {
  const { t } = useTranslation();
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

  if (loading) return <main className={styles.page}><div className={styles.empty}><Spin /></div></main>;
  if (!detail) return <main className={styles.page}><Alert type='error' content={error || t('pluginPlatform.detail.notFound')} /></main>;

  const { summary, manifest } = detail;
  const trashed = summary.trashed_at_ms !== undefined;
  const fullDataRestore = restoreMode === 'previous_code_and_data';
  const needsLossAck = fullDataRestore && requiresDataLossWarning(summary);

  return (
    <main className={styles.page}>
      <header className={styles.header}>
        <div className={styles.actions}>
          <Button type='text' icon={<ArrowLeft />} onClick={() => navigate('/plugins')}>
            {t('pluginPlatform.actions.back')}
          </Button>
          <div className={styles.headerCopy}>
            <div className={styles.actions}>
              <h1>{summary.display_name}</h1>
              <Tag>{t(`pluginPlatform.shape.${pluginShape(summary)}`)}</Tag>
            </div>
            <p>{summary.description}</p>
          </div>
        </div>
        <div className={styles.actions}>
          {!desktopShell && <Tag color={summary.enabled ? 'green' : 'gray'}>
            {summary.enabled ? t('pluginPlatform.actions.enabled') : t('pluginPlatform.actions.disabled')}
          </Tag>}
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
      {searchParams.has('saved') && <Alert type='success' content={t('pluginPlatform.detail.saved')} />}
      {error && <Alert type='error' content={error} />}
      {!desktopShell && <Alert type='info' content={t('pluginPlatform.readOnly.body')} />}
      {summary.last_error && <Alert type='warning' content={summary.last_error} />}
      {surface && (
        <PluginSurfacePanel
          descriptor={surface}
          title={summary.display_name}
          closing={busy}
          onReload={() => void load()}
          onClose={() => void closeSurface()}
        />
      )}
      {!surface && summary.has_ui && !summary.enabled && (
        <div className={styles.empty}>
          <h2>{t('pluginPlatform.detail.disabledTitle')}</h2>
          <p>{t('pluginPlatform.detail.disabledBody')}</p>
          {desktopShell && !trashed && <Button type='primary' onClick={() => void setEnabled(true)}>{t('pluginPlatform.actions.enable')}</Button>}
        </div>
      )}
      {!summary.has_ui && (
        <section className={styles.section}>
          <div className={styles.sectionHeader}>
            <div><h2>{t('pluginPlatform.detail.headlessTitle')}</h2><p>{t('pluginPlatform.detail.headlessBody')}</p></div>
            <Tag color={summary.runtime.state === 'failed' ? 'red' : 'green'}>
              {t(`pluginPlatform.service.${summary.runtime.state}`)}
            </Tag>
          </div>
        </section>
      )}
      <div className={styles.detailGrid}>
        <section className={styles.section}>
          <h2>{t('pluginPlatform.detail.actions')}</h2>
          <div className={styles.bindingList}>
            {manifest.actions.map((action) => (
              <div key={action.action_id} className={styles.binding}>
                <span><strong>{action.name}</strong><br /><small>{action.description}</small></span>
                <span>
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
          <h2>{t('pluginPlatform.detail.bindings')}</h2>
          <div className={styles.bindingList}>
            {manifest.bindings.map((binding) => (
              <div key={`${binding.point}:${binding.action_id}`} className={styles.binding}>
                <code>{binding.point}</code>
                <span>{binding.supported ? t('pluginPlatform.detail.available') : binding.unavailable_reason}</span>
              </div>
            ))}
            {!manifest.bindings.length && <p className={styles.muted}>{t('pluginPlatform.detail.noBindings')}</p>}
          </div>
        </section>
        <section className={styles.section}>
          <h2>{t('pluginPlatform.detail.storage')}</h2>
          <div className={styles.facts}>
            <div className={styles.fact}><span>{t('pluginPlatform.detail.artifact')}</span><code>{summary.active.artifact_digest.slice(0, 16)}…</code></div>
            <div className={styles.fact}><span>{t('pluginPlatform.detail.dataGeneration')}</span><code>{summary.active.data_generation}</code></div>
            <div className={styles.fact}><span>{t('pluginPlatform.detail.dataVersion')}</span><code>{summary.active.data_version}</code></div>
            <div className={styles.fact}><span>{t('pluginPlatform.detail.credentials')}</span><code>{detail.credential_bindings.length}</code></div>
          </div>
        </section>
        <section className={styles.section}>
          <h2>{t('pluginPlatform.detail.configuration')}</h2>
          <Tag color={detail.config.valid ? 'green' : 'red'}>
            {t(detail.config.valid
              ? 'pluginPlatform.detail.configurationValid'
              : 'pluginPlatform.detail.configurationInvalid')}
          </Tag>
          <pre>{JSON.stringify(detail.config.values, null, 2)}</pre>
          {!detail.config.valid && detail.config.validation_errors.length > 0 && (
            <Alert type='warning' content={detail.config.validation_errors.join('\n')} />
          )}
        </section>
        <section className={styles.section}>
          <h2>{t('pluginPlatform.detail.permissions')}</h2>
          <div className={styles.bindingList}>
            {detail.grants.map((grant) => <div key={grant.permission} className={styles.binding}>
              <code>{grant.permission}</code><Tag color={grant.granted ? 'green' : 'gray'}>{grant.granted ? t('pluginPlatform.detail.granted') : t('pluginPlatform.detail.denied')}</Tag>
            </div>)}
            {!detail.grants.length && <p className={styles.muted}>{t('pluginPlatform.detail.noPermissions')}</p>}
          </div>
        </section>
      </div>
      {desktopShell && <section className={styles.section}>
        <div className={styles.actions}>
          {!trashed && summary.previous && <Button icon={<Redo />} disabled={busy} onClick={() => {
            setRestoreMode('previous_code'); setAcknowledgeLoss(false); setDialog('restore');
          }}>{t('pluginPlatform.actions.restore')}</Button>}
          {trashed && <Button icon={<Redo />} disabled={busy} onClick={() => void mutate((current) => pluginPlatform.plugins.restore.invoke({
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
              : 'pluginPlatform.dialog.export_backup.disclosure')} />
            <Input value={destination} onChange={setDestination} placeholder={t('pluginPlatform.dialog.destination')} />
          </>}
          {dialog === 'restore' && <>
            <Checkbox checked={restoreMode === 'previous_code_and_data'} onChange={(checked) => {
              setRestoreMode(checked ? 'previous_code_and_data' : 'previous_code');
              setAcknowledgeLoss(false);
            }}>{t('pluginPlatform.dialog.restore.withData')}</Checkbox>
            {needsLossAck && <Alert type='warning' content={t('pluginPlatform.dialog.restore.dataLoss')} />}
            {needsLossAck && <Checkbox checked={acknowledgeLoss} onChange={setAcknowledgeLoss}>
              {t('pluginPlatform.dialog.restore.acknowledge')}
            </Checkbox>}
          </>}
          {dialog === 'trash' && <Alert type='warning' content={t('pluginPlatform.dialog.trash.body')} />}
          {dialog === 'delete' && <Alert type='error' content={t('pluginPlatform.dialog.delete.body')} />}
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
      {commandOutput && <section className={styles.section}>
        <h2>{t('pluginPlatform.command.result')}</h2>
        <pre>{commandOutput}</pre>
      </section>}
    </main>
  );
}
