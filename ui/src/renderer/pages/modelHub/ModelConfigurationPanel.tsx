/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { createContext, useContext, useId, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Button } from '@arco-design/web-react';
import { Left } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import styles from './ModelConfigurationPanel.module.css';

const WorkspaceContext = createContext<{
  outlet: HTMLDivElement | null;
  setEditors: React.Dispatch<React.SetStateAction<number>>;
} | null>(null);

/** Keep the list mounted so returning from a form restores its state and scroll. */
export function ModelConfigurationWorkspace({ children, onEditingChange }: {
  children: React.ReactNode;
  onEditingChange?: (editing: boolean) => void;
}) {
  const [outlet, setOutlet] = useState<HTMLDivElement | null>(null);
  const [editors, setEditors] = useState(0);
  const editing = editors > 0;
  useLayoutEffect(() => onEditingChange?.(editing), [editing, onEditingChange]);

  return <WorkspaceContext.Provider value={{ outlet, setEditors }}>
    <div className={styles.workspace} data-model-configuration-workspace>
      <div className={styles.overview} hidden={editing} style={editing ? { display: 'none' } : undefined}>
        {children}
      </div>
      <div ref={setOutlet} className={styles.outlet} hidden={!editing} />
    </div>
  </WorkspaceContext.Provider>;
}

/** A normal content page: no mask, outside-click handler, or Escape dismissal. */
export default function ModelConfigurationPanel({ visible, title, subtitle, onBack, busy = false, footer, children }: {
  visible?: boolean;
  title: React.ReactNode;
  subtitle?: React.ReactNode;
  onBack: () => void;
  busy?: boolean;
  footer?: React.ReactNode;
  children: React.ReactNode;
}) {
  const { t } = useTranslation();
  const workspace = useContext(WorkspaceContext);
  const setEditors = workspace?.setEditors;
  const headingId = useId();
  const headingRef = useRef<HTMLHeadingElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    if (!visible) return;
    const trigger = document.activeElement instanceof HTMLElement ? document.activeElement : undefined;
    setEditors?.(count => count + 1);
    return () => {
      setEditors?.(count => count - 1);
      // The overview becomes visible in the same React commit.
      requestAnimationFrame(() => { if (trigger?.isConnected) trigger.focus({ preventScroll: true }); });
    };
  }, [setEditors, visible]);

  useLayoutEffect(() => {
    if (!visible) return;
    // Registration reveals the outlet after this commit. Focus once it is visible.
    const frame = requestAnimationFrame(() => {
      if (bodyRef.current) bodyRef.current.scrollTop = 0;
      headingRef.current?.focus({ preventScroll: true });
    });
    return () => cancelAnimationFrame(frame);
  }, [title, visible, workspace?.outlet]);

  if (!visible) return null;
  const page = <section className={styles.editor} aria-labelledby={headingId} data-model-configuration-page
    autoCapitalize='none' autoCorrect='off' spellCheck={false}>
    <header className={styles.header}>
      <div className={`${styles.width} ${styles.heading}`}>
        <Button type='secondary' className={styles.back} disabled={busy}
          aria-label={t('settings.modelConfiguration.back')} icon={<Left theme='outline' size='16' />} onClick={onBack} />
        <div className={styles.headingCopy}>
          <h2 ref={headingRef} id={headingId} tabIndex={-1} className={styles.title}>{title}</h2>
          {subtitle && <div className={styles.subtitle}>{subtitle}</div>}
        </div>
      </div>
    </header>
    <div ref={bodyRef} className={styles.body} data-model-configuration-body>
      <div className={styles.width}><div className={styles.sheet}>{children}</div></div>
    </div>
    {footer != null && <footer className={styles.footer}>
      <div className={styles.width}>{footer}</div>
    </footer>}
  </section>;
  return workspace?.outlet ? createPortal(page, workspace.outlet) : page;
}
