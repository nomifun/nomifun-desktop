/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';

interface ReaderPanelProps {
  /** Sticky header content — stays visible while the day scrolls under it. */
  header: React.ReactNode;
  children: React.ReactNode;
}

/**
 * The reading surface: one bounded panel with a sticky header.
 *
 * Deliberately no `overflow-hidden` — that would turn the panel into a scrollport
 * and silently kill the sticky header. The header therefore carries the panel's
 * own opaque background so nothing shows through as content slides beneath it.
 */
const ReaderPanel: React.FC<ReaderPanelProps> = ({ header, children }) => (
  <div className='flex min-w-0 flex-col rd-12px border border-solid border-[var(--color-border-2)] bg-[var(--color-bg-2)]'>
    <div className='sticky top-0 z-2 flex items-center gap-8px rounded-t-12px border-b border-solid border-[var(--color-border-2)] border-l-0 border-r-0 border-t-0 bg-[var(--color-bg-2)] px-16px py-10px'>
      {header}
    </div>
    <div className='flex min-w-0 flex-col gap-14px px-16px py-14px'>{children}</div>
  </div>
);

export default ReaderPanel;
