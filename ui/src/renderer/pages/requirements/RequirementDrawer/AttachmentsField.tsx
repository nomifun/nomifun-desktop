/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Collapsible image-attachments field for the requirement create/edit drawer.
 *
 * Extracted verbatim from the old `RequirementFormPage` dropzone (drag / paste /
 * click-to-upload, image-only + 30MB validation, temp→persistent attachment
 * refs, existing-attachment display/remove). The host owns persistence: this
 * field only manages attachment refs via the `onChange` / `onRemoveExisting`
 * callbacks and never calls requirement create/update itself.
 *
 * Wraps the dropzone in a self-rolled collapsible section (default COLLAPSED)
 * so it no longer dominates the form. Header shows the label + a running count
 * (existing + newly-added).
 */

import classNames from 'classnames';
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '@arco-design/web-react';
import { Down, Right } from '@icon-park/react';
import type { INewAttachmentRef, IAttachment } from '@/common/adapter/ipcBridge';
import type { AttachmentId } from '@/common/types/ids';
import type { FileMetadata } from '@/renderer/services/FileService';
import { FileService, getFileExtension, imageExts } from '@/renderer/services/FileService';
import { useDragUpload } from '@renderer/hooks/file/useDragUpload';
import { usePasteService } from '@renderer/hooks/file/usePasteService';
import { useUploadState } from '@renderer/hooks/file/useUploadState';
import { useArcoMessage } from '@renderer/utils/ui/useArcoMessage';
import FilePreview from '@/renderer/components/media/FilePreview';
import UploadProgressBar from '@/renderer/components/media/UploadProgressBar';

interface AttachmentsFieldProps {
  /** Newly added (uploaded) refs — controlled value. */
  value: INewAttachmentRef[];
  onChange: (refs: INewAttachmentRef[]) => void;
  /** Already-persisted attachments (edit mode). */
  existing?: IAttachment[];
  onRemoveExisting?: (id: AttachmentId) => void;
  /** Lets the host disable submit while uploads are in flight. */
  onUploadingChange?: (uploading: boolean) => void;
}

const AttachmentsField: React.FC<AttachmentsFieldProps> = ({
  value,
  onChange,
  existing = [],
  onRemoveExisting,
  onUploadingChange,
}) => {
  const { t } = useTranslation();
  const [message, messageCtx] = useArcoMessage();
  const [open, setOpen] = useState(false);
  const fileInputRef = useRef<HTMLInputElement>(null);
  // click-to-upload (file input) in-flight flag
  const [attachUploading, setAttachUploading] = useState(false);
  // drag/paste go through an HTTP upload that doesn't surface in `value` until
  // it lands — track those via the shared 'requirement' upload store too.
  const { isUploading: attachmentUploadsInFlight } = useUploadState('requirement');

  // Surface a single combined "uploading" signal to the host so it can disable
  // submit (mirrors the original page's `attachUploading || attachmentUploadsInFlight`).
  const uploading = attachUploading || attachmentUploadsInFlight;
  useEffect(() => {
    onUploadingChange?.(uploading);
  }, [uploading, onUploadingChange]);

  const handleFilesAdded = useCallback(
    (files: FileMetadata[]) => {
      const images = files.filter((f) => imageExts.includes(getFileExtension(f.name)));
      if (images.length < files.length) {
        message.warning(t('requirements.form.attachmentsOnlyImages'));
      }
      if (images.length > 0) {
        onChange([...value, ...images.map((f) => ({ source_path: f.path, file_name: f.name }))]);
      }
    },
    [message, t, onChange, value]
  );

  const { dragHandlers, isFileDragging } = useDragUpload({
    supportedExts: imageExts,
    onFilesAdded: handleFilesAdded,
    source: 'requirement',
  });

  const pasteService = usePasteService({
    supportedExts: imageExts,
    onFilesAdded: handleFilesAdded,
    source: 'requirement',
  });

  const handleFileInputChange = useCallback(
    async (e: React.ChangeEvent<HTMLInputElement>) => {
      const fileList = e.target.files;
      if (!fileList || fileList.length === 0) return;
      setAttachUploading(true);
      try {
        const processed = await FileService.processDroppedFiles(fileList, undefined, 'requirement');
        handleFilesAdded(processed);
      } catch (err) {
        message.error(
          err instanceof Error && err.message === 'FILE_TOO_LARGE'
            ? t('requirements.form.attachmentTooLarge')
            : String(err)
        );
      } finally {
        setAttachUploading(false);
      }
      e.target.value = '';
    },
    [handleFilesAdded, message, t]
  );

  const removeAdded = useCallback(
    (index: number) => {
      onChange(value.filter((_, j) => j !== index));
    },
    [onChange, value]
  );

  const toggleOpen = useCallback(() => setOpen((o) => !o), []);
  const onHeaderKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      setOpen((o) => !o);
    }
  }, []);

  const count = existing.length + value.length;

  return (
    <div className='rounded-8px border border-solid border-border-2 overflow-hidden'>
      {messageCtx}
      {/* ── Collapsible header ─────────────────────────────────────────── */}
      <div
        role='button'
        tabIndex={0}
        aria-expanded={open}
        onClick={toggleOpen}
        onKeyDown={onHeaderKeyDown}
        className={classNames(
          'flex items-center gap-8px px-12px py-10px cursor-pointer select-none outline-none',
          'text-14px text-t-primary hover:bg-fill-2 transition-colors'
        )}
      >
        {open ? (
          <Down theme='outline' size='14' className='text-t-secondary' />
        ) : (
          <Right theme='outline' size='14' className='text-t-secondary' />
        )}
        <span className='font-medium'>{t('requirements.form.attachmentsLabel')}</span>
        {count > 0 ? <span className='text-t-secondary'>({count})</span> : null}
      </div>

      {/* ── Expanded body: dropzone + previews ─────────────────────────── */}
      {open ? (
        <div className='border-t border-solid border-border-2 px-12px py-12px flex flex-col gap-8px'>
          <div className='text-12px text-t-secondary'>{t('requirements.form.attachmentsHelp')}</div>
          <div
            {...dragHandlers}
            onPaste={pasteService.onPaste}
            onFocus={pasteService.onFocus}
            tabIndex={-1}
            className={classNames(
              'rounded-8px border border-dashed p-12px flex flex-col gap-8px outline-none',
              isFileDragging ? 'border-primary-6 bg-primary-1' : 'border-border-2'
            )}
          >
            <div className='flex flex-wrap gap-8px'>
              {existing.map((a) => (
                <FilePreview key={a.id} path={a.abs_path} onRemove={() => onRemoveExisting?.(a.id)} />
              ))}
              {value.map((f, i) => (
                <FilePreview
                  key={`${f.source_path}-${i}`}
                  path={f.source_path}
                  onRemove={() => removeAdded(i)}
                />
              ))}
              <Button
                shape='round'
                size='small'
                loading={attachUploading}
                onClick={() => fileInputRef.current?.click()}
              >
                {t('requirements.form.addImage')}
              </Button>
            </div>
            <UploadProgressBar source='requirement' />
            <input
              ref={fileInputRef}
              type='file'
              multiple
              accept='.jpg,.jpeg,.png,.gif,.bmp,.webp,.svg'
              style={{ display: 'none' }}
              onChange={handleFileInputChange}
              data-testid='requirement-attachment-input'
            />
          </div>
        </div>
      ) : null}
    </div>
  );
};

export default AttachmentsField;
