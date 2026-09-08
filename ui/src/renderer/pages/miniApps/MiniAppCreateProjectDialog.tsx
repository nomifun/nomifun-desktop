/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type {
  MiniAppWorkshop,
} from '@/common/types/miniAppPlatform';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import {
  Form,
  Input,
  Modal,
  type ModalProps,
} from '@arco-design/web-react';
import React, { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import styles from './MiniAppWorkbench.module.css';

interface MiniAppCreateProjectDialogProps {
  visible: boolean;
  libraryRevision: number;
  onCancel: () => void;
  onCreated: (workshop: MiniAppWorkshop) => void;
}

const CreateProjectModal =
  Modal as unknown as React.ComponentType<
    React.PropsWithChildren<ModalProps>
  >;

function errorMessage(error: unknown): string {
  if (isBackendHttpError(error)) {
    const detail = error.backendMessage || error.message;
    return error.code ? `${error.code}: ${detail}` : detail;
  }
  return error instanceof Error ? error.message : String(error);
}

const MiniAppCreateProjectDialog: React.FC<MiniAppCreateProjectDialogProps> = ({
  visible,
  libraryRevision,
  onCancel,
  onCreated,
}) => {
  const { t } = useTranslation();
  const [displayName, setDisplayName] = useState('');
  const [description, setDescription] = useState('');
  const [validationError, setValidationError] = useState('');
  const [requestError, setRequestError] = useState('');
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    if (!visible) return;
    setDisplayName('');
    setDescription('');
    setValidationError('');
    setRequestError('');
    setSubmitting(false);
  }, [visible]);

  const submit = async () => {
    const name = displayName.trim();
    if (!name) {
      setValidationError(t('miniApps.create.displayNameRequired'));
      return;
    }

    setValidationError('');
    setRequestError('');
    setSubmitting(true);
    try {
      const workshop = await ipcBridge.miniapps.createProject.invoke({
        expected_library_revision: libraryRevision,
        display_name: name,
        ...(description.trim() ? { description: description.trim() } : {}),
        kind: 'ui_only',
      });
      onCreated(workshop);
    } catch (error) {
      console.error('[miniapps] failed to create project', error);
      setRequestError(
        t('miniApps.create.error', { message: errorMessage(error) })
      );
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <CreateProjectModal
      className={styles.modal}
      title={t('miniApps.create.title')}
      visible={visible}
      onCancel={onCancel}
      onOk={() => void submit()}
      okText={t('miniApps.create.submit')}
      cancelText={t('miniApps.create.cancel')}
      confirmLoading={submitting}
      unmountOnExit
    >
      <p className={styles.dialogIntro}>{t('miniApps.create.intro')}</p>
      <Form layout='vertical'>
        <Form.Item label={t('miniApps.create.displayName')}>
          <Input
            autoFocus
            value={displayName}
            maxLength={120}
            placeholder={t('miniApps.create.displayNamePlaceholder')}
            onChange={setDisplayName}
            onPressEnter={() => void submit()}
          />
          {validationError && (
            <div className={styles.noticeError}>{validationError}</div>
          )}
        </Form.Item>
        <Form.Item label={t('miniApps.create.description')}>
          <Input.TextArea
            value={description}
            maxLength={500}
            autoSize={{ minRows: 3, maxRows: 6 }}
            placeholder={t('miniApps.create.descriptionPlaceholder')}
            onChange={setDescription}
          />
        </Form.Item>
        <div className={styles.notice}>
          {t('miniApps.create.uiOnlyHint')}
        </div>
      </Form>
      {requestError && (
        <div className={`${styles.notice} ${styles.noticeError}`}>
          {requestError}
        </div>
      )}
    </CreateProjectModal>
  );
};

export default MiniAppCreateProjectDialog;
