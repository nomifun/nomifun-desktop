/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import classNames from 'classnames';
import React from 'react';

export type SkillButtonTone = 'primary' | 'quiet' | 'danger';

interface SkillButtonProps {
  onClick: () => void;
  tone?: SkillButtonTone;
  size?: 'sm' | 'md';
  icon?: React.ReactNode;
  disabled?: boolean;
  className?: string;
  children: React.ReactNode;
}

const TONES: Record<SkillButtonTone, string> = {
  // The app's best CTA treatment: a soft primary wash, never a saturated fill.
  primary:
    'bg-[rgba(var(--primary-6),0.12)] text-[var(--color-text-1)] font-700 shadow-[0_6px_18px_rgba(var(--primary-6),0.14)] hover:bg-[rgba(var(--primary-6),0.18)]',
  quiet:
    'bg-[var(--color-bg-2)] border border-solid border-[var(--color-border-2)] text-t-secondary hover:text-t-primary hover:bg-fill-2',
  danger: 'text-danger-6 hover:bg-[rgba(var(--danger-6),0.1)]',
};

/**
 * A pill action. `role='button'` on a div rather than a real `<button>`: WebView2
 * paints a black focus box on native buttons, which this app avoids everywhere.
 */
const SkillButton: React.FC<SkillButtonProps> = ({
  onClick,
  tone = 'quiet',
  size = 'sm',
  icon,
  disabled = false,
  className,
  children,
}) => (
  <div
    role='button'
    tabIndex={disabled ? -1 : 0}
    aria-disabled={disabled}
    onClick={(event) => {
      event.stopPropagation();
      if (!disabled) onClick();
    }}
    onKeyDown={(event) => {
      if (event.key !== 'Enter' && event.key !== ' ') return;
      event.preventDefault();
      event.stopPropagation();
      if (!disabled) onClick();
    }}
    className={classNames(
      'inline-flex shrink-0 select-none items-center gap-4px rd-full outline-none transition-colors',
      size === 'sm' ? 'px-10px py-4px text-12px' : 'px-18px py-9px text-13px',
      disabled ? 'cursor-not-allowed opacity-50' : 'cursor-pointer',
      TONES[tone],
      className
    )}
  >
    {icon}
    {children}
  </div>
);

export default SkillButton;
