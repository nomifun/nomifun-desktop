/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Dropdown } from '@arco-design/web-react';
import { Delete, Download, EditTwo, MoreOne, PreviewOpen } from '@icon-park/react';
import React, { useCallback, useEffect, useId, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { CreativeAsset } from '../types';
import type { CreativeAssetAction, CreativeAssetLibraryLabels } from './types';
import styles from './CreativeAssetLibrary.module.css';

interface CreativeAssetActionsMenuProps {
  asset: CreativeAsset;
  disabled: boolean;
  labels: CreativeAssetLibraryLabels;
  onOpen?: CreativeAssetAction;
  onEdit?: CreativeAssetAction;
  onDownload?: CreativeAssetAction;
  onRemove?: CreativeAssetAction;
}

const popupContainer = () => document.getElementById('creative-studio-portal-root') ?? document.body;
const iconProps = { theme: 'outline' as const, size: 15, fill: 'currentColor', strokeWidth: 3 };

/** Presentation-only menu: the page retains preview, edit and delete dialogs. */
const CreativeAssetActionsMenu: React.FC<CreativeAssetActionsMenuProps> = ({
  asset, disabled, labels, onOpen, onEdit, onDownload, onRemove,
}) => {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const focusLast = useRef(false);
  const menuId = useId();
  const moreLabel = `${t('common.more', { defaultValue: '更多' })}：${asset.title}`;
  const actions = [
    { key: 'open', label: labels.open, callback: onOpen, icon: <PreviewOpen {...iconProps} /> },
    { key: 'edit', label: labels.edit, callback: onEdit, icon: <EditTwo {...iconProps} /> },
    { key: 'download', label: labels.download, callback: asset.kind !== 'text' ? onDownload : undefined, icon: <Download {...iconProps} /> },
    { key: 'remove', label: labels.remove, callback: onRemove, icon: <Delete {...iconProps} /> },
  ].filter((action) => action.callback);

  useEffect(() => { if (disabled) setOpen(false); }, [disabled]);

  const focusMenu = useCallback((menu: HTMLDivElement | null) => {
    const items = menu?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]');
    if (items?.length) items[focusLast.current ? items.length - 1 : 0]?.focus({ preventScroll: true });
  }, []);

  const closeAndRestoreFocus = () => {
    setOpen(false);
    triggerRef.current?.focus({ preventScroll: true });
  };

  const onMenuKeyDown: React.KeyboardEventHandler<HTMLDivElement> = (event) => {
    if (event.key === 'Escape' || event.key === 'Tab') {
      if (event.key === 'Escape') event.preventDefault();
      event.stopPropagation();
      closeAndRestoreFocus();
      return;
    }
    if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
    event.preventDefault();
    const items = [...event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="menuitem"]:not(:disabled)')];
    if (!items.length) return;
    const index = items.indexOf(event.currentTarget.ownerDocument.activeElement as HTMLButtonElement);
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? items.length - 1
      : (index + (event.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length;
    items[next]?.focus();
  };

  if (!actions.length) return null;
  return (
    <Dropdown
      trigger='click'
      position='br'
      popupVisible={open && !disabled}
      onVisibleChange={(visible) => { focusLast.current = false; setOpen(visible); }}
      disabled={disabled}
      unmountOnExit
      getPopupContainer={popupContainer}
      triggerProps={{ autoFitPosition: true, escToClose: true }}
      droplist={open && !disabled ? (
        <div ref={focusMenu} id={menuId} className={styles.assetMenu} role='menu' aria-label={moreLabel} onKeyDown={onMenuKeyDown}>
          {actions.map((action) => (
            <button
              key={action.key}
              type='button'
              role='menuitem'
              className={styles.assetMenuItem}
              data-danger={action.key === 'remove' || undefined}
              disabled={disabled}
              onClick={(event) => {
                event.stopPropagation();
                if (disabled) return;
                closeAndRestoreFocus();
                action.callback?.(asset);
              }}
            >
              <span aria-hidden='true'>{action.icon}</span>
              <span>{action.label}</span>
            </button>
          ))}
        </div>
      ) : <span />}
    >
      <button
        ref={triggerRef}
        type='button'
        className={styles.assetMoreButton}
        aria-label={moreLabel}
        title={t('common.more', { defaultValue: '更多' })}
        aria-haspopup='menu'
        aria-expanded={open && !disabled}
        aria-controls={open && !disabled ? menuId : undefined}
        disabled={disabled}
        onKeyDown={(event) => {
          if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
            event.preventDefault();
            focusLast.current = event.key === 'ArrowUp';
            setOpen(true);
          }
        }}
      >
        <MoreOne theme='outline' size={18} fill='currentColor' strokeWidth={3} />
      </button>
    </Dropdown>
  );
};

export default CreativeAssetActionsMenu;
