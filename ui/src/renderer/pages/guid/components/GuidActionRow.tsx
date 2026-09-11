/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import { useLayoutContext } from '@/renderer/hooks/context/LayoutContext';
import { getCleanFileNames, FileService } from '@/renderer/services/FileService';
import { iconColors } from '@/renderer/styles/colors';
import { isDesktopShell } from '@/renderer/utils/platform';
import { Button, Dropdown, Menu, Message, Tooltip } from '@arco-design/web-react';
import { ArrowUp, Plus, Robot, UploadOne } from '@icon-park/react';
import React, { useCallback, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import styles from '../index.module.css';

type GuidActionRowProps = {
  // File handling
  files: string[];
  onFilesUploaded: (paths: string[]) => void;

  // Send button
  loading: boolean;
  isButtonDisabled: boolean;
  speechInputNode?: React.ReactNode;
  onSend: () => void;
  /** When true the primary button starts an AutoWork session (no chat send):
   * it shows a robot icon + "Start AutoWork" tooltip. Disabled/onClick are
   * still driven by the parent. */
  autoWorkMode?: boolean;
};

const GuidActionRow: React.FC<GuidActionRowProps> = ({
  files,
  onFilesUploaded,
  loading,
  isButtonDisabled,
  speechInputNode,
  onSend,
  autoWorkMode = false,
}) => {
  const { t } = useTranslation();
  const layout = useLayoutContext();
  const isMobile = layout?.isMobile ?? false;
  const [isPlusDropdownOpen, setIsPlusDropdownOpen] = useState(false);

  // Browser file picker ref (WebUI only)
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [uploading, setUploading] = useState(false);

  const handleLocalFileChange = useCallback(
    async (e: React.ChangeEvent<HTMLInputElement>) => {
      const fileList = e.target.files;
      if (!fileList || fileList.length === 0) return;
      setUploading(true);
      try {
        const processed = await FileService.processDroppedFiles(fileList);
        if (processed.length > 0) {
          onFilesUploaded(processed.map((f) => f.path));
        }
      } catch {
        Message.error(t('common.fileAttach.failed'));
      } finally {
        setUploading(false);
      }
      // Reset so the same file can be re-selected
      e.target.value = '';
    },
    [onFilesUploaded, t]
  );

  const isWebUI = !isDesktopShell();

  const menuContent = (
    <Menu
      className='min-w-200px'
      onClickMenuItem={(key: string) => {
        if (key === 'file') {
          ipcBridge.dialog.showOpen
            .invoke({ properties: ['openFile', 'multiSelections'] })
            .then((uploadedFiles) => {
              if (uploadedFiles && uploadedFiles.length > 0) {
                onFilesUploaded(uploadedFiles);
              }
            })
            .catch((error) => {
              console.error('Failed to open file dialog:', error);
            });
        } else if (key === 'device') {
          fileInputRef.current?.click();
        }
      }}
    >
      {isWebUI ? (
        <>
          <Menu.Item key='file'>
            <div className='flex items-center gap-8px'>
              <UploadOne theme='outline' size='16' fill={iconColors.secondary} style={{ lineHeight: 0 }} />
              <span>{t('common.fileAttach.addFiles')}</span>
            </div>
          </Menu.Item>
          <Menu.Item key='device'>
            <div className='flex items-center gap-8px'>
              <UploadOne theme='outline' size='16' fill={iconColors.secondary} style={{ lineHeight: 0 }} />
              <span>{t('common.fileAttach.myDevice')}</span>
            </div>
          </Menu.Item>
        </>
      ) : (
        <Menu.Item key='file'>
          <div className='flex items-center gap-8px'>
            <UploadOne theme='outline' size='16' fill={iconColors.secondary} style={{ lineHeight: 0 }} />
            <span>{t('common.fileAttach.addFiles')}</span>
          </div>
        </Menu.Item>
      )}
    </Menu>
  );

  return (
    <div className={styles.actionRow}>
      <div className={styles.actionTools}>
        <div className={styles.actionEntry}>
          <Dropdown trigger='hover' onVisibleChange={setIsPlusDropdownOpen} droplist={menuContent}>
            <span className='flex items-center gap-4px cursor-pointer lh-[1]'>
              <Button
                type='secondary'
                shape='circle'
                className={isPlusDropdownOpen ? styles.plusButtonRotate : ''}
                icon={<Plus theme='outline' size='14' strokeWidth={2} fill={iconColors.primary} />}
                loading={uploading}
                disabled={uploading}
                data-testid='file-upload-btn'
              />
              {files.length > 0 && (
                <Tooltip
                  className={'!max-w-max'}
                  content={<span className='whitespace-break-spaces'>{getCleanFileNames(files).join('\n')}</span>}
                >
                  <span className='text-t-primary'>File({files.length})</span>
                </Tooltip>
              )}
            </span>
          </Dropdown>
          {isWebUI && (
            <input
              ref={fileInputRef}
              type='file'
              multiple
              style={{ display: 'none' }}
              onChange={handleLocalFileChange}
            />
          )}
        </div>
      </div>
      <div className={`${styles.actionSubmit} ${!isMobile ? styles.actionSubmitResponsive : ''}`}>
        {speechInputNode}
        <Tooltip
          content={t('requirements.autowork.startSession')}
          disabled={!autoWorkMode}
        >
          <Button
            shape='circle'
            type='primary'
            loading={loading}
            disabled={isButtonDisabled}
            className='send-button-custom'
            style={{
              backgroundColor: isButtonDisabled ? undefined : '#000000',
              borderColor: isButtonDisabled ? undefined : '#000000',
            }}
            icon={
              autoWorkMode ? (
                <Robot theme='filled' size='14' fill='white' strokeWidth={5} />
              ) : (
                <ArrowUp theme='filled' size='14' fill='white' strokeWidth={5} />
              )
            }
            onClick={onSend}
            data-testid='guid-send-btn'
          />
        </Tooltip>
      </div>
    </div>
  );
};

export default GuidActionRow;
