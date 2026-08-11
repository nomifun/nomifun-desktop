/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import classNames from 'classnames';

interface RowActionProps {
  onClick: () => void;
  /**
   * Renders the pressed/open treatment (e.g. the detail pane it owns is open).
   * Supplying it at all marks this action as a toggle (`aria-pressed`); leave it
   * undefined for one-shot actions so no toggle state is announced.
   */
  active?: boolean;
  /** Drop the outline for inline, low-weight actions such as “reset”. */
  quiet?: boolean;
  className?: string;
  children: React.ReactNode;
}

/**
 * The tab's settings-row action button. A `role='button'` div rather than a real
 * `<button>`: WebView2 paints a black focus box on native buttons, which the app
 * avoids everywhere.
 */
const RowAction: React.FC<RowActionProps> = ({ onClick, active, quiet = false, className, children }) => (
  <div
    role='button'
    tabIndex={0}
    aria-pressed={active}
    onClick={onClick}
    onKeyDown={(event) => {
      if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        onClick();
      }
    }}
    className={classNames(
      'shrink-0 flex items-center gap-5px rd-8px cursor-pointer select-none transition-colors',
      quiet ? 'px-8px py-3px text-12px' : 'px-12px py-6px text-13px font-500',
      !quiet && 'border border-solid border-[var(--color-border-2)]',
      active ? '!bg-primary-1 !text-primary-6' : 'text-t-secondary hover:bg-fill-2 active:bg-fill-3',
      className
    )}
  >
    {children}
  </div>
);

export default RowAction;
