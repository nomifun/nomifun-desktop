/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Modal } from '@arco-design/web-react';
import React from 'react';
import styles from './CreativeDetailLayout.module.css';

export const CreativeDetailModal: React.FC<React.PropsWithChildren<{
  visible: boolean;
  title: React.ReactNode;
  onClose: () => void;
}>> = ({ visible, title, onClose, children }) => (
  <Modal
    visible={visible}
    title={title}
    footer={null}
    style={{ width: 900, maxWidth: 'calc(100vw - 32px)' }}
    autoFocus={false}
    focusLock
    unmountOnExit
    getPopupContainer={() => document.getElementById('creative-studio-portal-root') ?? document.body}
    onCancel={onClose}
  >
    {children}
  </Modal>
);

interface CreativeDetailLayoutProps extends React.HTMLAttributes<HTMLDivElement> {
  visual: React.ReactNode;
  badges?: React.ReactNode;
  footer?: React.ReactNode;
}

/** Shared artwork details: independently scrolling preview and information. */
export const CreativeDetailLayout: React.FC<CreativeDetailLayoutProps> = ({
  visual, badges, footer, children, className, ...props
}) => (
  <div {...props} className={className} data-creative-detail-layout>
    <div className={styles.resultDetails}>
      <div className={styles.detailVisualColumn} data-creative-detail-visual>{visual}</div>
      <div className={styles.detailInfoColumn} data-creative-detail-info>
        {badges ? <div className={styles.detailTags}>{badges}</div> : null}
        {children}
      </div>
    </div>
    {footer ? <footer className={styles.detailFooter}>{footer}</footer> : null}
  </div>
);

export const CreativeDetailSection: React.FC<React.PropsWithChildren<{
  label: React.ReactNode;
  action?: React.ReactNode;
}>> = ({ label, action, children }) => (
  <section className={styles.detailSection}>
    <div className={styles.detailSectionHeading}>
      <span className={styles.detailSectionLabel}>{label}</span>
      {action}
    </div>
    {children}
  </section>
);

export const CreativeDetailText: React.FC<React.PropsWithChildren<{ error?: boolean }>> = ({ children, error }) => (
  <p className={styles.detailPrompt} data-error={error || undefined}>{children}</p>
);

export const CreativeDetailFacts: React.FC<React.PropsWithChildren> = ({ children }) => (
  <dl className={styles.detailFacts}>{children}</dl>
);
