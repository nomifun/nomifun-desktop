/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { Navigate } from 'react-router-dom';

export interface CreativeStudioRouteRedirectProps {
  to: string;
}

/** Explicit redirect boundary for leaf routes that are intentionally canonicalized by the Router. */
export const CreativeStudioRouteRedirect: React.FC<CreativeStudioRouteRedirectProps> = ({ to }) => (
  <Navigate to={to} replace />
);

/**
 * Transitional zero-content index boundary.
 *
 * The project-list slice replaces this component at `/workshop`; keeping this
 * boundary empty prevents a fabricated landing experience from flashing while
 * independently developed route slices are connected.
 */
const CreativeStudioHomePage: React.FC = () => null;

export default CreativeStudioHomePage;
