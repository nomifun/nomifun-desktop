/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Modal } from '@arco-design/web-react';
import classNames from 'classnames';
import React from 'react';

import styles from './CreativeResourceDialog.module.css';

export type CreativeResourceDialogKind = 'assets' | 'prompts' | 'templates';
export type CreativeResourceDialogScope = 'conversation' | 'canvas';

export interface CreativeResourceDialogProps {
  kind: CreativeResourceDialogKind;
  title: React.ReactNode;
  children: React.ReactNode;
  scope?: CreativeResourceDialogScope;
  contentClassName?: string;
  popupContainer?: HTMLElement | null;
  onClose(): void;
}

/** Shared desktop modal contract for Creative Studio resource entry points. */
const CreativeResourceDialog: React.FC<CreativeResourceDialogProps> = ({
  kind,
  title,
  children,
  scope,
  contentClassName,
  popupContainer,
  onClose,
}) => (
  <Modal
    visible
    title={title}
    footer={null}
    className={styles.dialog}
    style={{ width: 1120, maxWidth: 'calc(100vw - 48px)' }}
    autoFocus={false}
    focusLock
    maskClosable
    escToExit
    unmountOnExit
    getPopupContainer={popupContainer ? () => popupContainer : undefined}
    onCancel={onClose}
  >
    <div
      className={classNames(styles.content, contentClassName)}
      data-creative-resource-dialog={kind}
      data-creation-resource-dialog={scope === 'conversation' ? kind : undefined}
      data-canvas-resource-dialog={scope === 'canvas' ? kind : undefined}
    >
      {children}
    </div>
  </Modal>
);

export default CreativeResourceDialog;
