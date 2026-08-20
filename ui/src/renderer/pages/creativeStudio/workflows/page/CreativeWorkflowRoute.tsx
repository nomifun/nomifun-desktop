/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useNavigate } from 'react-router-dom';

import CreativeWorkflowWorkspacePage from './CreativeWorkflowWorkspacePage';

const CreativeWorkflowRoute: React.FC = () => {
  const navigate = useNavigate();
  return <CreativeWorkflowWorkspacePage onOpenModelSettings={() => void navigate('/models')} />;
};

export default CreativeWorkflowRoute;
