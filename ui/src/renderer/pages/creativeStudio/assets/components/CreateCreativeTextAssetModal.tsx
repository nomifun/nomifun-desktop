/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Button, Checkbox, Input, InputTag, Modal } from '@arco-design/web-react';
import React from 'react';

import type { CreateTextAssetLabels, CreativeTextAssetFormValue } from './types';
import { DEFAULT_CREATE_TEXT_ASSET_LABELS } from './types';
import styles from './CreativeAssetLibrary.module.css';

export interface CreateCreativeTextAssetModalProps {
  open: boolean;
  value: CreativeTextAssetFormValue;
  submitting?: boolean;
  error?: string | null;
  labels?: Partial<CreateTextAssetLabels>;
  onChange: (value: CreativeTextAssetFormValue) => void;
  onCancel: () => void;
  onSubmit: (value: CreativeTextAssetFormValue) => void;
}

const CreateCreativeTextAssetModal: React.FC<CreateCreativeTextAssetModalProps> = ({
  open,
  value,
  submitting = false,
  error,
  labels: labelOverrides,
  onChange,
  onCancel,
  onSubmit,
}) => {
  const labels = { ...DEFAULT_CREATE_TEXT_ASSET_LABELS, ...labelOverrides };
  const valid = value.title.trim().length > 0 && value.textContent.trim().length > 0;
  const patch = (next: Partial<CreativeTextAssetFormValue>) => onChange({ ...value, ...next });

  return (
    <Modal
      visible={open}
      title={labels.title}
      footer={null}
      autoFocus={false}
      focusLock
      unmountOnExit
      maskClosable={!submitting}
      closable={!submitting}
      className={styles.textAssetModal}
      getPopupContainer={() => document.getElementById('creative-studio-portal-root') ?? document.body}
      onCancel={() => {
        if (!submitting) onCancel();
      }}
    >
      <form
        className={styles.textAssetForm}
        data-create-text-asset-form
        onSubmit={(event) => {
          event.preventDefault();
          if (valid && !submitting) onSubmit(value);
        }}
      >
        <p className={styles.modalDescription}>{labels.description}</p>

        <label className={styles.field}>
          <span>{labels.titleLabel}</span>
          <Input
            value={value.title}
            placeholder={labels.titlePlaceholder}
            maxLength={240}
            disabled={submitting}
            aria-required='true'
            onChange={(title) => patch({ title })}
          />
        </label>

        <label className={styles.field}>
          <span>{labels.contentLabel}</span>
          <Input.TextArea
            value={value.textContent}
            placeholder={labels.contentPlaceholder}
            autoSize={{ minRows: 7, maxRows: 14 }}
            maxLength={1_000_000}
            disabled={submitting}
            aria-required='true'
            onChange={(textContent) => patch({ textContent })}
          />
        </label>

        <div className={styles.formColumns}>
          <label className={styles.field}>
            <span>{labels.collectionLabel}</span>
            <Input
              value={value.collection}
              placeholder={labels.collectionPlaceholder}
              maxLength={240}
              disabled={submitting}
              onChange={(collection) => patch({ collection })}
            />
          </label>
          <label className={styles.field}>
            <span>{labels.tagsLabel}</span>
            <InputTag
              value={value.tags}
              placeholder={labels.tagsPlaceholder}
              allowClear
              disabled={submitting}
              onChange={(tags) => patch({ tags: tags.map(String) })}
            />
          </label>
        </div>

        <Checkbox
          checked={value.inLibrary}
          disabled={submitting}
          onChange={(inLibrary) => patch({ inLibrary })}
        >
          {labels.saveToLibrary}
        </Checkbox>

        {!valid ? <p className={styles.requiredHint}>{labels.requiredHint}</p> : null}
        {error ? (
          <p className={styles.formError} role='alert'>
            {error}
          </p>
        ) : null}

        <footer className={styles.modalFooter}>
          <Button type='secondary' disabled={submitting} onClick={onCancel}>
            {labels.cancel}
          </Button>
          <Button type='primary' htmlType='submit' loading={submitting} disabled={!valid || submitting}>
            {submitting ? labels.submitting : labels.submit}
          </Button>
        </footer>
      </form>
    </Modal>
  );
};

export default CreateCreativeTextAssetModal;
