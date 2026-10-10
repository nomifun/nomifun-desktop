import {
  resourceSectionForPath,
  type ResourceSection,
} from '@renderer/pages/creativeStudio/app/resourceRoutes';

let conversationRoute: Promise<typeof import('@renderer/pages/conversation')> | undefined;
export const loadConversationRoute = () => conversationRoute ??= import('@renderer/pages/conversation').catch(error => {
  conversationRoute = undefined;
  throw error;
});

export const loadResourcePageBoundary = () =>
  import('@renderer/pages/creativeStudio/app/ResourcePageBoundary');
export const loadCreativeStudioCanvasesRoute = () =>
  import('@renderer/pages/creativeStudio/canvases/CreativeStudioCanvasesRoute');
export const loadCreativeStudioPromptsRoute = () =>
  import('@renderer/pages/creativeStudio/prompts/page/CreativeStudioPromptsRoute');
export const loadCreativeStudioAssetsRoute = () =>
  import('@renderer/pages/creativeStudio/assets/page/CreativeAssetLibraryPage');
export const loadCreativeStudioCanvasRoute = () =>
  import('@renderer/pages/creativeStudio/canvases/CreativeCanvasProductRoute');
export const loadCreativeStudioTemplateRoute = () =>
  import('@renderer/pages/creativeStudio/templates/page/CreativeTemplateRoute');
export const loadKnowledgeDetailRoute = () =>
  import('@renderer/pages/knowledge/KnowledgeDetailPage');

const resourceRouteLoaders: Record<ResourceSection, () => Promise<unknown>> = {
  canvases: loadCreativeStudioCanvasesRoute,
  canvas: loadCreativeStudioCanvasRoute,
  prompts: loadCreativeStudioPromptsRoute,
  assets: loadCreativeStudioAssetsRoute,
  templates: loadCreativeStudioTemplateRoute,
};

const ignorePreloadFailure = (preload: Promise<unknown>): Promise<void> =>
  preload.then(
    () => undefined,
    () => undefined
  );

/**
 * Warm the exact retained resource page and its overlay host. This is only an
 * optimisation; the regular route boundary still owns any real load failure.
 */
export const preloadResourceRoute = (path: string): Promise<void> => {
  const section = resourceSectionForPath(path);
  const loader = section ? resourceRouteLoaders[section] : null;
  if (!loader) return Promise.resolve();

  return ignorePreloadFailure(
    Promise.all([loadResourcePageBoundary(), loader()])
  );
};

/** Warm the knowledge detail split chunk before a card navigation. */
export const preloadKnowledgeDetailRoute = (): Promise<void> =>
  ignorePreloadFailure(loadKnowledgeDetailRoute());
