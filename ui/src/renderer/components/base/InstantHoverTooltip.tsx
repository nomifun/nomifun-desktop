/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { autoUpdate, flip, FloatingPortal, offset, shift, useFloating } from '@floating-ui/react';
import classNames from 'classnames';
import React, { useEffect, useState } from 'react';

type InstantHoverTooltipProps = {
  content: React.ReactNode;
  children: React.ReactNode;
  position?: 'top' | 'right' | 'bottom' | 'left';
  className?: string;
  tooltipClassName?: string;
  disabled?: boolean;
};

const InstantHoverTooltip: React.FC<InstantHoverTooltipProps> = ({
  content,
  children,
  position = 'top',
  className,
  tooltipClassName,
  disabled = false,
}) => {
  const [visible, setVisible] = useState(false);
  const shown = visible && !disabled;
  const { refs, floatingStyles, isPositioned, placement } = useFloating({
    open: shown,
    placement: position,
    strategy: 'fixed',
    whileElementsMounted: autoUpdate,
    middleware: [offset(position === 'left' || position === 'right' ? 8 : 6), flip({ padding: 8 }), shift({ padding: 8 })],
  });

  useEffect(() => {
    if (disabled) setVisible(false);
  }, [disabled]);

  return (
    <div
      ref={refs.setReference}
      className={classNames('relative inline-flex shrink-0', className)}
      onMouseEnter={() => setVisible(!disabled)}
      onMouseLeave={() => setVisible(false)}
      onFocus={() => setVisible(!disabled)}
      onBlur={() => setVisible(false)}
    >
      {children}
      {/* Escape scroll containers and stacking contexts in the trigger's pane. */}
      {shown && (
        <FloatingPortal>
          <span
            ref={refs.setFloating}
            role='tooltip'
            data-placement={placement}
            style={{ ...floatingStyles, opacity: isPositioned ? 1 : 0 }}
            className={classNames(
              'pointer-events-none z-[10000] box-border w-max max-w-[calc(100vw-16px)] whitespace-normal break-words rd-6px bg-[var(--color-tooltip-bg)] px-8px py-5px text-12px font-500 leading-none text-white shadow-[0_6px_18px_rgba(0,0,0,0.18)] transition-opacity duration-75',
              tooltipClassName
            )}
          >
            {content}
          </span>
        </FloatingPortal>
      )}
    </div>
  );
};

export default InstantHoverTooltip;
