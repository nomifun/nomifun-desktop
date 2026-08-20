/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback } from 'react';
import { useNavigate } from 'react-router-dom';

import { creativeStudioCanvasProjectPath } from '../app/routes';
import CreativeStudioProjectsPage from './CreativeStudioProjectsPage';
import type { CreativeStudioProjectSummary } from './types';

const CreativeStudioProjectsRoute: React.FC = () => {
  const navigate = useNavigate();
  const openProject = useCallback(
    (project: CreativeStudioProjectSummary) => {
      navigate(creativeStudioCanvasProjectPath(project.id));
    },
    [navigate]
  );

  return <CreativeStudioProjectsPage onOpenProject={openProject} />;
};

export default CreativeStudioProjectsRoute;
