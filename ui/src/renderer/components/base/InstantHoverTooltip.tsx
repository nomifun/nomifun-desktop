/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import classNames from 'classnames';
import React, { useState } from 'react';

type InstantHoverTooltipProps = {
  content: React.ReactNode;
  children: React.ReactNode;
  position?: 'top' | 'right' | 'bottom';
  className?: string;
};

const positionClassName: Record<NonNullable<InstantHoverTooltipProps['position']>, string> = {
  top: 'left-1/2 bottom-[calc(100%+6px)] -translate-x-1/2',
  right: 'left-[calc(100%+8px)] top-1/2 -translate-y-1/2',
  bottom: 'left-1/2 top-[calc(100%+6px)] -translate-x-1/2',
};

const InstantHoverTooltip: React.FC<InstantHoverTooltipProps> = ({
  content,
  children,
  position = 'top',
  className,
}) => {
  const [visible, setVisible] = useState(false);

  return (
    <div
      className={classNames('relative inline-flex shrink-0', className)}
      onMouseEnter={() => setVisible(true)}
      onMouseLeave={() => setVisible(false)}
      onFocus={() => setVisible(true)}
      onBlur={() => setVisible(false)}
    >
      {children}
      <span
        role='tooltip'
        aria-hidden={!visible}
        className={classNames(
          'pointer-events-none absolute z-[10000] whitespace-nowrap rd-6px bg-[#1f2329] px-8px py-5px text-12px font-500 leading-none text-white shadow-[0_6px_18px_rgba(0,0,0,0.18)] transition-opacity duration-75',
          positionClassName[position],
          visible ? 'visible opacity-100' : 'invisible opacity-0'
        )}
      >
        {content}
      </span>
    </div>
  );
};

export default InstantHoverTooltip;
