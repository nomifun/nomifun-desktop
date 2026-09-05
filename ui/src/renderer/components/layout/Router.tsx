import React, { Suspense } from 'react';
import { HashRouter, Navigate, Route, Routes, useLocation, useParams } from 'react-router-dom';
import AppLoader from '@renderer/components/layout/AppLoader';
import ProtectedAppRuntime from '@renderer/components/layout/ProtectedAppRuntime';
import RouteErrorBoundary from '@renderer/components/layout/RouteErrorBoundary';
import { useAuth } from '@renderer/hooks/context/AuthContext';
import {
  CREATIVE_STUDIO_CANVASES_PATH,
  CREATIVE_STUDIO_ROOT_PATH,
  creativeStudioSectionForPath,
  type CreativeStudioSection,
} from '@renderer/pages/creativeStudio/app/routes';
const Conversation = React.lazy(() => import('@renderer/pages/conversation'));
const Guid = React.lazy(() => import('@renderer/pages/guid'));
const PresetSettings = React.lazy(() => import('@renderer/pages/settings/PresetSettings'));
const SkillsSettingsPage = React.lazy(() => import('@renderer/pages/settings/SkillsSettingsPage'));
const ModelHubPage = React.lazy(() => import('@renderer/pages/modelHub'));
const McpPage = React.lazy(() => import('@renderer/pages/mcp'));
const OpenCapabilitiesPage = React.lazy(() => import('@renderer/pages/openCapabilities'));
const BrowserPage = React.lazy(() => import('@renderer/pages/browser'));
const SystemSettings = React.lazy(() => import('@renderer/pages/settings/SystemSettings'));
const ExecutionEngineSettings = React.lazy(() => import('@renderer/pages/settings/AgentSettings'));
const SshHostSettings = React.lazy(() => import('@renderer/pages/settings/SshHostSettings'));
const ExtensionSettingsPage = React.lazy(() => import('@renderer/pages/settings/ExtensionSettingsPage'));
const ComputerHistorySettings = React.lazy(() => import('@renderer/pages/settings/ComputerHistorySettings'));
const LoginPage = React.lazy(() => import('@renderer/pages/login'));
const ComponentsShowcase = React.lazy(() => import('@renderer/pages/TestShowcase'));
const ScheduledTasksPage = React.lazy(() => import('@renderer/pages/cron/ScheduledTasksPage'));
const TaskDetailPage = React.lazy(() => import('@renderer/pages/cron/ScheduledTasksPage/TaskDetailPage'));
const RequirementsLayout = React.lazy(() => import('@renderer/pages/requirements/RequirementsLayout'));
const WorkspacePage = React.lazy(() => import('@renderer/pages/requirements/WorkspacePage'));
const ExtensionsPage = React.lazy(() => import('@renderer/pages/requirements/ExtensionsPage'));
const SourcesPage = React.lazy(() => import('@renderer/pages/requirements/SourcesPage'));
const TerminalSessionPage = React.lazy(() => import('@renderer/pages/terminal/TerminalSessionPage'));
const TerminalCreatePage = React.lazy(() => import('@renderer/pages/terminal/TerminalCreatePage'));
const NomiConfigPage = React.lazy(() => import('@renderer/pages/nomi'));
const CustomerServiceRosterPage = React.lazy(() => import('@renderer/pages/customerService'));
const CustomerServiceDetailPage = React.lazy(() => import('@renderer/pages/customerService/CsAgentDetailPage'));
const KnowledgeListPage = React.lazy(() => import('@renderer/pages/knowledge/KnowledgeListPage'));
const KnowledgeDetailPage = React.lazy(() => import('@renderer/pages/knowledge/KnowledgeDetailPage'));
const loadCreativeStudioFocusShell = () =>
  import('@renderer/pages/creativeStudio/app/CreativeStudioFocusShell');
const loadCreativeStudioCanvasesRoute = () =>
  import('@renderer/pages/creativeStudio/canvases/CreativeStudioCanvasesRoute');
const loadCreativeStudioPromptsRoute = () =>
  import('@renderer/pages/creativeStudio/prompts/page/CreativeStudioPromptsRoute');
const loadCreativeStudioAssetsRoute = () =>
  import('@renderer/pages/creativeStudio/assets/page/CreativeAssetLibraryPage');
const loadCreativeStudioCanvasRoute = () =>
  import('@renderer/pages/creativeStudio/canvases/CreativeCanvasProductRoute');
const loadCreativeStudioWorkbenches = () =>
  import('@renderer/pages/creativeStudio/workbenches/product');
