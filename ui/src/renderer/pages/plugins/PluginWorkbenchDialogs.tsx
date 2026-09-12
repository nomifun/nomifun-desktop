/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type {
  CreatePluginProjectRequest,
  ImportPluginRequest,
  PluginProjectDetail,
  PluginSummary,
} from '@/common/types/pluginPlatform';
import { Alert, Button, Checkbox, Form, Input, Radio, Select } from '@arco-design/web-react';
import { Code, FolderClose, Upload } from '@icon-park/react';
import React, { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import NomiModal from '@/renderer/components/base/NomiModal';
import {
  EMPTY_TEST_INPUT_DIGEST,
  isPluginDigest,
  type PluginApplyTargetSelection,
  type PluginLoadFailure,
} from './pluginWorkbenchModel';
import styles from './PluginWorkbenchPage.module.css';

type CreateProjectFields = Omit<
  CreatePluginProjectRequest,
  'expected_library_revision' | 'linked_mount_id' | 'expected_linked_mount_revision' | 'expected_linked_target_digest'
>;

export interface PluginProjectCreateModalProps {
  visible: boolean;
  libraryRevision: number;
  loading: boolean;
  failure?: PluginLoadFailure | null;
  onCancel: () => void;
  onSubmit: (request: CreateProjectFields & { expected_library_revision: number }) => void | Promise<void>;
}

export const PluginProjectCreateModal: React.FC<PluginProjectCreateModalProps> = ({
  visible,
  libraryRevision,
  loading,
  failure,
  onCancel,
  onSubmit,
}) => {
  const { t } = useTranslation();
  const [form] = Form.useForm<CreateProjectFields>();

  useEffect(() => {
    if (!visible) return;
    form.resetFields();
    form.setFieldsValue({
      package_id: '',
      package_version: '0.1.0',
      display_name: '',
      description: '',
      language: 'type_script',
    });
  }, [form, visible]);

  const handleSubmit = async () => {
    try {
      const values = await form.validate();
      await onSubmit({
        ...values,
        package_id: values.package_id.trim(),
        package_version: values.package_version.trim(),
        display_name: values.display_name.trim(),
        description: values.description.trim(),
        expected_library_revision: libraryRevision,
      });
    } catch {
      // Validation errors are rendered by the Form; the parent owns request errors.
    }
  };

  return (
    <NomiModal
      visible={visible}
      header={t('pluginWorkbench.dialogs.create.title')}
      onCancel={loading ? undefined : onCancel}
      onOk={() => void handleSubmit()}
      okText={t('pluginWorkbench.actions.createProject')}
      cancelText={t('pluginWorkbench.actions.cancel')}
      confirmLoading={loading}
      maskClosable={false}
      autoFocus={false}
      unmountOnExit
    >
      <div className={styles.dialogIntro}>{t('pluginWorkbench.dialogs.create.body')}</div>
      {failure && <Alert type='error' showIcon content={failure.message} />}
      <Form form={form} layout='vertical' className={styles.dialogForm}>
        <Form.Item
          field='display_name'
          label={t('pluginWorkbench.dialogs.create.displayName')}
          rules={[{ required: true, message: t('pluginWorkbench.dialogs.create.displayNameRequired') }]}
        >
          <Input maxLength={100} showWordLimit />
        </Form.Item>
        <div className={styles.dialogFieldGrid}>
          <Form.Item
            field='package_id'
            label={t('pluginWorkbench.dialogs.create.packageId')}
            rules={[{ required: true, message: t('pluginWorkbench.dialogs.create.packageIdRequired') }]}
          >
            <Input placeholder='dev.example.plugin' />
          </Form.Item>
          <Form.Item
            field='package_version'
            label={t('pluginWorkbench.dialogs.create.packageVersion')}
            rules={[{ required: true, message: t('pluginWorkbench.dialogs.create.packageVersionRequired') }]}
          >
            <Input placeholder='0.1.0' />
          </Form.Item>
        </div>
        <Form.Item field='language' label={t('pluginWorkbench.dialogs.create.language')}>
          <Select>
            <Select.Option value='type_script'>
              {t('pluginWorkbench.dialogs.create.typeScript')}
            </Select.Option>
            <Select.Option value='java_script'>
              {t('pluginWorkbench.dialogs.create.javaScript')}
            </Select.Option>
          </Select>
        </Form.Item>
        <Form.Item field='description' label={t('pluginWorkbench.dialogs.create.description')}>
          <Input.TextArea autoSize={{ minRows: 3, maxRows: 6 }} maxLength={300} showWordLimit />
        </Form.Item>
      </Form>
    </NomiModal>
  );
};

type ImportSourceKind = 'directory' | 'archive' | 'share_bundle';

export interface PluginPrebuiltImportModalProps {
  visible: boolean;
  libraryRevision: number;
  loading: boolean;
  failure?: PluginLoadFailure | null;
  onCancel: () => void;
  onSubmit: (request: ImportPluginRequest) => void | Promise<void>;
}

export const PluginPrebuiltImportModal: React.FC<PluginPrebuiltImportModalProps> = ({
  visible,
  libraryRevision,
  loading,
  failure,
  onCancel,
  onSubmit,
}) => {
  const { t } = useTranslation();
  const [sourceKind, setSourceKind] = useState<ImportSourceKind>('directory');
  const [sourcePath, setSourcePath] = useState('');
  const [digest, setDigest] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [picking, setPicking] = useState(false);

  useEffect(() => {
    if (!visible) return;
    setSourceKind('directory');
    setSourcePath('');
    setDigest('');
    setError(null);
    setPicking(false);
  }, [visible]);

  const pickSource = async () => {
    setPicking(true);
    setError(null);
    try {
      const result = await ipcBridge.dialog.showOpen.invoke(
        sourceKind !== 'archive'
          ? { properties: ['openDirectory'] }
          : {
              properties: ['openFile'],
              filters: [{ name: t('pluginWorkbench.dialogs.import.archiveFilter'), extensions: ['zip'] }],
            }
      );
      if (result?.[0]) setSourcePath(result[0]);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    } finally {
      setPicking(false);
    }
  };

  const handleSubmit = async () => {
    const normalizedPath = sourcePath.trim();
    const normalizedDigest = digest.trim().toLowerCase();
    if (!normalizedPath) {
      setError(t('pluginWorkbench.dialogs.import.sourceRequired'));
      return;
    }
    if (!isPluginDigest(normalizedDigest)) {
      setError(t('pluginWorkbench.dialogs.import.digestInvalid'));
      return;
    }
    setError(null);
    await onSubmit({
      expected_library_revision: libraryRevision,
      import_kind:
        sourceKind === 'share_bundle' ? 'share_bundle' : 'prebuilt_artifact',
      source_path: normalizedPath,
      expected_bundle_or_artifact_digest: normalizedDigest,
    });
  };

  return (
    <NomiModal
      visible={visible}
      header={t('pluginWorkbench.dialogs.import.title')}
      onCancel={loading ? undefined : onCancel}
      onOk={() => void handleSubmit()}
      okText={t('pluginWorkbench.actions.importArtifact')}
      cancelText={t('pluginWorkbench.actions.cancel')}
      confirmLoading={loading}
      maskClosable={false}
      autoFocus={false}
      unmountOnExit
    >
      <div className={styles.dialogIntro}>{t('pluginWorkbench.dialogs.import.body')}</div>
      <div className={styles.dialogForm}>
        {failure && <Alert type='error' showIcon content={failure.message} />}
        <div className={styles.dialogFieldLabel}>{t('pluginWorkbench.dialogs.import.sourceType')}</div>
        <Radio.Group
          type='button'
          size='small'
          value={sourceKind}
          onChange={(value: string) => {
            setSourceKind(value as ImportSourceKind);
            setSourcePath('');
            setError(null);
          }}
        >
          <Radio value='directory'>
            <FolderClose theme='outline' size='14' /> {t('pluginWorkbench.dialogs.import.directory')}
          </Radio>
          <Radio value='archive'>
            <Upload theme='outline' size='14' /> {t('pluginWorkbench.dialogs.import.archive')}
          </Radio>
          <Radio value='share_bundle'>
            <Upload theme='outline' size='14' /> {t('pluginWorkbench.dialogs.import.shareBundle')}
          </Radio>
        </Radio.Group>

        <div className={styles.dialogFieldLabel}>{t('pluginWorkbench.dialogs.import.sourcePath')}</div>
        <div className={styles.dialogPickerRow}>
          <Input
            value={sourcePath}
            readOnly
            placeholder={t('pluginWorkbench.dialogs.import.sourcePlaceholder')}
            title={sourcePath}
          />
          <Button
            icon={<FolderClose theme='outline' size='14' />}
            loading={picking}
            onClick={() => void pickSource()}
          >
            {t('pluginWorkbench.dialogs.import.chooseSource')}
          </Button>
        </div>

        <div className={styles.dialogFieldLabel}>{t('pluginWorkbench.dialogs.import.digest')}</div>
        <Input
          value={digest}
          onChange={setDigest}
          placeholder={t('pluginWorkbench.dialogs.import.digestPlaceholder')}
          className={styles.monoInput}
          spellCheck={false}
        />
        <div className={styles.dialogHint}>{t('pluginWorkbench.dialogs.import.digestHint')}</div>
        {error && <Alert type='error' showIcon content={error} />}
      </div>
    </NomiModal>
  );
};

export interface PluginCandidateTestModalProps {
  visible: boolean;
  detail: PluginProjectDetail | null;
  loading: boolean;
  failure?: PluginLoadFailure | null;
  onCancel: () => void;
  onSubmit: (resolvedTestInputDigest: string) => void | Promise<void>;
}

export const PluginCandidateTestModal: React.FC<PluginCandidateTestModalProps> = ({
  visible,
  loading,
  failure,
  onCancel,
  onSubmit,
}) => {
  const { t } = useTranslation();
  const [acknowledged, setAcknowledged] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!visible) return;
    setAcknowledged(false);
    setError(null);
  }, [visible]);

  const handleSubmit = async () => {
    if (!acknowledged) {
      setError(t('pluginWorkbench.dialogs.test.acknowledgementRequired'));
      return;
    }
    setError(null);
    await onSubmit(EMPTY_TEST_INPUT_DIGEST);
  };

  return (
    <NomiModal
      visible={visible}
      header={t('pluginWorkbench.dialogs.test.title')}
      onCancel={loading ? undefined : onCancel}
      onOk={() => void handleSubmit()}
      okText={t('pluginWorkbench.actions.testCandidate')}
      cancelText={t('pluginWorkbench.actions.cancel')}
      confirmLoading={loading}
      maskClosable={false}
      autoFocus={false}
      unmountOnExit
    >
      <div className={styles.dialogIntro}>{t('pluginWorkbench.dialogs.test.body')}</div>
      <div className={styles.dialogForm}>
        {failure && <Alert type='error' showIcon content={failure.message} />}
        <Alert type='warning' showIcon content={t('pluginWorkbench.dialogs.test.sideEffectWarning')} />
        <Checkbox checked={acknowledged} onChange={setAcknowledged}>
          {t('pluginWorkbench.dialogs.test.acknowledgeRisk')}
        </Checkbox>
        {error && <Alert type='error' showIcon content={error} />}
      </div>
    </NomiModal>
  );
};

