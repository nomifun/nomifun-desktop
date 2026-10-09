/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import classNames from 'classnames';
import React from 'react';
import PageHeader, { PAGE_TITLE_CLASS } from './PageHeader';

interface HubPageShellProps {
  title: string;
  subtitle?: string;
  /** Optional scope hook for page-specific visual contracts. */
  className?: string;
  /** Tailwind max-width class for the centered content column. */
  maxWidthClass?: string;
  /** Rendered between the header and the body (e.g. a segmented tab bar). */
  toolbar?: React.ReactNode;
  /** Page-level actions aligned to the right of the visible heading. */
  actions?: React.ReactNode;
  /** Dense workbenches provide their visible title inside the navigation pane. */
  hideHeading?: boolean;
  children: React.ReactNode;
}

/** Shared visual contract for top-level destinations opened from the app rail. */
export const HUB_PAGE_TITLE_CLASS = PAGE_TITLE_CLASS;

/**
 * HubPageShell — shared chrome for the homepage "hub" destinations (Model
 * Management, Presets, Skills, MCP). Mirrors the scroll container + centered content
 * column of `SettingsPageWrapper` so the embedded settings content components lay
 * out correctly.
 */
const HubPageShell: React.FC<HubPageShellProps> = ({
  title,
  subtitle,
  className,
  maxWidthClass = 'md:max-w-1100px',
  toolbar,
  actions,
  hideHeading = false,
  children,
}) => {
  return (
    <div
      className={classNames(
        'w-full min-h-full box-border overflow-y-auto',
        className,
        'px-24px md:px-32px py-24px'
      )}
    >
      <div className={classNames('mx-auto w-full', maxWidthClass)}>
        {hideHeading ? (
          <h1 className='sr-only'>{title}</h1>
        ) : (
          <PageHeader title={title} description={subtitle} actions={actions} />
        )}
        {toolbar && <div className='mb-20px'>{toolbar}</div>}
        {children}
      </div>
    </div>
  );
};

export default HubPageShell;