const loadCreativeStudioImageWorkbenchRoute = () =>
  loadCreativeStudioWorkbenches().then((module) => ({
    default: module.ImageWorkbenchProductRoute,
  }));
const loadCreativeStudioVideoWorkbenchRoute = () =>
  loadCreativeStudioWorkbenches().then((module) => ({
    default: module.VideoWorkbenchProductRoute,
  }));
const loadCreativeStudioDirectorRoute = () =>
  import('@renderer/pages/creativeStudio/canvases/CreativeCanvasDirectorRoute');
const loadCreativeStudioTemplateRoute = () =>
  import('@renderer/pages/creativeStudio/templates/page/CreativeTemplateRoute');

const creativeStudioRouteLoaders: Record<CreativeStudioSection, () => Promise<unknown>> = {
  canvases: loadCreativeStudioCanvasesRoute,
  canvas: loadCreativeStudioCanvasRoute,
  director: loadCreativeStudioDirectorRoute,
  image: loadCreativeStudioImageWorkbenchRoute,
  video: loadCreativeStudioVideoWorkbenchRoute,
  prompts: loadCreativeStudioPromptsRoute,
  assets: loadCreativeStudioAssetsRoute,
  templates: loadCreativeStudioTemplateRoute,
};

const ignoreCreativeStudioPreloadFailure = (preload: Promise<unknown>): Promise<void> =>
  preload.then(
    () => undefined,
    () => undefined
  );

/**
 * Warms the focused product shell and the exact lazy route needed for a
 * Creative Studio destination. Preloading remains best-effort: route errors
 * still render through the normal route error boundary when the user navigates.
 */
export const preloadCreativeStudioRoute = (path: string): Promise<void> => {
  const section = creativeStudioSectionForPath(path);
  const loader = section ? creativeStudioRouteLoaders[section] : null;
  if (!loader) return Promise.resolve();

  return ignoreCreativeStudioPreloadFailure(
    Promise.all([loadCreativeStudioFocusShell(), loader()])
  );
};

/** Preload the sections exposed by the Creative Studio product sidebar at idle. */
export const preloadCreativeStudioNavigationRoutes = (): Promise<void> =>
  ignoreCreativeStudioPreloadFailure(
    Promise.all([
      loadCreativeStudioFocusShell(),
      loadCreativeStudioCanvasesRoute(),
      loadCreativeStudioWorkbenches(),
      loadCreativeStudioPromptsRoute(),
      loadCreativeStudioAssetsRoute(),
      loadCreativeStudioTemplateRoute(),
    ])
  );

const CreativeStudioFocusShell = React.lazy(loadCreativeStudioFocusShell);
const CreativeStudioCanvasesRoute = React.lazy(loadCreativeStudioCanvasesRoute);
const CreativeStudioPromptsRoute = React.lazy(loadCreativeStudioPromptsRoute);
const CreativeStudioAssetsRoute = React.lazy(loadCreativeStudioAssetsRoute);
const CreativeStudioCanvasRoute = React.lazy(loadCreativeStudioCanvasRoute);
const CreativeStudioImageWorkbenchRoute = React.lazy(loadCreativeStudioImageWorkbenchRoute);
const CreativeStudioVideoWorkbenchRoute = React.lazy(loadCreativeStudioVideoWorkbenchRoute);
const CreativeStudioDirectorRoute = React.lazy(loadCreativeStudioDirectorRoute);
const CreativeStudioTemplateRoute = React.lazy(loadCreativeStudioTemplateRoute);
const MiniAppsListPage = React.lazy(() => import('@renderer/pages/miniApps'));
const MiniAppRunnerPage = React.lazy(() => import('@renderer/pages/miniApps/RunnerPage'));
const CompanionPage = React.lazy(() => import('@renderer/pages/companion'));
const ConversationShell = React.lazy(() => import('@renderer/pages/conversation/components/ConversationShell'));

const RouteFallback: React.FC<{ Component: React.LazyExoticComponent<React.ComponentType> }> = ({ Component }) => {
  const location = useLocation();
  const resetKey = `${location.pathname}${location.search}${location.hash}`;

  return (
    <RouteErrorBoundary resetKey={resetKey}>
      <Suspense fallback={<AppLoader />}>
        <Component />
      </Suspense>
    </RouteErrorBoundary>
  );
};

const withRouteFallback = (Component: React.LazyExoticComponent<React.ComponentType>) => (
  <RouteFallback Component={Component} />
);

const SessionShellRoute: React.FC = () => {
  const location = useLocation();
  const resetKey = `${location.pathname}${location.search}${location.hash}`;

  return (
    <RouteErrorBoundary resetKey={resetKey}>
      <Suspense fallback={<AppLoader />}>
        <ConversationShell />
      </Suspense>
    </RouteErrorBoundary>
  );
};

