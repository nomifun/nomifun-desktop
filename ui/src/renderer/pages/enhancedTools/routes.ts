export const ENHANCED_TOOLS_PATH = '/enhanced-tools';

export type EnhancedToolsCategory = 'mcp' | 'skills' | 'connectors';

export const enhancedToolsUrl = (category: EnhancedToolsCategory, view?: 'market'): string => {
  const params = new URLSearchParams({ category });
  if (view) params.set('view', view);
  return `${ENHANCED_TOOLS_PATH}?${params}`;
};

/** Redirect old management links without losing skill focus or unrelated query state. */
export const legacyEnhancedToolsUrl = (
  category: EnhancedToolsCategory,
  search: string,
  hash = '',
): string => {
  const params = new URLSearchParams(search);
  const oldTab = params.get('tab');
  params.delete('tab');
  params.set('category', category);
  if (oldTab === 'market') params.set('view', 'market');
  else if (oldTab === 'library' || oldTab === 'servers') params.delete('view');
  return `${ENHANCED_TOOLS_PATH}?${params}${hash}`;
};
