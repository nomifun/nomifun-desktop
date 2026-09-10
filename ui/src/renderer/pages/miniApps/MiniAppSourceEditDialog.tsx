/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import type {
  MiniAppSourceFile,
  MiniAppWorkshop,
} from '@/common/types/miniAppPlatform';
import { Alert, Button, Input, Modal } from '@arco-design/web-react';
import React, { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import styles from './MiniAppWorkbench.module.css';

export interface MiniAppSourceEditDialogProps {
  visible: boolean;
  workshop: MiniAppWorkshop | null;
  onCancel: () => void;
  onSaved: (workshop: MiniAppWorkshop) => void;
}

const formatError = (error: unknown): string => {
  if (isBackendHttpError(error)) {
    return error.backendMessage || error.message;
  }
  return error instanceof Error ? error.message : String(error);
};

const normalizePath = (path: string): string =>
  path.trim().replaceAll('\\', '/');

const isSafeRelativePath = (path: string): boolean =>
  Boolean(path) &&
  !path.startsWith('/') &&
  !path.endsWith('/') &&
  path.split('/').every((segment) => segment !== '' && segment !== '.' && segment !== '..');

const MiniAppSourceEditDialog: React.FC<MiniAppSourceEditDialogProps> = ({
  visible,
  workshop,
  onCancel,
  onSaved,
}) => {
  const { t } = useTranslation();
  const [path, setPath] = useState('ui/index.html');
  const [source, setSource] = useState<MiniAppSourceFile | null>(null);
  const [content, setContent] = useState('');
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadSource = async (requestedPath: string) => {
    if (!workshop) return;
    const normalizedPath = normalizePath(requestedPath);
    if (!isSafeRelativePath(normalizedPath)) {
      setError(t('miniApps.sourceEdit.pathInvalid'));
      return;
    }
    setLoading(true);
    setError(null);
    setSource(null);
    try {
      const loaded = await ipcBridge.miniapps.getSourceFile.invoke({
        miniapp_id: workshop.miniapp.miniapp_id,
        path: normalizedPath,
      });
      if (
        loaded.project_id !== workshop.project_id ||
        loaded.miniapp_id !== workshop.miniapp.miniapp_id
      ) {
        throw new Error(t('miniApps.sourceEdit.identityMismatch'));
      }
      setPath(loaded.path);
      setSource(loaded);
      setContent(loaded.content);
    } catch (loadError) {
      setError(formatError(loadError));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    if (!visible || !workshop) return;
    const defaultPath = 'ui/index.html';
    setPath(defaultPath);
    setSource(null);
    setContent('');
    setSaving(false);
    setError(null);
    void loadSource(defaultPath);
    // The visible Project head is the dialog's complete editing authority.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    visible,
    workshop?.miniapp.miniapp_id,
    workshop?.project_id,
    workshop?.project_revision,
    workshop?.source_snapshot_digest,
  ]);

  const handlePathChange = (next: string) => {
    setPath(next);
    setSource(null);
    setContent('');
    setError(null);
  };

  const handleSave = async () => {
    if (!workshop || !source || normalizePath(path) !== source.path) {
      setError(t('miniApps.sourceEdit.loadRequired'));
      return;
    }
    if (!content.length) {
      setError(t('miniApps.sourceEdit.contentRequired'));
      return;
    }
    setSaving(true);
    setError(null);
    try {
      const updated = await ipcBridge.miniapps.replaceSourceFile.invoke({
        miniapp_id: workshop.miniapp.miniapp_id,
        expected_product_revision: workshop.miniapp.product_revision,
        project_id: workshop.project_id,
        expected_project_revision: workshop.project_revision,
        expected_build_generation: source.build_generation,
        expected_source_snapshot_digest: source.source_snapshot_digest,
        path: source.path,
        content,
      });
      onSaved(updated);
    } catch (saveError) {
      setError(formatError(saveError));
    } finally {
      setSaving(false);
    }
  };

  const blocked = loading || saving;
  return (
    <Modal
      visible={visible}
      title={t('miniApps.sourceEdit.title')}
      onCancel={blocked ? undefined : onCancel}
      onOk={() => void handleSave()}
      okText={t('miniApps.sourceEdit.save')}
      cancelText={t('miniApps.actions.cancel')}
      confirmLoading={saving}
      okButtonProps={{ disabled: loading || !source }}
      maskClosable={false}
      autoFocus={false}
      unmountOnExit
      className={styles.modal}
    >
      <div className={styles.dialogIntro}>
        {t('miniApps.sourceEdit.intro')}
      </div>
      <div className={styles.dialogForm}>
        <div className={styles.dialogFieldLabel}>
          {t('miniApps.sourceEdit.path')}
        </div>
        <div className={styles.dialogPickerRow}>
          <Input
            value={path}
            onChange={handlePathChange}
            aria-label={t('miniApps.sourceEdit.path')}
            placeholder='ui/index.html'
            spellCheck={false}
            disabled={blocked}
            className={styles.monoInput}
          />
          <Button
            onClick={() => void loadSource(path)}
            loading={loading}
            disabled={saving}
          >
            {t('miniApps.sourceEdit.load')}
          </Button>
        </div>
        <div className={styles.dialogHint}>
          {workshop?.miniapp.kind === 'service'
            ? t('miniApps.sourceEdit.serviceHint')
            : t('miniApps.sourceEdit.uiHint')}
        </div>
        <div className={styles.dialogFieldLabel}>
          {t('miniApps.sourceEdit.content')}
        </div>
        <Input.TextArea
          value={content}
          onChange={setContent}
          aria-label={t('miniApps.sourceEdit.content')}
          autoSize={{ minRows: 12, maxRows: 26 }}
          spellCheck={false}
          disabled={blocked || !source}
          className={styles.monoInput}
        />
        {source && (
          <div className={styles.dialogHint}>
            {t('miniApps.sourceEdit.fence', {
              generation: source.build_generation,
              digest: source.source_snapshot_digest.slice(0, 12),
            })}
          </div>
        )}
        {error && <Alert type='error' showIcon content={error} />}
      </div>
    </Modal>
  );
};

export default MiniAppSourceEditDialog;
