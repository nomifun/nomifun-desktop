/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Modal } from '@arco-design/web-react';
import React from 'react';
import { useTranslation } from 'react-i18next';

import CreativeAssetPickerContent, {
  CREATIVE_ASSET_PICKER_KIND_FILTERS,
  type CreativeAssetPickerContentProps,
} from './CreativeAssetPickerContent';
import styles from './CreativeAssetPickerModal.module.css';

export { CREATIVE_ASSET_PICKER_KIND_FILTERS };

// The shared content owns the actual <CreativeAssetMedia /> records so the
// modal and embedded Canvas picker cannot drift apart visually.

export interface CreativeAssetPickerModalProps
  extends Omit<CreativeAssetPickerContentProps, 'open' | 'onCancel'> {
  open: boolean;
  title?: string;
  onCancel(): void;
}

const CreativeAssetPickerModal: React.FC<CreativeAssetPickerModalProps> = ({
  open,
  title,
  onCancel,
  ...contentProps
}) => {
  const { t } = useTranslation();

  return (
    <Modal
      visible={open}
      alignCenter={false}
      className={styles.modal}
      title={title ?? t('creativeStudio.assets.picker.title', { defaultValue: '资产库' })}
      footer={null}
      autoFocus={false}
      focusLock
      unmountOnExit
      getPopupContainer={() =>
        document.getElementById('resource-page-portal-root') ?? document.body
      }
      onCancel={onCancel}
    >
      <CreativeAssetPickerContent
        {...contentProps}
        open={open}
        onCancel={onCancel}
      />
    </Modal>
  );
};

export default CreativeAssetPickerModal;
