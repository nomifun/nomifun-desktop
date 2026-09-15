/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback } from 'react';
import { useNavigate } from 'react-router-dom';

import { canvasPath } from '../app/resourceRoutes';
import type { CreativeCanvasSummary } from '../domain';
import CreativeStudioCanvasesPage from './CreativeStudioCanvasesPage';

const CreativeStudioCanvasesRoute: React.FC = () => {
  const navigate = useNavigate();
  const openCanvas = useCallback(
    (canvas: CreativeCanvasSummary) => {
      navigate(canvasPath(canvas.canvasId));
    },
    [navigate]
  );

  return <CreativeStudioCanvasesPage onOpenCanvas={openCanvas} />;
};

export default CreativeStudioCanvasesRoute;
