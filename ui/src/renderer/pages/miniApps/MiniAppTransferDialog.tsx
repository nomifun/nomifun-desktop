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
  mode:
    | 'export'
    | 'export_backup'
    | 'import_share'
    | 'import_artifact'
    | 'import_backup';
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
  artifactDigest?: string;
  bundleDigest?: string;
  backupMetadataDigest?: string;
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

function backupSummary(
  value: unknown,
  backupMetadataDigest: string
): ImportSummary | null {
  const metadata = asRecord(value);
  const sourceMiniAppId = stringField(metadata, 'source_miniapp_id');
  return sourceMiniAppId && backupMetadataDigest
    ? { displayName: '', backupMetadataDigest }
    : null;
}

function normalizeSuggestedFolderName(
  displayName: string | undefined,
  suffix: 'share' | 'backup'
): string {
  const normalized = (displayName || 'miniapp')
    .trim()
    .replace(INVALID_FOLDER_CHARACTER, '-')
    .replace(/[. ]+$/g, '');
  return `${normalized || 'miniapp'}-${suffix}`;
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
      normalizeSuggestedFolderName(
        workshop?.miniapp.display_name,
        mode === 'export_backup' ? 'backup' : 'share'
      )
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
      : mode === 'export_backup'
        ? t('miniApps.transfer.exportBackup.title')
      : mode === 'import_share'
        ? t('miniApps.transfer.importShare.title')
        : mode === 'import_artifact'
          ? t('miniApps.transfer.importArtifact.title')
          : t('miniApps.transfer.importBackup.title');
  const intro =
    mode === 'export'
      ? t('miniApps.transfer.export.intro')
      : mode === 'export_backup'
        ? t('miniApps.transfer.exportBackup.intro')
      : mode === 'import_share'
        ? t('miniApps.transfer.importShare.intro')
        : mode === 'import_artifact'
          ? t('miniApps.transfer.importArtifact.intro')
          : t('miniApps.transfer.importBackup.intro');
  const submitLabel =
    mode === 'export'
      ? t('miniApps.transfer.export.submit')
      : mode === 'export_backup'
        ? t('miniApps.transfer.exportBackup.submit')
      : mode === 'import_share'
        ? t('miniApps.transfer.importShare.submit')
        : mode === 'import_artifact'
          ? t('miniApps.transfer.importArtifact.submit')
          : t('miniApps.transfer.importBackup.submit');

  const canSubmit =
    mode === 'export' || mode === 'export_backup'
      ? Boolean(
          workshop &&
            destinationPath &&
            (mode === 'export_backup'
              ? workshop.miniapp.lifecycle === 'disabled'
              : selectedRelease)
        )
      : Boolean(
          sourcePath &&
            importSummary &&
            displayName.trim() &&
            (mode === 'import_share'
              ? importSummary.bundleDigest && importSummary.artifactDigest
              : mode === 'import_artifact'
                ? importSummary.artifactDigest
                : importSummary.backupMetadataDigest)
        );

  const sha256Hex = async (value: string): Promise<string> => {
    const digest = await globalThis.crypto.subtle.digest(
      'SHA-256',
      new TextEncoder().encode(value)
    );
    return Array.from(new Uint8Array(digest), (byte) =>
      byte.toString(16).padStart(2, '0')
    ).join('');
  };

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

      if (mode === 'export' || mode === 'export_backup') {
        setParentPath(selectedPath);
        return;
      }

      setSourcePath(selectedPath);
      setDisplayName('');
      setImportSummary(null);
      const manifestRelativePath =
        mode === 'import_share'
          ? 'bundle.json'
          : mode === 'import_artifact'
            ? 'release/artifact.json'
            : 'metadata.json';
      const metadataContents = await ipcBridge.fs.readFile.invoke({
        path: joinLocalPath(selectedPath, manifestRelativePath),
      });
      if (!metadataContents) {
        throw new Error(t('miniApps.transfer.errors.metadataUnreadable'));
      }
      let parsed: unknown;
      try {
        parsed = JSON.parse(metadataContents) as unknown;
      } catch {
        throw new Error(t('miniApps.transfer.errors.metadataInvalid'));
      }
      const summary =
        mode === 'import_share'
          ? shareSummary(parsed)
          : mode === 'import_artifact'
            ? artifactSummary(parsed)
            : backupSummary(
                parsed,
                await sha256Hex(metadataContents)
              );
      if (!summary) {
        throw new Error(t('miniApps.transfer.errors.metadataInvalid'));
      }
      if (mode === 'import_backup') {
        const productContents = await ipcBridge.fs.readFile.invoke({
          path: joinLocalPath(selectedPath, 'product.json'),
        });
        if (productContents) {
          try {
            const product = asRecord(JSON.parse(productContents));
            const productName = stringField(product, 'display_name');
            if (productName) summary.displayName = productName;
          } catch {
            throw new Error(t('miniApps.transfer.errors.metadataInvalid'));
          }
        }
      }
      setImportSummary(summary);
      setDisplayName(summary.displayName || t('miniApps.transfer.importBackup.defaultName'));
    } catch (error) {
      setRequestError(errorMessage(error));
    } finally {
      setPicking(false);
    }
  };

  const submit = async () => {
    setValidationError('');
    setRequestError('');

    if (mode === 'export' || mode === 'export_backup') {
      if (
        !workshop ||
        (mode === 'export' && !selectedRelease) ||
        (mode === 'export_backup' && workshop.miniapp.lifecycle !== 'disabled')
      ) {
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
      setSubmitting(true);
      try {
        const operation =
          mode === 'export_backup'
            ? await ipcBridge.miniapps.exportBackup.invoke({
                miniapp_id: workshop.miniapp.miniapp_id,
                expected_product_revision: workshop.miniapp.product_revision,
                expected_lifecycle: 'disabled',
                expected_pointer_revision:
                  workshop.miniapp.releases.pointer_revision,
                expected_config_revision: workshop.config.config_revision,
                expected_credential_bindings_revision:
                  workshop.credential_bindings_revision,
                destination_path: targetPath,
              })
            : await ipcBridge.miniapps.share.invoke(
                miniAppShareRequest(
                  workshop,
                  exportContent,
                  targetPath,
                  includeSource
                )!
              );
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
              expected_release_digest: importSummary.artifactDigest!,
              display_name: importedDisplayName,
            })
          : mode === 'import_artifact'
            ? await ipcBridge.miniapps.importArtifact.invoke({
                expected_library_revision: libraryRevision,
                source_path: joinLocalPath(sourcePath, 'release'),
                expected_artifact_digest: importSummary.artifactDigest!,
                display_name: importedDisplayName,
              })
            : await ipcBridge.miniapps.importBackup.invoke({
                expected_library_revision: libraryRevision,
                source_path: sourcePath,
                expected_backup_metadata_digest:
                  importSummary.backupMetadataDigest!,
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
      aria-describedby='miniapp-transfer-intro'
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
      <p id='miniapp-transfer-intro' className={styles.dialogIntro}>
        {intro}
      </p>
      <div className={styles.transferForm}>
        {mode === 'export' || mode === 'export_backup' ? (
          <>
            {mode === 'export' ? <div className={styles.transferField}>
              <div className={styles.transferFieldLabel}>
                {t('miniApps.transfer.export.release')}
              </div>
              <Radio.Group
                type='button'
                aria-label={t('miniApps.transfer.export.release')}
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
            </div> : (
              <Alert
                type='info'
                showIcon
                content={t('miniApps.transfer.exportBackup.disabledOnly')}
              />
            )}

            {mode === 'export' && <div className={styles.transferField}>
              <Checkbox
                checked={includeSource}
                disabled={!canIncludeSource}
                aria-label={t('miniApps.transfer.export.includeSource')}
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
            </div>}

            <div className={styles.transferField}>
              <div className={styles.transferFieldLabel}>
                {t('miniApps.transfer.export.parentDirectory')}
              </div>
              <div className={styles.transferPickerRow}>
                <Input
                  value={parentPath}
                  readOnly
                  aria-label={t('miniApps.transfer.export.parentDirectory')}
                  title={parentPath}
                  placeholder={t(
                    'miniApps.transfer.export.parentPlaceholder'
                  )}
                />
                <Button
                  icon={<FolderClose theme='outline' size='14' />}
                  aria-label={`${t('miniApps.transfer.chooseDirectory')}: ${t(
                    'miniApps.transfer.export.parentDirectory'
                  )}`}
                  loading={picking}
                  disabled={submitting}
                  onClick={() => void pickDirectory()}
                >
                  {t('miniApps.transfer.chooseDirectory')}
                </Button>
              </div>
            </div>

            {(mode === 'export' || mode === 'export_backup') && <div className={styles.transferField}>
              <div className={styles.transferFieldLabel}>
                {t('miniApps.transfer.export.folderName')}
              </div>
              <Input
                value={folderName}
                maxLength={120}
                disabled={submitting}
                aria-label={t('miniApps.transfer.export.folderName')}
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
                {t(
                  mode === 'export_backup'
                    ? 'miniApps.transfer.exportBackup.destinationHint'
                    : 'miniApps.transfer.export.destinationHint'
                )}
              </div>
              {destinationPath && (
                <div className={styles.transferPath} title={destinationPath}>
                  {destinationPath}
                </div>
              )}
            </div>}
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
                  aria-label={t('miniApps.transfer.import.sourceDirectory')}
                  title={sourcePath}
                  placeholder={t(
                    mode === 'import_share'
                      ? 'miniApps.transfer.importShare.sourcePlaceholder'
                      : mode === 'import_artifact'
                        ? 'miniApps.transfer.importArtifact.sourcePlaceholder'
                        : 'miniApps.transfer.importBackup.sourcePlaceholder'
                  )}
                />
                <Button
                  icon={<FolderClose theme='outline' size='14' />}
                  aria-label={`${t('miniApps.transfer.chooseDirectory')}: ${t(
                    'miniApps.transfer.import.sourceDirectory'
                  )}`}
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
                    : mode === 'import_artifact'
                      ? 'miniApps.transfer.importArtifact.layoutHint'
                      : 'miniApps.transfer.importBackup.layoutHint'
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
                aria-label={t('miniApps.transfer.import.displayName')}
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
                {importSummary.artifactDigest && (
                  <div className={styles.transferMetadataRow}>
                    <span>{t('miniApps.transfer.import.artifactDigest')}</span>
                    <code title={importSummary.artifactDigest}>
                      {importSummary.artifactDigest}
                    </code>
                  </div>
                )}
                {importSummary.backupMetadataDigest && (
                  <div className={styles.transferMetadataRow}>
                    <span>
                      {t('miniApps.transfer.import.backupMetadataDigest')}
                    </span>
                    <code title={importSummary.backupMetadataDigest}>
                      {importSummary.backupMetadataDigest}
                    </code>
                  </div>
                )}
              </div>
            )}

            <Alert
              type='info'
              showIcon
              role='status'
              aria-live='polite'
              content={t('miniApps.transfer.import.backendAuthority')}
            />
          </>
        )}

        {validationError && (
          <Alert
            type='warning'
            showIcon
            role='alert'
            aria-live='assertive'
            content={validationError}
          />
        )}
        {requestError && (
          <Alert
            type='error'
            showIcon
            role='alert'
            aria-live='assertive'
            content={requestError}
          />
        )}
      </div>
    </TransferModal>
  );
};

export default MiniAppTransferDialog;
