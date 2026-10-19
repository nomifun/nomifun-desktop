/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Dropdown } from '@arco-design/web-react';
import { More } from '@icon-park/react';
import React, { useCallback, useEffect, useId, useRef, useState } from 'react';

import styles from './CreativeStudioCanvasesPage.module.css';

interface CanvasAction {
  key: string;
  label: string;
  icon: React.ReactNode;
  onSelect: () => void;
  disabled?: boolean;
  disabledReason?: string;
  danger?: boolean;
}

interface CanvasActionsMenuProps {
  label: string;
  disabled?: boolean;
  triggerClassName: string;
  actions: readonly CanvasAction[];
}

const popupContainer = () =>
  document.getElementById('resource-page-portal-root') ?? document.body;

export default function CanvasActionsMenu({
  label,
  disabled = false,
  triggerClassName,
  actions,
}: CanvasActionsMenuProps) {
  const triggerRef = useRef<HTMLButtonElement>(null);
  const focusLast = useRef(false);
  const menuId = useId();
  const [menuOpen, setMenuOpen] = useState(false);
  const visible = menuOpen && !disabled;

  useEffect(() => {
    if (disabled) setMenuOpen(false);
  }, [disabled]);

  const closeMenu = () => {
    setMenuOpen(false);
    triggerRef.current?.focus({ preventScroll: true });
  };
  const focusMenu = useCallback((menu: HTMLDivElement | null) => {
    const items = menu?.querySelectorAll<HTMLButtonElement>(
      '[role="menuitem"]:not(:disabled)'
    );
    if (items?.length) {
      items[focusLast.current ? items.length - 1 : 0]?.focus({ preventScroll: true });
    }
  }, []);
  const onMenuKeyDown: React.KeyboardEventHandler<HTMLDivElement> = (event) => {
    event.stopPropagation();
    if (event.key === 'Escape' || event.key === 'Tab') {
      if (event.key === 'Escape') event.preventDefault();
      closeMenu();
      return;
    }
    if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
    event.preventDefault();
    const items = [
      ...event.currentTarget.querySelectorAll<HTMLButtonElement>(
        '[role="menuitem"]:not(:disabled)'
      ),
    ];
    if (!items.length) return;
    const index = items.indexOf(
      event.currentTarget.ownerDocument.activeElement as HTMLButtonElement
    );
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? items.length - 1
      : (index + (event.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length;
    items[next]?.focus({ preventScroll: true });
  };

  return (
    <Dropdown
      trigger='click'
      position='br'
      popupVisible={visible}
      onVisibleChange={(nextVisible) => {
        focusLast.current = false;
        setMenuOpen(nextVisible);
      }}
      disabled={disabled}
      unmountOnExit
      getPopupContainer={popupContainer}
      triggerProps={{ autoFitPosition: true, escToClose: true }}
      droplist={visible ? (
        <div id={menuId} ref={focusMenu} className={styles.cardMenu}
          role='menu' aria-label={label} onKeyDown={onMenuKeyDown}>
          {actions.map((action, index) => (
            <React.Fragment key={action.key}>
              {action.danger && index > 0 && <div className={styles.menuDivider} role='separator' />}
              <button type='button' role='menuitem'
                className={action.danger ? `${styles.menuItem} ${styles.dangerMenuItem}` : styles.menuItem}
                disabled={action.disabled}
                aria-disabled={action.disabled || undefined}
                title={action.disabled ? action.disabledReason : undefined}
                onClick={() => {
                  if (disabled || action.disabled) return;
                  closeMenu();
                  action.onSelect();
                }}>
                {action.icon}{action.label}
              </button>
            </React.Fragment>
          ))}
        </div>
      ) : <span />}
    >
      <button ref={triggerRef} type='button' className={triggerClassName}
        aria-label={label} title={label} aria-haspopup='menu'
        aria-expanded={visible} aria-controls={visible ? menuId : undefined}
        aria-disabled={disabled || undefined} disabled={disabled}
        onKeyDown={(event) => {
          if (disabled) return;
          if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
            event.preventDefault();
            focusLast.current = event.key === 'ArrowUp';
            setMenuOpen(true);
          }
        }}>
        <More theme='outline' size={16} fill='currentColor' />
      </button>
    </Dropdown>
  );
}
