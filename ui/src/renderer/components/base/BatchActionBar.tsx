/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import classNames from 'classnames';
import React from 'react';

type BatchAction = {
  key: string;
  label: string;
  onClick: () => void;
  danger?: boolean;
  disabled?: boolean;
};

type BatchActionBarProps = {
  selectAllLabel: string;
  onSelectAll: () => void;
  actions: BatchAction[];
  className?: string;
};

/**
 * Shared presentational batch action bar. Renders a row with a "select all"
 * link on the left and action links on the right. Matches the terminal section
 * visual pattern: no background, no border, text links only.
 */
const BatchActionBar: React.FC<BatchActionBarProps> = ({ selectAllLabel, onSelectAll, actions, className }) => {
  return (
    <div className={classNames('flex items-center justify-between px-12px py-6px mt-4px', className)}>
      <span className='text-12px text-t-secondary cursor-pointer hover:text-t-primary' onClick={onSelectAll}>
        {selectAllLabel}
      </span>
      <span className='flex items-center gap-12px'>
        {actions.map((action) => (
          <span
            key={action.key}
            className={classNames('text-12px', {
              'text-t-tertiary cursor-not-allowed': action.disabled,
              'text-danger cursor-pointer hover:opacity-80': !action.disabled && action.danger,
              'text-t-secondary cursor-pointer hover:text-t-primary': !action.disabled && !action.danger,
            })}
            onClick={() => {
              if (!action.disabled) {
                action.onClick();
              }
            }}
          >
            {action.label}
          </span>
        ))}
      </span>
    </div>
  );
};

export default BatchActionBar;
