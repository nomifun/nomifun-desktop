/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import type {
  MiniAppOperationSummary,
  MiniAppShareContent,
  MiniAppWorkshop,
} from '@/common/types/miniAppPlatform';
import { joinLocalPath } from '@/common/utils/localPath';
import {
  Alert,
  Button,
  Checkbox,
  Input,
  Modal,
  Radio,
  type ModalProps,
} from '@arco-design/web-react';
import { Download, FolderClose, Upload } from '@icon-park/react';
import React, { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { miniAppShareRequest } from './model';
import styles from './MiniAppWorkbench.module.css';

export interface MiniAppTransferDialogProps {
  mode: 'export' | 'import_share' | 'import_artifact';
  visible: boolean;
  libraryRevision: number;
  workshop?: MiniAppWorkshop | null;
  onCancel: () => void;
  onImported: (workshop: MiniAppWorkshop) => void;
  onExported: (
    operation: MiniAppOperationSummary,
    destinationPath: string
  ) => void;
}

interface ImportSummary {
  displayName: string;
  artifactDigest: string;
  bundleDigest?: string;
}

type JsonRecord = Record<string, unknown>;

const TransferModal =
  Modal as unknown as React.ComponentType<
    React.PropsWithChildren<ModalProps>
  >;

const WINDOWS_RESERVED_NAME =
  /^(?:con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/i;
const INVALID_FOLDER_CHARACTER = /[<>:"/\\|?*\u0000-\u001f]/;

function asRecord(value: unknown): JsonRecord | null {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
    ? (value as JsonRecord)
    : null;
}

function stringField(record: JsonRecord | null, field: string): string | null {
  const value = record?.[field];
  return typeof value === 'string' && value.trim() ? value.trim() : null;
}

function artifactSummary(value: unknown): ImportSummary | null {
  const artifact = asRecord(value);
  const manifest = asRecord(artifact?.manifest);
  const payload = asRecord(manifest?.payload);
  const display = asRecord(payload?.display);
  const displayName = stringField(display, 'name');
  const artifactDigest = stringField(artifact, 'artifact_digest');
  return displayName && artifactDigest
    ? { displayName, artifactDigest }
    : null;
}

function shareSummary(value: unknown): ImportSummary | null {
  const bundle = asRecord(value);
  const release = artifactSummary(bundle?.release);
  const bundleDigest = stringField(bundle, 'bundle_digest');
  return release && bundleDigest ? { ...release, bundleDigest } : null;
}

function normalizeSuggestedFolderName(displayName: string | undefined): string {
  const normalized = (displayName || 'miniapp')
    .trim()
    .replace(INVALID_FOLDER_CHARACTER, '-')
    .replace(/[. ]+$/g, '');
  return `${normalized || 'miniapp'}-share`;
}

function validFolderName(value: string): boolean {
  const normalized = value.trim();
  return Boolean(
    normalized &&
      normalized !== '.' &&
      normalized !== '..' &&
      !INVALID_FOLDER_CHARACTER.test(normalized) &&
      !/[. ]$/.test(normalized) &&
      !WINDOWS_RESERVED_NAME.test(normalized)
  );
}

function errorMessage(error: unknown): string {
  if (isBackendHttpError(error)) {
    const detail = error.backendMessage || error.message;
    return error.code ? `${error.code}: ${detail}` : detail;
  }
  return error instanceof Error ? error.message : String(error);
}

const MiniAppTransferDialog: React.FC<MiniAppTransferDialogProps> = ({
  mode,
  visible,
  libraryRevision,
  workshop,
  onCancel,
  onImported,
  onExported,
}) => {
  const { t } = useTranslation();
  const [exportContent, setExportContent] =
    useState<MiniAppShareContent>('ready_release');
  const [includeSource, setIncludeSource] = useState(false);
  const [parentPath, setParentPath] = useState('');
  const [folderName, setFolderName] = useState('');
  const [sourcePath, setSourcePath] = useState('');
  const [displayName, setDisplayName] = useState('');
  const [importSummary, setImportSummary] = useState<ImportSummary | null>(
    null
  );
  const [picking, setPicking] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [validationError, setValidationError] = useState('');
  const [requestError, setRequestError] = useState('');

  const miniappId = workshop?.miniapp.miniapp_id;

  useEffect(() => {
    if (!visible) return;
    setExportContent(workshop?.ready ? 'ready_release' : 'active_release');
    setIncludeSource(workshop?.source_state === 'editable');
    setParentPath('');
    setFolderName(
      normalizeSuggestedFolderName(workshop?.miniapp.display_name)
    );
    setSourcePath('');
    setDisplayName('');
    setImportSummary(null);
    setPicking(false);
    setSubmitting(false);
    setValidationError('');
    setRequestError('');
  }, [miniappId, mode, visible]);

  const selectedRelease = useMemo(
    () =>
      exportContent === 'ready_release'
        ? workshop?.ready?.release
        : workshop?.miniapp.releases.active,
    [exportContent, workshop]
  );
  const canIncludeSource = workshop?.source_state === 'editable';
  const destinationPath =
    parentPath && validFolderName(folderName)
      ? joinLocalPath(parentPath, folderName.trim())
      : '';

  const title =
    mode === 'export'
      ? t('miniApps.transfer.export.title')
      : mode === 'import_share'
        ? t('miniApps.transfer.importShare.title')
        : t('miniApps.transfer.importArtifact.title');
  const intro =
    mode === 'export'
      ? t('miniApps.transfer.export.intro')
      : mode === 'import_share'
        ? t('miniApps.transfer.importShare.intro')
        : t('miniApps.transfer.importArtifact.intro');
  const submitLabel =
    mode === 'export'
      ? t('miniApps.transfer.export.submit')
      : mode === 'import_share'
        ? t('miniApps.transfer.importShare.submit')
        : t('miniApps.transfer.importArtifact.submit');

  const canSubmit =
    mode === 'export'
      ? Boolean(workshop && selectedRelease && destinationPath)
      : Boolean(
          sourcePath &&
            importSummary &&
            displayName.trim() &&
            (mode !== 'import_share' || importSummary.bundleDigest)
        );

  const pickDirectory = async () => {
    setPicking(true);
    setValidationError('');
    setRequestError('');
    try {
      const paths = await ipcBridge.dialog.showOpen.invoke({
        properties: ['openDirectory'],
      });
      const selectedPath = paths?.[0]?.trim();
      if (!selectedPath) return;

      if (mode === 'export') {
        setParentPath(selectedPath);
        return;
      }

      setSourcePath(selectedPath);
      setDisplayName('');
      setImportSummary(null);
      const manifestRelativePath =
        mode === 'import_share'
          ? 'bundle.json'
          : 'release/artifact.json';
      const manifestPath = joinLocalPath(
        selectedPath,
        manifestRelativePath
      );
      const contents = await ipcBridge.fs.readFile.invoke({
        path: manifestPath,
      });
      if (!contents) {
        throw new Error(t('miniApps.transfer.errors.metadataUnreadable'));
      }

      let parsed: unknown;
      try {
        parsed = JSON.parse(contents) as unknown;
      } catch {
        throw new Error(t('miniApps.transfer.errors.metadataInvalid'));
      }
      const summary =
        mode === 'import_share'
          ? shareSummary(parsed)
          : artifactSummary(parsed);
      if (!summary) {
        throw new Error(t('miniApps.transfer.errors.metadataInvalid'));
      }
      setImportSummary(summary);
      setDisplayName(summary.displayName);
    } catch (error) {
      setRequestError(errorMessage(error));
    } finally {
      setPicking(false);
    }
  };

  const submit = async () => {
    setValidationError('');
    setRequestError('');

    if (mode === 'export') {
      if (!workshop || !selectedRelease) {
        setValidationError(t('miniApps.transfer.errors.releaseRequired'));
        return;
      }
      if (!parentPath) {
        setValidationError(t('miniApps.transfer.errors.parentRequired'));
        return;
      }
      if (!validFolderName(folderName)) {
        setValidationError(t('miniApps.transfer.errors.folderNameInvalid'));
        return;
      }

      const targetPath = joinLocalPath(parentPath, folderName.trim());
      const request = miniAppShareRequest(
        workshop,
        exportContent,
        targetPath,
        includeSource
      );
      if (!request) {
        setValidationError(t('miniApps.transfer.errors.releaseRequired'));
        return;
      }
      setSubmitting(true);
      try {
        const operation = await ipcBridge.miniapps.share.invoke(request);
        onExported(operation, targetPath);
      } catch (error) {
        setRequestError(errorMessage(error));
      } finally {
        setSubmitting(false);
      }
      return;
    }

    if (!sourcePath || !importSummary) {
      setValidationError(t('miniApps.transfer.errors.sourceRequired'));
      return;
    }
    const importedDisplayName = displayName.trim();
    if (!importedDisplayName) {
      setValidationError(t('miniApps.transfer.errors.displayNameRequired'));
      return;
    }

    setSubmitting(true);
    try {
      const imported =
        mode === 'import_share'
          ? await ipcBridge.miniapps.importShare.invoke({
              expected_library_revision: libraryRevision,
              source_path: sourcePath,
              expected_bundle_digest: importSummary.bundleDigest!,
              expected_release_digest: importSummary.artifactDigest,
              display_name: importedDisplayName,
            })
          : await ipcBridge.miniapps.importArtifact.invoke({
              expected_library_revision: libraryRevision,
              source_path: joinLocalPath(sourcePath, 'release'),
              expected_artifact_digest: importSummary.artifactDigest,
              display_name: importedDisplayName,
            });
      onImported(imported);
    } catch (error) {
      setRequestError(errorMessage(error));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <TransferModal
      className={styles.modal}
      visible={visible}
      title={title}
      onCancel={submitting || picking ? undefined : onCancel}
      onOk={() => void submit()}
      okText={submitLabel}
      cancelText={t('miniApps.actions.cancel')}
      confirmLoading={submitting}
      okButtonProps={{ disabled: !canSubmit || picking }}
      cancelButtonProps={{ disabled: submitting || picking }}
      maskClosable={false}
      autoFocus={false}
      unmountOnExit
    >
      <p className={styles.dialogIntro}>{intro}</p>
      <div className={styles.transferForm}>
        {mode === 'export' ? (
          <>
            <div className={styles.transferField}>
              <div className={styles.transferFieldLabel}>
                {t('miniApps.transfer.export.release')}
              </div>
              <Radio.Group
                type='button'
                value={exportContent}
                onChange={(value: MiniAppShareContent) => {
                  setExportContent(value);
                  setValidationError('');
                }}
                options={[
                  {
                    label: t('miniApps.transfer.export.ready'),
                    value: 'ready_release',
                    disabled: !workshop?.ready,
                  },
                  {
                    label: t('miniApps.transfer.export.active'),
                    value: 'active_release',
                    disabled: !workshop?.miniapp.releases.active,
                  },
                ]}
              />
              {selectedRelease && (
                <div className={styles.transferDigest} title={selectedRelease.release_digest}>
                  {selectedRelease.release_digest}
                </div>
              )}
            </div>

            <div className={styles.transferField}>
              <Checkbox
                checked={includeSource}
                disabled={!canIncludeSource}
                onChange={setIncludeSource}
              >
                {t('miniApps.transfer.export.includeSource')}
              </Checkbox>
              <div className={styles.transferHint}>
                {t(
                  canIncludeSource
                    ? 'miniApps.transfer.export.includeSourceHint'
                    : 'miniApps.transfer.export.sourceUnavailable'
                )}
              </div>
            </div>

            <div className={styles.transferField}>
              <div className={styles.transferFieldLabel}>
                {t('miniApps.transfer.export.parentDirectory')}
              </div>
              <div className={styles.transferPickerRow}>
                <Input
                  value={parentPath}
                  readOnly
                  title={parentPath}
                  placeholder={t(
                    'miniApps.transfer.export.parentPlaceholder'
                  )}
                />
                <Button
                  icon={<FolderClose theme='outline' size='14' />}
                  loading={picking}
                  disabled={submitting}
                  onClick={() => void pickDirectory()}
                >
                  {t('miniApps.transfer.chooseDirectory')}
                </Button>
              </div>
            </div>

            <div className={styles.transferField}>
              <div className={styles.transferFieldLabel}>
                {t('miniApps.transfer.export.folderName')}
              </div>
              <Input
                value={folderName}
                maxLength={120}
                disabled={submitting}
                placeholder={t(
                  'miniApps.transfer.export.folderNamePlaceholder'
                )}
                onChange={(value: string) => {
                  setFolderName(value);
                  setValidationError('');
                }}
                prefix={<Download theme='outline' size='14' />}
              />
              <div className={styles.transferHint}>
                {t('miniApps.transfer.export.destinationHint')}
              </div>
              {destinationPath && (
                <div className={styles.transferPath} title={destinationPath}>
                  {destinationPath}
                </div>
              )}
            </div>
          </>
        ) : (
          <>
            <div className={styles.transferField}>
              <div className={styles.transferFieldLabel}>
                {t('miniApps.transfer.import.sourceDirectory')}
              </div>
              <div className={styles.transferPickerRow}>
                <Input
                  value={sourcePath}
                  readOnly
                  title={sourcePath}
                  placeholder={t(
                    mode === 'import_share'
                      ? 'miniApps.transfer.importShare.sourcePlaceholder'
                      : 'miniApps.transfer.importArtifact.sourcePlaceholder'
                  )}
                />
                <Button
                  icon={<FolderClose theme='outline' size='14' />}
                  loading={picking}
                  disabled={submitting}
                  onClick={() => void pickDirectory()}
                >
                  {t('miniApps.transfer.chooseDirectory')}
                </Button>
              </div>
              <div className={styles.transferHint}>
                {t(
                  mode === 'import_share'
                    ? 'miniApps.transfer.importShare.layoutHint'
                    : 'miniApps.transfer.importArtifact.layoutHint'
                )}
              </div>
            </div>

            <div className={styles.transferField}>
              <div className={styles.transferFieldLabel}>
                {t('miniApps.transfer.import.displayName')}
              </div>
              <Input
                value={displayName}
                maxLength={120}
                disabled={!importSummary || submitting}
                placeholder={t(
                  'miniApps.transfer.import.displayNamePlaceholder'
                )}
                onChange={(value: string) => {
                  setDisplayName(value);
                  setValidationError('');
                }}
                prefix={<Upload theme='outline' size='14' />}
              />
            </div>

            {importSummary && (
              <div className={styles.transferMetadata}>
                {importSummary.bundleDigest && (
                  <div className={styles.transferMetadataRow}>
                    <span>{t('miniApps.transfer.import.bundleDigest')}</span>
                    <code title={importSummary.bundleDigest}>
                      {importSummary.bundleDigest}
                    </code>
                  </div>
                )}
                <div className={styles.transferMetadataRow}>
                  <span>{t('miniApps.transfer.import.artifactDigest')}</span>
                  <code title={importSummary.artifactDigest}>
                    {importSummary.artifactDigest}
                  </code>
                </div>
              </div>
            )}

            <Alert
              type='info'
              showIcon
              content={t('miniApps.transfer.import.backendAuthority')}
            />
          </>
        )}

        {validationError && (
          <Alert type='warning' showIcon content={validationError} />
        )}
        {requestError && (
          <Alert type='error' showIcon content={requestError} />
        )}
      </div>
    </TransferModal>
  );
};

export default MiniAppTransferDialog;
