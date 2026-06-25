/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import classNames from 'classnames';

type FigureActionTone = 'primary' | 'danger';

export const FigureActionVeil: React.FC<{ className?: string }> = ({ className }) => (
  <div
    aria-hidden='true'
    className={classNames(
      'pointer-events-none absolute inset-x-0 top-0 z-1 h-56px bg-gradient-to-b from-[rgba(15,23,42,0.24)] to-transparent opacity-0 transition-opacity duration-150 group-hover:opacity-100 group-focus-within:opacity-100',
      className
    )}
  />
);

export const FigureActionSurface: React.FC<{
  children: React.ReactNode;
  className?: string;
}> = ({ children, className }) => (
  <div
    onClick={(e) => e.stopPropagation()}
    className={classNames(
      'figure-card-action-surface absolute right-8px top-8px z-3 flex items-center gap-2px p-2px rd-8px border border-solid border-[rgba(255,255,255,0.24)] bg-[rgba(15,23,42,0.68)] shadow-[0_8px_20px_rgba(15,23,42,0.24)] backdrop-blur-md opacity-0 pointer-events-none transition-all duration-150 group-hover:opacity-100 group-hover:pointer-events-auto group-focus-within:opacity-100 group-focus-within:pointer-events-auto',
      className
    )}
  >
    {children}
  </div>
);

export const FigureActionButton: React.FC<{
  title: string;
  ariaLabel: string;
  tone: FigureActionTone;
  disabled?: boolean;
  onClick?: React.MouseEventHandler<HTMLButtonElement>;
  children: React.ReactNode;
}> = ({ title, ariaLabel, tone, disabled, onClick, children }) => (
  <button
    type='button'
    disabled={disabled}
    title={title}
    aria-label={ariaLabel}
    onClick={onClick}
    className={classNames(
      'figure-card-action-button flex items-center justify-center w-24px h-24px rd-6px border-none b-none bg-transparent p-0 leading-none appearance-none outline-none text-[rgba(255,255,255,0.92)] cursor-pointer transition-colors duration-150 hover:bg-[rgba(255,255,255,0.16)] focus-visible:bg-[rgba(255,255,255,0.18)] focus-visible:outline-none disabled:cursor-not-allowed disabled:opacity-45',
      tone === 'primary'
        ? 'hover:!text-[rgb(var(--primary-6))] focus-visible:!text-[rgb(var(--primary-6))]'
        : 'hover:!text-[rgb(var(--danger-6))] focus-visible:!text-[rgb(var(--danger-6))]'
    )}
  >
    {children}
  </button>
);
