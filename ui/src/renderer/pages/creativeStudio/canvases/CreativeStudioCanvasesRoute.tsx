/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';

import { preloadResourceRoute } from '@/renderer/components/layout/routePreload';
import { canvasPath } from '../app/resourceRoutes';
import type { CreativeCanvasSummary } from '../domain';
import { preloadCreativeProject } from '../services';
import CreativeStudioCanvasesPage from './CreativeStudioCanvasesPage';

const CreativeStudioCanvasesRoute: React.FC = () => {
  const navigate = useNavigate();
  const { t } = useTranslation();
  const openingCanvasIdRef = useRef<string | null>(null);
  const [openingCanvasId, setOpeningCanvasId] = useState<string | null>(null);
  const warmCanvas = useCallback((canvas: CreativeCanvasSummary) => {
    void preloadResourceRoute(canvasPath(canvas.canvasId));
    void preloadCreativeProject(canvas.canvasId).catch(() => undefined);
  }, []);

  // The Canvas editor chunk is intentionally split (it is much larger than
  // the library). Warm it after the lightweight list has painted, while hover
  // and keyboard focus still provide an immediate intent-based fast path.
  useEffect(() => {
    const timer = window.setTimeout(() => {
      void preloadResourceRoute(canvasPath('__route-warmup__'));
    }, 500);
    return () => window.clearTimeout(timer);
  }, []);

  const openCanvas = useCallback(
    (canvas: CreativeCanvasSummary) => {
      if (openingCanvasIdRef.current) return;
      openingCanvasIdRef.current = canvas.canvasId;
      setOpeningCanvasId(canvas.canvasId);
      warmCanvas(canvas);
      navigate(canvasPath(canvas.canvasId), {
        state: {
          routeLoadingLabel: t('creativeStudio.canvases.openingCanvas', {
            title: canvas.title,
            defaultValue: '正在打开“{{title}}”…',
          }),
          canvasSummary: canvas,
        },
      });
    },
    [navigate, t, warmCanvas]
  );

  return (
    <CreativeStudioCanvasesPage
      openingCanvasId={openingCanvasId}
      onPrefetchCanvas={warmCanvas}
      onOpenCanvas={openCanvas}
    />
  );
};

export default CreativeStudioCanvasesRoute;