export interface PluginCandidateApplyModalProps {
  visible: boolean;
  detail: PluginProjectDetail | null;
  linkedMount?: PluginSummary;
  loading: boolean;
  failure?: PluginLoadFailure | null;
  onCancel: () => void;
  onSubmit: (input: {
    target: PluginApplyTargetSelection;
    allowBreaking: boolean;
    acknowledgeTestWarning: boolean;
  }) => void | Promise<void>;
}

export const PluginCandidateApplyModal: React.FC<PluginCandidateApplyModalProps> = ({
  visible,
  detail,
  linkedMount,
  loading,
  failure,
  onCancel,
  onSubmit,
}) => {
  const { t } = useTranslation();
  const ready = detail?.ready;
  const hasLinkedMount = Boolean(detail?.summary.linked_mount_id);
  const canUseExistingMount = Boolean(hasLinkedMount && linkedMount?.current);
  const target: PluginApplyTargetSelection = hasLinkedMount ? 'existing_mount' : 'initial_install';
  const [allowBreaking, setAllowBreaking] = useState(false);
  const [acknowledgeTestWarning, setAcknowledgeTestWarning] = useState(false);

  useEffect(() => {
    if (!visible) return;
    setAllowBreaking(false);
    setAcknowledgeTestWarning(false);
  }, [visible, detail]);

  const needsBreakingAcknowledgement = ready?.impact.compatibility === 'breaking';
  const needsTestAcknowledgement = ready?.test.status !== 'passed';
  const blockedByTarget = target === 'existing_mount' && !canUseExistingMount;
  const canSubmit =
    Boolean(ready?.impact.can_apply) &&
    !blockedByTarget &&
    (!needsBreakingAcknowledgement || allowBreaking) &&
    (!needsTestAcknowledgement || acknowledgeTestWarning);

  const targetLabel = useMemo(() => {
    if (target === 'existing_mount') {
      return linkedMount?.display_name
        ? t('pluginWorkbench.dialogs.apply.existingMountNamed', {
            name: linkedMount.display_name,
          })
        : t('pluginWorkbench.dialogs.apply.existingMount');
    }
    return t('pluginWorkbench.dialogs.apply.initialInstall');
  }, [linkedMount?.display_name, t, target]);

  return (
    <NomiModal
      visible={visible}
      header={t('pluginWorkbench.dialogs.apply.title')}
      onCancel={loading ? undefined : onCancel}
      onOk={() =>
        void onSubmit({
          target,
          allowBreaking,
          acknowledgeTestWarning,
        })
      }
      okText={t('pluginWorkbench.product.saveAndEnable')}
      cancelText={t('pluginWorkbench.actions.cancel')}
      confirmLoading={loading}
      okButtonProps={{ disabled: !canSubmit }}
      maskClosable={false}
      autoFocus={false}
      unmountOnExit
    >
      <div className={styles.dialogIntro}>{t('pluginWorkbench.dialogs.apply.body')}</div>
      {!ready ? (
        <Alert type='error' showIcon content={t('pluginWorkbench.dialogs.apply.noCandidate')} />
      ) : (
        <div className={styles.dialogForm}>
          {failure && <Alert type='error' showIcon content={failure.message} />}
          <div className={styles.dialogFieldLabel}>{t('pluginWorkbench.dialogs.apply.target')}</div>
          <div className={styles.applyTarget}>
            <Code theme='outline' size='16' />
            <span>{targetLabel}</span>
          </div>
          {blockedByTarget && (
            <Alert type='error' showIcon content={t('pluginWorkbench.dialogs.apply.targetUnavailable')} />
          )}

          {needsBreakingAcknowledgement && (
            <Checkbox checked={allowBreaking} onChange={setAllowBreaking}>
              {t('pluginWorkbench.dialogs.apply.allowBreaking')}
            </Checkbox>
          )}
          {needsTestAcknowledgement && (
            <Checkbox
              checked={acknowledgeTestWarning}
              onChange={setAcknowledgeTestWarning}
            >
              {t('pluginWorkbench.dialogs.apply.acknowledgeTestWarning')}
            </Checkbox>
          )}

          {(ready.impact.changed_contracts.length > 0 || ready.impact.affected_consumers.length > 0) && (
            <details className={styles.dialogSubsection}>
              <summary>{t('common.technical_details')}</summary>
              <div className={styles.dialogChipList}>
                {ready.impact.changed_contracts.map((contract) => (
                  <span key={contract} className={`${styles.chip} ${styles.mono}`}>
                    {contract}
                  </span>
                ))}
                {ready.impact.affected_consumers.map((consumer) => (
                  <span key={`${consumer.surface}:${consumer.consumer_id}`} className={styles.chip}>
                    {consumer.surface}: {consumer.consumer_id}
                  </span>
                ))}
              </div>
            </details>
          )}
          {!ready.impact.can_apply && ready.impact.blocking_reasons.length > 0 && (
            <Alert
              type='warning'
              showIcon
              content={t('pluginWorkbench.dialogs.apply.applyBlocked')}
            />
          )}
        </div>
      )}
    </NomiModal>
  );
};

export default PluginProjectCreateModal;
