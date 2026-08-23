/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { Navigate, useLocation } from 'react-router-dom';

import { CREATIVE_STUDIO_CANVASES_PATH } from '../app/routes';

/** @deprecated `/workshop/projects` is a redirect, never a product surface. */
const CreativeStudioProjectsRoute: React.FC = () => {
  const { search, hash } = useLocation();
  return (
    <Navigate
      to={`${CREATIVE_STUDIO_CANVASES_PATH}${search}${hash}`}
      replace
    />
  );
};

export default CreativeStudioProjectsRoute;
