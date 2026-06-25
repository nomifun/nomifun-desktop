/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { Components, DatabaseConfig, ListView } from '@icon-park/react';

/** The three top-level sections of the requirements platform shell. */
export type RequirementsSection = 'workspace' | 'extensions' | 'sources';

export interface RequirementsSectionDef {
  key: RequirementsSection;
  /** Localized rail label. */
  label: string;
  /** Rail icon. */
  icon: React.ReactNode;
  /** Absolute route path the rail item navigates to. */
  path: string;
}

/**
 * The requirements-platform section definitions, in rail order.
 *
 * Each section is a real nested route under `/requirements`, so `path` is an
 * absolute pathname (the shell rail navigates to it and derives the active
 * section from the current location). `workspace` is the index route, so its
 * path is the bare `/requirements`.
 */
export const useRequirementsSections = (): RequirementsSectionDef[] => {
  const { t } = useTranslation();
  return [
    {
      key: 'workspace',
      label: t('requirements.section.workspace'),
      icon: <ListView theme='outline' size='16' strokeWidth={3} />,
      path: '/requirements',
    },
    {
      key: 'extensions',
      label: t('requirements.section.extensions'),
      icon: <Components theme='outline' size='16' strokeWidth={3} />,
      path: '/requirements/extensions',
    },
    {
      key: 'sources',
      label: t('requirements.section.sources'),
      icon: <DatabaseConfig theme='outline' size='16' strokeWidth={3} />,
      path: '/requirements/sources',
    },
  ];
};
