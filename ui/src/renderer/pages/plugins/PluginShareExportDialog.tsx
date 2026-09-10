/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type {
  PluginProjectDetail,
  PluginSummary,
  SharePluginRequest,
} from '@/common/types/pluginPlatform';
import { joinLocalPath } from '@/common/utils/localPath';
import { Alert, Button, Checkbox, Input, Modal, Radio } from '@arco-design/web-react';
import { FolderClose } from '@icon-park/react';
import React, { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { PluginLoadFailure } from './pluginWorkbenchModel';
import styles from './PluginWorkbenchPage.module.css';

type ShareSource = 'ready_candidate' | 'current_mount';

export interface PluginShareExportDialogProps {
  visible: boolean;
  detail: PluginProjectDetail | null;
  linkedMount?: PluginSummary;
  loading: boolean;
  failure?: PluginLoadFailure | null;
  onCancel: () => void;
  onSubmit: (request: SharePluginRequest) => void | Promise<void>;
}

const invalidFolderName = /[<>:"/\\|?*\u0000-\u001f]/;
const invalidFolderCharacters = /[<>:"/\\|?*\u0000-\u001f]/g;
const windowsReservedName = /^(?:con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/i;

const PluginShareExportDialog: React.FC<PluginShareExportDialogProps> = ({
  visible,
  detail,
  linkedMount,
  loading,
  failure,
  onCancel,
  onSubmit,
}) => {
  const { t } = useTranslation();
  const [source, setSource] = useState<ShareSource>('ready_candidate');
  const [includeSource, setIncludeSource] = useState(true);
  const [parentPath, setParentPath] = useState('');
  const [folderName, setFolderName] = useState('plugin-share');
  const [picking, setPicking] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!visible) return;
    setSource(detail?.ready ? 'ready_candidate' : 'current_mount');
    setIncludeSource(Boolean(detail?.ready && detail.summary.source_state === 'editable'));
    setParentPath('');
    setFolderName(
      `${detail?.summary.display_name || 'plugin'}-share`
        .replace(invalidFolderCharacters, '-')
        .replace(/[. ]+$/g, '')
    );
    setPicking(false);
    setError(null);
  }, [detail?.summary.project_id, visible]);

  const pickParent = async () => {
    setPicking(true);
    setError(null);
    try {
      const result = await ipcBridge.dialog.showOpen.invoke({
        properties: ['openDirectory'],
      });
      if (result?.[0]) setParentPath(result[0]);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    } finally {
      setPicking(false);
    }
  };

  const handleSubmit = async () => {
    const name = folderName.trim();
    if (
      !detail ||
      !parentPath ||
      !name ||
      invalidFolderName.test(name) ||
      /[. ]$/.test(name) ||
      windowsReservedName.test(name)
    ) {
      setError(t('pluginWorkbench.dialogs.share.destinationInvalid'));
      return;
    }
    const common = {
      project_id: detail.summary.project_id,
      expected_project_revision: detail.summary.project_revision,
      destination_path: joinLocalPath(parentPath, name),
    };
    let request: SharePluginRequest;
    if (source === 'ready_candidate') {
      if (!detail.ready) {
        setError(t('pluginWorkbench.dialogs.share.readyUnavailable'));
        return;
      }
      request = {
        ...common,
        source,
        candidate_id: detail.ready.candidate.candidate_id,
        expected_candidate_digest: detail.ready.candidate.candidate_digest,
        include_source: includeSource,
      };
    } else {
      if (!linkedMount?.current) {
        setError(t('pluginWorkbench.dialogs.share.currentUnavailable'));
        return;
      }
      request = {
        ...common,
        source,
        mount_id: linkedMount.mount_id,
        expected_mount_revision: linkedMount.mount_revision,
        expected_target_digest: linkedMount.current.artifact_digest,
        include_source: false,
      };
    }
    setError(null);
    await onSubmit(request);
  };

  return (
    <Modal
      visible={visible}
      title={t('pluginWorkbench.dialogs.share.title')}
      onCancel={loading ? undefined : onCancel}
      onOk={() => void handleSubmit()}
      okText={t('pluginWorkbench.dialogs.share.submit')}
      cancelText={t('pluginWorkbench.actions.cancel')}
      confirmLoading={loading}
      maskClosable={false}
      autoFocus={false}
      unmountOnExit
    >
      <div className={styles.dialogIntro}>{t('pluginWorkbench.dialogs.share.body')}</div>
      <div className={styles.dialogForm}>
        {failure && <Alert type='error' showIcon content={failure.message} />}
        <div className={styles.dialogFieldLabel}>{t('pluginWorkbench.dialogs.share.source')}</div>
        <Radio.Group
          type='button'
          value={source}
          onChange={(value: string) => {
            setSource(value as ShareSource);
            if (value === 'current_mount') setIncludeSource(false);
            setError(null);
          }}
        >
          <Radio value='ready_candidate' disabled={!detail?.ready}>
            {t('pluginWorkbench.dialogs.share.readyCandidate')}
          </Radio>
          <Radio value='current_mount' disabled={!linkedMount?.current}>
            {t('pluginWorkbench.dialogs.share.currentMount')}
          </Radio>
        </Radio.Group>
        <Checkbox
          checked={includeSource}
          disabled={source !== 'ready_candidate' || detail?.summary.source_state !== 'editable'}
          onChange={setIncludeSource}
        >
          {t('pluginWorkbench.dialogs.share.includeSource')}
        </Checkbox>
        <div className={styles.dialogHint}>{t('pluginWorkbench.dialogs.share.privacy')}</div>
        <div className={styles.dialogFieldLabel}>
          {t('pluginWorkbench.dialogs.share.destinationParent')}
        </div>
        <div className={styles.dialogPickerRow}>
          <Input value={parentPath} readOnly title={parentPath} />
          <Button
            icon={<FolderClose theme='outline' size='14' />}
            loading={picking}
            onClick={() => void pickParent()}
          >
            {t('pluginWorkbench.dialogs.share.chooseParent')}
          </Button>
        </div>
        <div className={styles.dialogFieldLabel}>{t('pluginWorkbench.dialogs.share.folderName')}</div>
        <Input value={folderName} onChange={setFolderName} spellCheck={false} />
        {error && <Alert type='error' showIcon content={error} />}
      </div>
    </Modal>
  );
};

export default PluginShareExportDialog;
