/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  PluginProjectDetail,
  UpdatePluginDependenciesRequest,
} from '@/common/types/pluginPlatform';
import { Alert, Input, Modal } from '@arco-design/web-react';
import React, { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { PluginLoadFailure } from './pluginWorkbenchModel';
import styles from './PluginWorkbenchPage.module.css';

export interface PluginDependencyDialogProps {
  visible: boolean;
  detail: PluginProjectDetail | null;
  loading: boolean;
  failure?: PluginLoadFailure | null;
  onCancel: () => void;
  onSubmit: (request: UpdatePluginDependenciesRequest) => void | Promise<void>;
}

const PluginDependencyDialog: React.FC<PluginDependencyDialogProps> = ({
  visible,
  detail,
  loading,
  failure,
  onCancel,
  onSubmit,
}) => {
  const { t } = useTranslation();
  const [content, setContent] = useState('{}');
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!visible) return;
    setContent(JSON.stringify(detail?.direct_dependencies ?? {}, null, 2));
    setError(null);
  }, [visible, detail?.summary.project_id, detail?.direct_dependencies]);

  const handleSubmit = async () => {
    if (
      !detail?.summary.project_id ||
      !detail.source_snapshot_digest ||
      !detail.dependency_lock_digest
    ) {
      setError(t('pluginWorkbench.dialogs.dependencies.projectUnavailable'));
      return;
    }
    let parsed: unknown;
    try {
      parsed = JSON.parse(content);
    } catch {
      setError(t('pluginWorkbench.dialogs.dependencies.invalidJson'));
      return;
    }
    if (
      parsed == null ||
      Array.isArray(parsed) ||
      typeof parsed !== 'object' ||
      Object.values(parsed).some((value) => typeof value !== 'string')
    ) {
      setError(t('pluginWorkbench.dialogs.dependencies.invalidShape'));
      return;
    }
    setError(null);
    await onSubmit({
      project_id: detail.summary.project_id,
      expected_project_revision: detail.summary.project_revision,
      expected_build_generation: detail.summary.build_generation,
      expected_source_snapshot_digest: detail.source_snapshot_digest,
      expected_dependency_lock_digest: detail.dependency_lock_digest,
      dependencies: parsed as Record<string, string>,
    });
  };

  return (
    <Modal
      visible={visible}
      title={t('pluginWorkbench.dialogs.dependencies.title')}
      onCancel={loading ? undefined : onCancel}
      onOk={() => void handleSubmit()}
      okText={t('pluginWorkbench.dialogs.dependencies.submit')}
      cancelText={t('pluginWorkbench.actions.cancel')}
      confirmLoading={loading}
      maskClosable={false}
      autoFocus={false}
      unmountOnExit
    >
      <div className={styles.dialogIntro}>
        {t('pluginWorkbench.dialogs.dependencies.body')}
      </div>
      {failure && <Alert type='error' showIcon content={failure.message} />}
      <div className={styles.dialogForm}>
        <div className={styles.dialogFieldLabel}>
          {t('pluginWorkbench.dialogs.dependencies.directDependencies')}
        </div>
        <Input.TextArea
          value={content}
          onChange={setContent}
          onInput={(event) => setContent(event.currentTarget.value)}
          autoSize={{ minRows: 8, maxRows: 20 }}
          placeholder={'{\n  "package-name": "^1.0.0"\n}'}
          spellCheck={false}
          aria-label={t('pluginWorkbench.dialogs.dependencies.directDependencies')}
        />
        <div className={styles.dialogHint}>
          {t('pluginWorkbench.dialogs.dependencies.hint')}
        </div>
        {error && <Alert type='error' showIcon content={error} />}
      </div>
    </Modal>
  );
};

export default PluginDependencyDialog;
