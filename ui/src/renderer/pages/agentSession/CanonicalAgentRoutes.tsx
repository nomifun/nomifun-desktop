import AppLoader from '@/renderer/components/layout/AppLoader';
import RouteErrorBoundary from '@/renderer/components/layout/RouteErrorBoundary';
import React, { Suspense } from 'react';
import { HashRouter, Route, Routes, useLocation } from 'react-router-dom';

const AgentSettingsPage = React.lazy(() => import('@/renderer/pages/agentSettings'));
const AgentSessionPage = React.lazy(() => import('./AgentSessionPage'));

export const isCanonicalAgentHashRoute = (hash: string): boolean => {
  const path = hash.replace(/^#/, '').split('?')[0];
  return (
    path === '/settings/agent-presets' ||
    path.startsWith('/settings/agent-presets/') ||
    /^\/agent-sessions\/[0-9a-f-]+$/.test(path)
  );
};

const RouteView: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const location = useLocation();
  return (
    <RouteErrorBoundary resetKey={`${location.pathname}${location.search}`}>
      <Suspense fallback={<AppLoader />}>{children}</Suspense>
    </RouteErrorBoundary>
  );
};

const CanonicalAgentRoutes: React.FC<{ layout: React.ReactElement }> = ({ layout }) => (
  <HashRouter>
    <Routes>
      <Route element={layout}>
        <Route
          path='/settings/agent-presets/*'
          element={
            <RouteView>
              <AgentSettingsPage />
            </RouteView>
          }
        />
        <Route
          path='/agent-sessions/:agentSessionId'
          element={
            <RouteView>
              <AgentSessionPage />
            </RouteView>
          }
        />
      </Route>
    </Routes>
  </HashRouter>
);

export default CanonicalAgentRoutes;
