import { ipcBridge } from '@/common';
import type {
  ImportPluginRequest,
  PluginImportInspection,
} from '@/common/types/pluginPlatform';
import { Alert, Button, Spin, Tag } from '@arco-design/web-react';
import { FileZip, FolderClose, Link, Upload } from '@icon-park/react';
import React, { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import NomiModal from '@/renderer/components/base/NomiModal';
import type { PluginLoadFailure } from './pluginWorkbenchModel';
import styles from './PluginProductSurface.module.css';
import { pluginRuntimeProduct, type PluginRuntimeDraft } from '@/common/adapter/pluginRuntimeProductBridge';
import { useNavigate } from 'react-router-dom';

interface PluginSmartImportDialogProps {
  visible: boolean;
  libraryRevision: number;
  loading: boolean;
  failure?: PluginLoadFailure | null;
  onCancel: () => void;
  onSubmit: (request: ImportPluginRequest) => void | Promise<void>;
}

const PluginSmartImportDialog: React.FC<PluginSmartImportDialogProps> = ({
  visible,
  libraryRevision,
  loading,
  failure,
  onCancel,
  onSubmit,
}) => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [runtimeDraft, setRuntimeDraft] = useState<PluginRuntimeDraft | null>(null);
  const [sourcePath, setSourcePath] = useState('');
  const [inspection, setInspection] = useState<PluginImportInspection | null>(null);
  const [picking, setPicking] = useState(false);
  const [error, setError] = useState('');

  useEffect(() => {
    if (!visible) return;
    setSourcePath('');
    setInspection(null);
    setRuntimeDraft(null);
    setPicking(false);
    setError('');
  }, [visible]);

  const pick = async (kind: 'directory' | 'zip') => {
    setPicking(true);
    setError('');
    setInspection(null);
    try {
      const paths = await ipcBridge.dialog.showOpen.invoke(
        kind === 'directory'
          ? { properties: ['openDirectory'] }
          : {
              properties: ['openFile'],
              filters: [{ name: t('pluginWorkbench.dialogs.import.archiveFilter'), extensions: ['zip', 'nomiplugin', 'html', 'htm'] }],
            }
      );
      const selected = paths?.[0]?.trim();
      if (!selected) return;
      setSourcePath(selected);
      if (runtimeDraft) {
        await pluginRuntimeProduct.discard.invoke({ id: runtimeDraft.id, expected_revision: runtimeDraft.revision });
        setRuntimeDraft(null);
      }
      try {
        const next = await ipcBridge.plugins.inspectImport.invoke({ source_path: selected });
        setInspection(next);
      } catch {
        setRuntimeDraft(await pluginRuntimeProduct.inspect.invoke({ source_path: selected }));
      }
    } catch (caught) {
      console.error('[plugins] import inspection failed', caught);
      setError(t('pluginWorkbench.product.importInvalid'));
    } finally {
      setPicking(false);
    }
  };

  const submit = async () => {
    if (runtimeDraft) {
      setPicking(true);
      setError('');
      try {
        const app = await pluginRuntimeProduct.save.invoke({ id: runtimeDraft.id, expected_revision: runtimeDraft.revision });
        onCancel();
        navigate(`/plugins/run/${app.plugin.plugin_id}`);
      } catch {
        setError(t('pluginRuntime.product.importFailed'));
        try { setRuntimeDraft(await pluginRuntimeProduct.draft.invoke({ id: runtimeDraft.id })); } catch { /* Keep the recoverable draft. */ }
      } finally { setPicking(false); }
      return;
    }
    if (!inspection || !sourcePath) return;
    await onSubmit({
      expected_library_revision: libraryRevision,
      import_kind: inspection.import_kind,
      source_path: sourcePath,
      expected_bundle_or_artifact_digest: inspection.expected_digest,
    });
  };

  const cancel = async () => {
    if (loading || picking) return;
    if (runtimeDraft) {
      setPicking(true);
      try { await pluginRuntimeProduct.discard.invoke({ id: runtimeDraft.id, expected_revision: runtimeDraft.revision }); }
      catch { setError(t('pluginRuntime.product.operationFailed')); return; }
      finally { setPicking(false); }
    }
    onCancel();
  };

  return (
    <NomiModal
      visible={visible}
      header={t('pluginWorkbench.product.importTitle')}
      footer={null}
      size='medium'
      onCancel={loading || picking ? undefined : () => void cancel()}
      maskClosable={false}
      autoFocus={false}
      unmountOnExit
    >
      <div className={styles.importDialog}>
        <div className={styles.importIntro}>
          <span><Upload theme='outline' size={24} /></span>
          <div>
            <h2>{t('pluginWorkbench.product.importHeading')}</h2>
            <p>{t('pluginWorkbench.product.importBody')}</p>
          </div>
        </div>
        <div className={styles.importChoices}>
          <Button icon={<FolderClose size={16} />} disabled={picking || loading} onClick={() => void pick('directory')}>
            {t('pluginWorkbench.product.chooseFolder')}
          </Button>
          <Button icon={<FileZip size={16} />} disabled={picking || loading} onClick={() => void pick('zip')}>
            {t('pluginWorkbench.product.chooseZip')}
          </Button>
        </div>
        {picking && <div className={styles.importAnalyzing}><Spin /><span>{t('pluginWorkbench.product.importAnalyzing')}</span></div>}
        {(error || failure) && <Alert type='error' showIcon content={error || failure?.message} />}
        {inspection && (
          <section className={styles.importInspection}>
            <div className={styles.importPluginHeader}>
              <span className={styles.pluginIcon}><Link theme='outline' size={18} /></span>
              <div>
                <strong>{inspection.display_name}</strong>
                <small>{inspection.package_version} · {inspection.editable_source ? t('pluginWorkbench.product.editableImport') : t('pluginWorkbench.product.runtimeImport')}</small>
              </div>
              <Tag color='green'>{t('pluginWorkbench.product.importVerified')}</Tag>
            </div>
            <p>{inspection.description}</p>
            <div className={styles.importFacts}>
              <span>{t('pluginWorkbench.product.importCapabilities', { count: inspection.capability_count })}</span>
              <span>{t('pluginWorkbench.product.importNoAutoRun')}</span>
              <span>{t('pluginWorkbench.product.importNoCredentials')}</span>
            </div>
            <Button type='primary' long loading={loading} onClick={() => void submit()}>
              {t('pluginWorkbench.product.installAndEnable')}
            </Button>
            <details className={styles.creatorAdvanced}>
              <summary>{t('common.technical_details')}</summary>
              <code>{inspection.package_id}</code>
              <code>{inspection.expected_digest.slice(0, 16)}…</code>
            </details>
          </section>
        )}
        {runtimeDraft && (
          <section className={styles.importInspection}>
            <h3>{runtimeDraft.name}</h3>
            <p>{runtimeDraft.description}</p>
            <p>{t(runtimeDraft.import?.includes_data ? 'pluginRuntime.product.backup' : 'pluginWorkbench.product.importNoCredentials')}</p>
            <Button type='primary' long loading={picking} onClick={() => void submit()}>{t('pluginWorkbench.product.installAndEnable')}</Button>
          </section>
        )}
        {!inspection && !runtimeDraft && !picking && !error && (
          <div className={styles.importDropHint}>
            <Upload theme='outline' size={28} />
            <strong>{t('pluginWorkbench.product.importDropTitle')}</strong>
            <span>{t('pluginWorkbench.product.importDropBody')}</span>
          </div>
        )}
      </div>
    </NomiModal>
  );
};

export default PluginSmartImportDialog;
