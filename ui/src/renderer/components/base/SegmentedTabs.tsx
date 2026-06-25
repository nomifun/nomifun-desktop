/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import classNames from 'classnames';
import React from 'react';

export interface SegmentedTabItem {
  key: string;
  label: React.ReactNode;
  icon?: React.ReactNode;
}

interface SegmentedTabsProps {
  items: SegmentedTabItem[];
  activeKey: string;
  onChange: (key: string) => void;
  className?: string;
  /** Size of the control. `md` (default) suits page-level primary tabs. */
  size?: 'sm' | 'md';
}

/**
 * SegmentedTabs — a polished pill/segmented control for primary-level tab
 * switching. The active segment lifts onto a card surface with a soft shadow;
 * inactive segments are quiet text that brightens on hover. Themed entirely
 * through CSS variables so it tracks light/dark and the preset palettes.
 *
 * Overflows horizontally (with hidden scrollbar) so the bar stays usable as
 * more sections are added over time.
 */
const SegmentedTabs: React.FC<SegmentedTabsProps> = ({ items, activeKey, onChange, className, size = 'md' }) => {
  const heightClass = size === 'sm' ? 'h-30px px-12px text-13px' : 'h-36px px-16px text-14px';
  return (
    <div
      role='tablist'
      className={classNames(
        'inline-flex items-center gap-2px p-4px rounded-12px bg-[var(--color-fill-2)] max-w-full overflow-x-auto scrollbar-hide',
        className
      )}
    >
      {items.map((item) => {
        const active = item.key === activeKey;
        return (
          <button
            key={item.key}
            type='button'
            role='tab'
            aria-selected={active}
            onClick={() => onChange(item.key)}
            className={classNames(
              'group relative shrink-0 inline-flex items-center justify-center gap-6px rounded-9px font-[500] leading-none cursor-pointer transition-all duration-200 select-none border-none outline-none bg-transparent',
              heightClass,
              active
                ? '!bg-primary-1 !text-primary-6 shadow-[0_1px_2px_rgba(0,0,0,0.05),0_2px_8px_rgba(0,0,0,0.06)]'
                : 'text-t-secondary hover:text-t-primary'
            )}
          >
            {item.icon && (
              <span
                className={classNames(
                  'flex items-center justify-center transition-colors line-height-0',
                  active ? 'text-primary-6' : 'text-t-tertiary group-hover:text-t-secondary'
                )}
              >
                {item.icon}
              </span>
            )}
            <span className='whitespace-nowrap'>{item.label}</span>
          </button>
        );
      })}
    </div>
  );
};

export default SegmentedTabs;