const withSearch = (path: string, searchParams: URLSearchParams) => {
  const search = searchParams.toString();
  return search ? `${path}?${search}` : path;
};

/** Preserve local/remote tab deep links from the former settings route. */
const LegacyExecutionEngineRedirect: React.FC = () => {
  const { search } = useLocation();
  return <Navigate to={`/settings/execution-engines${search}`} replace />;
};

const LegacyExtensionsRedirect: React.FC = () => {
  const { search } = useLocation();
  const searchParams = new URLSearchParams(search);
  const tab = searchParams.get('tab');
  searchParams.delete('tab');

  if (tab === 'tools') {
    return <Navigate to={withSearch('/mcp', searchParams)} replace />;
  }

  return <Navigate to={withSearch('/skills', searchParams)} replace />;
};

const CreativeStudioCanvasesRedirect: React.FC = () => {
  const { search, hash } = useLocation();
  return (
    <Navigate
      to={`${CREATIVE_STUDIO_CANVASES_PATH}${search}${hash}`}
      replace
    />
  );
};

// Legacy `/requirements/:id/edit` deep links → open the workspace with the
// requirement pre-selected in edit mode (the new shell hosts editing in a
// drawer, not a standalone form page).
const RequirementEditRedirect: React.FC = () => {
  const { id } = useParams();
  return <Navigate to={`/requirements?req=${id}&edit=1`} replace />;
};

const getHashRouteRedirectUrl = () => {
  if (typeof window === 'undefined') return null;
  if (!['http:', 'https:'].includes(window.location.protocol)) return null;
  if (window.location.hash) return null;

  const { origin, pathname, search } = window.location;
  if (pathname === '/' || pathname.endsWith('/index.html')) return null;

  return `${origin}/#${pathname}${search}`;
};

