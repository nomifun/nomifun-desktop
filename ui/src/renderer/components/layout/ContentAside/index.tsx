/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useEffect } from 'react';
import classNames from 'classnames';
import { Close } from '@icon-park/react';
import { useResizableSplit } from '@/renderer/hooks/ui/useResizableSplit';

export interface ContentAsideProps {
  /** Panel heading. */
  title: React.ReactNode;
  /** Optional one-line context under the title. */
  subtitle?: React.ReactNode;
  /** Actions rendered left of the close button. */
  actions?: React.ReactNode;
  /** Called when the user closes the panel (button or Escape). */
  onClose: () => void;
  /** LocalStorage key for the persisted width. */
  storageKey: string;
  defaultWidth?: number;
  minWidth?: number;
  maxWidth?: number;
  children: React.ReactNode;
  className?: string;
}

/**
 * ContentAside — an on-demand detail pane docked to the right of a content-area
 * workspace.
 *
 * The counterpart to `ContentSider`: where the sider is a permanent recessed
 * navigation surface, this is a *transient* raised one. It reads as a card
 * floating above the workspace (rounded, hairlined, inset by a gutter) precisely
 * because it comes and goes — a recessed flush panel would imply permanence.
 *
 * The drag handle sits on the panel's LEFT edge (outside its own box) so the
 * grab target lives in the gutter between workspace and panel rather than on
 * the panel's border.
 *
 * Width is persisted per `storageKey`; double-clicking the handle restores the
 * default. Escape closes, matching every other dismissible surface in the app.
 */
const ContentAside: React.FC<ContentAsideProps> = ({
  title,
  subtitle,
  actions,
  onClose,
  storageKey,
  defaultWidth = 360,
  minWidth = 280,
  maxWidth = 620,
  children,
  className,
}) => {
  const resize = useResizableSplit({ unit: 'px', defaultWidth, minWidth, maxWidth, storageKey });

  const handleKeyDown = useCallback(
    (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose();
    },
    [onClose]
  );

  useEffect(() => {
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [handleKeyDown]);

  return (
    <aside
      // The gutter is asymmetric on purpose: a wider left inset separates the
      // panel from the workspace it annotates, a narrow right inset keeps it
      // visually docked to the window edge rather than floating free.
      className={classNames(
        'relative shrink-0 flex flex-col min-h-0 my-12px mr-12px ml-8px rd-15px border border-solid border-3 bg-[var(--color-bg-2)] overflow-hidden',
        className
      )}
      style={{ width: resize.splitRatio, minWidth }}
    >
      {resize.createDragHandle({ className: '-left-20px', reverse: true, linePlacement: 'end' })}
      <div className='shrink-0 flex items-start gap-8px px-14px pt-12px pb-10px'>
        <div className='min-w-0 flex-1'>
          <div className='text-14px leading-20px font-600 text-t-primary truncate'>{title}</div>
          {subtitle && <div className='mt-2px text-12px leading-18px text-t-tertiary truncate'>{subtitle}</div>}
        </div>
        {actions && <div className='shrink-0 flex items-center gap-6px'>{actions}</div>}
        {/* A real <button> would paint a black focus box in WebView2 — the app
            uses role='button' divs for custom clickables throughout. */}
        <div
          role='button'
          tabIndex={0}
          aria-label='close'
          onClick={onClose}
          onKeyDown={(event) => {
            if (event.key === 'Enter' || event.key === ' ') onClose();
          }}
          className='shrink-0 flex items-center justify-center w-22px h-22px rd-6px text-t-tertiary hover:text-t-primary hover:bg-[var(--color-fill-2)] transition-colors cursor-pointer'
        >
          <Close theme='outline' size='14' fill='currentColor' />
        </div>
      </div>
      <div className='flex-1 min-h-0 overflow-y-auto px-14px pb-14px'>{children}</div>
    </aside>
  );
};

export default ContentAside;
