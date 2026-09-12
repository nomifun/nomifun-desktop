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
  const [sourcePath, setSourcePath] = useState('');
  const [inspection, setInspection] = useState<PluginImportInspection | null>(null);
  const [picking, setPicking] = useState(false);
  const [error, setError] = useState('');

  useEffect(() => {
    if (!visible) return;
    setSourcePath('');
    setInspection(null);
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
              filters: [{ name: t('pluginWorkbench.dialogs.import.archiveFilter'), extensions: ['zip'] }],
            }
      );
      const selected = paths?.[0]?.trim();
      if (!selected) return;
      setSourcePath(selected);
      const next = await ipcBridge.plugins.inspectImport.invoke({ source_path: selected });
      setInspection(next);
    } catch (caught) {
      console.error('[plugins] import inspection failed', caught);
      setError(t('pluginWorkbench.product.importInvalid'));
    } finally {
      setPicking(false);
    }
  };

  const submit = async () => {
    if (!inspection || !sourcePath) return;
    await onSubmit({
      expected_library_revision: libraryRevision,
      import_kind: inspection.import_kind,
      source_path: sourcePath,
      expected_bundle_or_artifact_digest: inspection.expected_digest,
    });
  };

  return (
    <NomiModal
      visible={visible}
      header={t('pluginWorkbench.product.importTitle')}
      footer={null}
      size='medium'
      onCancel={loading ? undefined : onCancel}
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
        {!inspection && !picking && !error && (
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
