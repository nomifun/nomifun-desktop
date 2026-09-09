/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  ApplyPluginSourceEditRequest,
  PluginProjectDetail,
  PluginSourceFileEdit,
} from '@/common/types/pluginPlatform';
import { Alert, Input, Modal, Radio } from '@arco-design/web-react';
import React, { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { PluginLoadFailure } from './pluginWorkbenchModel';
import styles from './PluginWorkbenchPage.module.css';

export interface PluginSourceEditDialogProps {
  visible: boolean;
  detail: PluginProjectDetail | null;
  loading: boolean;
  failure?: PluginLoadFailure | null;
  onCancel: () => void;
  onSubmit: (request: ApplyPluginSourceEditRequest) => void | Promise<void>;
}

const PluginSourceEditDialog: React.FC<PluginSourceEditDialogProps> = ({
  visible,
  detail,
  loading,
  failure,
  onCancel,
  onSubmit,
}) => {
  const { t } = useTranslation();
  const [kind, setKind] = useState<PluginSourceFileEdit['kind']>('replace');
  const [path, setPath] = useState('');
  const [content, setContent] = useState('');
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!visible) return;
    setKind('replace');
    setPath('');
    setContent('');
    setError(null);
  }, [visible, detail?.summary.project_id]);

  const handleSubmit = async () => {
    const normalizedPath = path.trim().replaceAll('\\', '/');
    if (!normalizedPath || normalizedPath === '.' || normalizedPath.includes('..')) {
      setError(t('pluginWorkbench.dialogs.sourceEdit.pathRequired'));
      return;
    }
    if (kind === 'replace' && !content.trim()) {
      setError(t('pluginWorkbench.dialogs.sourceEdit.contentRequired'));
      return;
    }
    if (!detail?.summary.project_id || !detail.source_snapshot_digest) {
      setError(t('pluginWorkbench.dialogs.sourceEdit.projectUnavailable'));
      return;
    }
    setError(null);
    await onSubmit({
      project_id: detail.summary.project_id,
      expected_source_snapshot_digest: detail.source_snapshot_digest,
      edit:
        kind === 'replace'
          ? { kind: 'replace', path: normalizedPath, content }
          : { kind: 'delete', path: normalizedPath },
    });
  };

  return (
    <Modal
      visible={visible}
      title={t('pluginWorkbench.dialogs.sourceEdit.title')}
      onCancel={loading ? undefined : onCancel}
      onOk={() => void handleSubmit()}
      okText={t('pluginWorkbench.dialogs.sourceEdit.submit')}
      cancelText={t('pluginWorkbench.actions.cancel')}
      confirmLoading={loading}
      maskClosable={false}
      autoFocus={false}
      unmountOnExit
    >
      <div className={styles.dialogIntro}>
        {t('pluginWorkbench.dialogs.sourceEdit.body')}
      </div>
      {failure && <Alert type='error' showIcon content={failure.message} />}
      <div className={styles.dialogForm}>
        <div className={styles.dialogFieldLabel}>
          {t('pluginWorkbench.dialogs.sourceEdit.operation')}
        </div>
        <Radio.Group value={kind} onChange={setKind}>
          <Radio value='replace'>
            {t('pluginWorkbench.dialogs.sourceEdit.replace')}
          </Radio>
          <Radio value='delete'>
            {t('pluginWorkbench.dialogs.sourceEdit.delete')}
          </Radio>
        </Radio.Group>
        <div className={styles.dialogFieldLabel}>
          {t('pluginWorkbench.dialogs.sourceEdit.path')}
        </div>
        <Input
          value={path}
          onChange={setPath}
          placeholder={t('pluginWorkbench.dialogs.sourceEdit.pathPlaceholder')}
          spellCheck={false}
          aria-label={t('pluginWorkbench.dialogs.sourceEdit.path')}
        />
        {kind === 'replace' && (
          <>
            <div className={styles.dialogFieldLabel}>
              {t('pluginWorkbench.dialogs.sourceEdit.content')}
            </div>
            <Input.TextArea
              value={content}
              onChange={setContent}
              autoSize={{ minRows: 10, maxRows: 24 }}
              placeholder={t('pluginWorkbench.dialogs.sourceEdit.contentPlaceholder')}
              spellCheck={false}
              aria-label={t('pluginWorkbench.dialogs.sourceEdit.content')}
            />
          </>
        )}
        <div className={styles.dialogHint}>
          {t('pluginWorkbench.dialogs.sourceEdit.hint')}
        </div>
        {error && <Alert type='error' showIcon content={error} />}
      </div>
    </Modal>
  );
};

export default PluginSourceEditDialog;
