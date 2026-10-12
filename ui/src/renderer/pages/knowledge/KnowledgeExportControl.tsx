/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useRef, useState } from 'react';
import { Button, Input, Message } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import { isTauriRuntime } from '@/common/adapter/tauriRuntime';
import type { IKnowledgeBase } from '@/common/adapter/ipcBridge';
import { joinLocalPath } from '@/common/utils/localPath';
import NomiModal from '@/renderer/components/base/NomiModal';
import { KnowledgeErrorModal } from './KnowledgeErrorFeedback';
import { normalizeKnowledgeError, type KnowledgeErrorPresentation } from './knowledgeError';
import { defaultKnowledgeExportName, isKnowledgeExportNameValid, pickKnowledgeExportFile } from './knowledgeExport';

export default function KnowledgeExportControl({ base }: { base: IKnowledgeBase }) {
  const { t } = useTranslation();
  const busy = useRef(false);
  const [exporting, setExporting] = useState(false);
  const [error, setError] = useState<KnowledgeErrorPresentation | null>(null);
  const [directory, setDirectory] = useState<string | null>(null);
  const [filename, setFilename] = useState('');
  const [overwritePath, setOverwritePath] = useState<string | null>(null);

  const exportFile = async (path: string, overwrite: boolean) => {
    try {
      await ipcBridge.knowledge.exportBase.invoke({
        knowledge_base_id: base.knowledge_base_id,
        dest_path: path,
        overwrite,
      });
      Message.success(t('knowledge.detail.settings.exportOk'));
    } catch (failure) {
      if (!overwrite && isBackendHttpError(failure) && failure.code === 'KNOWLEDGE_EXPORT_DESTINATION_EXISTS') {
        setOverwritePath(path);
      } else {
        const presentation = normalizeKnowledgeError(failure, 'export', t);
        if (!presentation.handled) setError(presentation);
      }
    }
  };

  const chooseDestination = async () => {
    if (busy.current) return;
    busy.current = true;
    setExporting(true);
    setError(null);
    try {
      const defaultName = defaultKnowledgeExportName(base.name);
      if (isTauriRuntime()) {
        // Native save dialogs confirm replacement of an existing file.
        const path = await pickKnowledgeExportFile(defaultName);
        if (path) await exportFile(path, true);
      } else {
        // WebUI selects a server directory; append a validated ZIP basename.
        const paths = await ipcBridge.dialog.showOpen.invoke({ properties: ['openDirectory'] });
        if (paths?.[0]) {
          setFilename(defaultName);
          setDirectory(paths[0]);
        }
      }
    } catch (failure) {
      const presentation = normalizeKnowledgeError(failure, 'export', t);
      if (!presentation.handled) setError(presentation);
    } finally {
      busy.current = false;
      setExporting(false);
    }
  };

  const saveFile = async (path: string, overwrite: boolean) => {
    if (busy.current) return;
    busy.current = true;
    setExporting(true);
    setDirectory(null);
    setOverwritePath(null);
    try {
      await exportFile(path, overwrite);
    } finally {
      busy.current = false;
      setExporting(false);
    }
  };

  const validName = isKnowledgeExportNameValid(filename);
  const destination = directory ? joinLocalPath(directory, filename) : '';
  return (
    <>
      <Button size='mini' loading={exporting} disabled={Boolean(directory || overwritePath || error)} onClick={() => void chooseDestination()}>
        {t('knowledge.detail.settings.exportBtn')}
      </Button>
      <NomiModal
        visible={directory !== null}
        header={t('knowledge.export.saveTitle')}
        style={{ width: 520 }}
        maskClosable={false}
        onCancel={() => setDirectory(null)}
        footer={
          <div className='flex justify-end gap-8px'>
            <Button onClick={() => setDirectory(null)}>{t('common.cancel')}</Button>
            <Button type='primary' disabled={!validName} onClick={() => void saveFile(destination, false)}>{t('knowledge.detail.settings.exportBtn')}</Button>
          </div>
        }
      >
        <div className='flex flex-col gap-12px'>
          <label className='flex flex-col gap-6px text-13px'>
            {t('knowledge.export.filename')}
            <Input value={filename} onChange={setFilename} aria-label={t('knowledge.export.filename')} status={validName ? undefined : 'error'} />
          </label>
          {!validName && <div role='alert' className='text-12px text-[var(--danger)]'>{t('knowledge.export.filenameHint')}</div>}
          <div className='text-12px text-t-secondary'>
            <div className='mb-4px'>{t('knowledge.export.destination')}</div>
            <div className='break-all'>{destination}</div>
          </div>
        </div>
      </NomiModal>
      <NomiModal
        visible={overwritePath !== null}
        header={t('knowledge.export.overwriteTitle')}
        style={{ width: 520 }}
        maskClosable={false}
        onCancel={() => setOverwritePath(null)}
        footer={
          <div className='flex justify-end gap-8px'>
            <Button onClick={() => setOverwritePath(null)}>{t('common.cancel')}</Button>
            <Button type='primary' onClick={() => overwritePath && void saveFile(overwritePath, true)}>{t('knowledge.export.overwrite')}</Button>
          </div>
        }
      >
        <p className='m-0 mb-12px text-13px text-t-secondary'>{t('knowledge.export.overwriteHint')}</p>
        <div className='break-all text-12px text-t-secondary'>{overwritePath}</div>
      </NomiModal>
      <KnowledgeErrorModal
        error={error}
        onClose={() => setError(null)}
        onRetry={() => void chooseDestination()}
      />
    </>
  );
}
