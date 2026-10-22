/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useLocation, useNavigate } from 'react-router-dom';
import ResourceSpacePage from './ResourceSpacePage';
import { MATERIALS_PATH, TEMPLATES_PATH, resourceSpaceSectionForPath } from './resourceRoutes';

const Assets = React.lazy(() => import('../assets/page/CreativeAssetLibraryPage'));
const Templates = React.lazy(() => import('../templates/page/CreativeTemplateRoute'));

/** Existing deep links share one mounted shell; prompts stay on their own route. */
export default function ResourceSpaceRoute() {
  const { pathname } = useLocation();
  const navigate = useNavigate();
  return <ResourceSpacePage
    section={resourceSpaceSectionForPath(pathname) ?? 'assets'}
    onSectionChange={(section) => void navigate(section === 'assets' ? MATERIALS_PATH : TEMPLATES_PATH)}
    assets={<Assets />}
    templates={<Templates />}
  />;
}
