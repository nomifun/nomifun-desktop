/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Pagination } from '@arco-design/web-react';
import type { PaginationProps } from '@arco-design/web-react';
import classNames from 'classnames';
import React from 'react';

/** Shared visual contract for Arco and bespoke pagination surfaces. */
export const NOMI_PAGINATION_CLASS_NAME = 'nomi-pagination';

export type NomiPaginationProps = Omit<PaginationProps, 'mini' | 'size'>;

/**
 * Product pagination. Its size and theme treatment are intentionally fixed so
 * list pages cannot drift back to Arco's saturated active-page styling.
 */
const NomiPagination: React.FC<NomiPaginationProps> = ({ className, ...props }) => (
  <Pagination
    {...props}
    className={classNames(NOMI_PAGINATION_CLASS_NAME, className)}
    size='default'
  />
);

export default NomiPagination;