const PanelRoute: React.FC<{ layout: React.ReactElement }> = ({ layout }) => {
  const { status } = useAuth();
  const hashRouteRedirectUrl = getHashRouteRedirectUrl();

  if (hashRouteRedirectUrl) {
    window.location.replace(hashRouteRedirectUrl);
    return <AppLoader />;
  }

  return (
    <HashRouter>
      <Routes>
        <Route
          path='/login'
          element={status === 'authenticated' ? <Navigate to='/guid' replace /> : withRouteFallback(LoginPage)}
        />
        {/* The desktop-companion window route: fullscreen transparent, no app layout/sidebar. */}
        <Route path='/companion' element={withRouteFallback(CompanionPage)} />
        <Route element={<ProtectedAppRuntime />}>
          <Route element={layout}>
            <Route index element={<Navigate to='/guid' replace />} />
            {/* Creative Studio reuses the application titlebar and swaps the primary rail like Settings. */}
            <Route path={CREATIVE_STUDIO_ROOT_PATH} element={withRouteFallback(CreativeStudioFocusShell)}>
              <Route index element={<CreativeStudioCanvasesRedirect />} />
              <Route path='canvases' element={withRouteFallback(CreativeStudioCanvasesRoute)} />
              <Route path='projects' element={<CreativeStudioCanvasesRedirect />} />
              <Route path='canvas/:canvasId' element={withRouteFallback(CreativeStudioCanvasRoute)} />
              <Route path='director/:canvasId' element={withRouteFallback(CreativeStudioDirectorRoute)} />
              <Route path='image' element={withRouteFallback(CreativeStudioImageWorkbenchRoute)} />
              <Route path='video' element={withRouteFallback(CreativeStudioVideoWorkbenchRoute)} />
              <Route path='prompts' element={withRouteFallback(CreativeStudioPromptsRoute)} />
              <Route path='assets' element={withRouteFallback(CreativeStudioAssetsRoute)} />
              <Route path='templates' element={withRouteFallback(CreativeStudioTemplateRoute)} />
            </Route>
            {/* Models, presets, skills, and MCP are independent top-level capabilities. */}
            <Route path='/models' element={withRouteFallback(ModelHubPage)} />
            <Route path='/extensions' element={<LegacyExtensionsRedirect />} />
            <Route path='/mcp' element={withRouteFallback(McpPage)} />
            <Route path='/open-capabilities' element={withRouteFallback(OpenCapabilitiesPage)} />
            <Route path='/browser' element={withRouteFallback(BrowserPage)} />
            <Route path='/presets' element={withRouteFallback(PresetSettings)} />
            <Route path='/skills' element={withRouteFallback(SkillsSettingsPage)} />
            {/* Session section — the secondary sidebar (ContentSider) persists across these routes */}
            <Route element={<SessionShellRoute />}>
              <Route path='/guid' element={withRouteFallback(Guid)} />
              <Route path='/conversation/:id' element={withRouteFallback(Conversation)} />
              <Route path='/terminal-new' element={withRouteFallback(TerminalCreatePage)} />
              <Route path='/terminal/:id' element={withRouteFallback(TerminalSessionPage)} />
            </Route>
            {/* Relocated to the capability rail. */}
            <Route path='/settings/model' element={<Navigate to='/models?section=models' replace />} />
            <Route path='/settings/agent' element={<LegacyExecutionEngineRedirect />} />
            <Route path='/settings/capabilities' element={<Navigate to='/skills' replace />} />
            <Route path='/settings/skills-hub' element={<Navigate to='/skills' replace />} />
            <Route path='/settings/tools' element={<Navigate to='/open-capabilities' replace />} />
            <Route path='/settings/display' element={<Navigate to='/settings/system' replace />} />
            <Route path='/settings/webui' element={<Navigate to='/open-capabilities' replace />} />
            <Route path='/settings/system' element={withRouteFallback(SystemSettings)} />
            <Route path='/settings/execution-engines' element={withRouteFallback(ExecutionEngineSettings)} />
            <Route path='/settings/ssh-hosts' element={withRouteFallback(SshHostSettings)} />
            <Route path='/settings/agent-runtime' element={<Navigate to='/settings/execution-engines' replace />} />
            <Route path='/settings/browser-use' element={<Navigate to='/browser?tab=settings' replace />} />
            <Route path='/settings/computer-use' element={withRouteFallback(SystemSettings)} />
            <Route path='/settings/computer-history' element={withRouteFallback(ComputerHistorySettings)} />
            <Route path='/settings/about' element={withRouteFallback(SystemSettings)} />
            <Route path='/settings/ext/:tabId' element={withRouteFallback(ExtensionSettingsPage)} />
            <Route path='/settings/webhook' element={<Navigate to='/requirements/extensions?tab=notify' replace />} />
            <Route path='/settings' element={<Navigate to='/settings/system' replace />} />
            <Route path='/test/components' element={withRouteFallback(ComponentsShowcase)} />
            <Route path='/scheduled' element={withRouteFallback(ScheduledTasksPage)} />
            <Route path='/scheduled/:cron_job_id' element={withRouteFallback(TaskDetailPage)} />
            {/* Requirements platform — nested shell (ContentSider persists across sections) */}
            <Route path='/requirements' element={withRouteFallback(RequirementsLayout)}>
              <Route index element={withRouteFallback(WorkspacePage)} />
              <Route path='extensions' element={withRouteFallback(ExtensionsPage)} />
              <Route path='sources' element={withRouteFallback(SourcesPage)} />
            </Route>
            {/* Legacy requirement routes → fold into the new shell (preserve deep links) */}
            <Route path='/requirements/kanban' element={<Navigate to='/requirements?view=board' replace />} />
            <Route path='/requirements/new' element={<Navigate to='/requirements?new=1' replace />} />
            <Route path='/requirements/:id/edit' element={<RequirementEditRedirect />} />
            <Route path='/requirements/tag-sessions' element={<Navigate to='/requirements/extensions?tab=autowork' replace />} />
            <Route path='/autowork' element={<Navigate to='/requirements/extensions?tab=autowork' replace />} />
            {/* Webhook config relocated into 扩展能力 */}
            <Route path='/other' element={<Navigate to='/requirements/extensions?tab=notify' replace />} />
            <Route path='/nomi' element={withRouteFallback(NomiConfigPage)} />
            {/* 客服 (Customer Service) — a first-class domain separate from desktop companions. */}
            <Route path='/customer-service' element={withRouteFallback(CustomerServiceRosterPage)} />
            <Route path='/customer-service/:cs_agent_id' element={withRouteFallback(CustomerServiceDetailPage)} />
            <Route path='/knowledge' element={withRouteFallback(KnowledgeListPage)} />
            <Route path='/knowledge/:id' element={withRouteFallback(KnowledgeDetailPage)} />
            {/* 小程序 (Mini-apps) — the solidified library and its full-page runner. */}
            <Route path='/mini-apps' element={withRouteFallback(MiniAppsListPage)} />
            <Route path='/mini-apps/:id' element={withRouteFallback(MiniAppRunnerPage)} />
          </Route>
        </Route>
        <Route path='*' element={<Navigate to={status === 'authenticated' ? '/guid' : '/login'} replace />} />
      </Routes>
    </HashRouter>
  );
};

export default PanelRoute;
