import { describe, expect, test } from 'bun:test';
import { enhancedToolsUrl, legacyEnhancedToolsUrl } from './routes';

describe('enhanced tools navigation', () => {
  test('builds explicit category and market destinations', () => {
    expect(enhancedToolsUrl('mcp')).toBe('/enhanced-tools?category=mcp');
    expect(enhancedToolsUrl('skills')).toBe('/enhanced-tools?category=skills');
    expect(enhancedToolsUrl('skills', 'market')).toBe('/enhanced-tools?category=skills&view=market');
  });

  test('preserves legacy skill focus, extra query values, and fragment', () => {
    const result = legacyEnhancedToolsUrl('skills', '?tab=market&highlight=PDF%20Tools&keep=1&keep=2', '#details');
    const url = new URL(result, 'https://nomifun.test');
    expect(url.pathname).toBe('/enhanced-tools');
    expect(url.searchParams.get('category')).toBe('skills');
    expect(url.searchParams.get('view')).toBe('market');
    expect(url.searchParams.get('highlight')).toBe('PDF Tools');
    expect(url.searchParams.getAll('keep')).toEqual(['1', '2']);
    expect(url.searchParams.has('tab')).toBe(false);
    expect(url.hash).toBe('#details');
  });

  test('maps MCP markets and installed panes to the same canonical page', () => {
    expect(new URL(legacyEnhancedToolsUrl('mcp', '?tab=market'), 'https://nomifun.test').searchParams.get('view')).toBe('market');
    for (const [category, tab] of [['mcp', 'servers'], ['skills', 'library']] as const) {
      const url = new URL(legacyEnhancedToolsUrl(category, `?tab=${tab}&view=market&category=other&keep=1`), 'https://nomifun.test');
      expect(url.searchParams.get('category')).toBe(category);
      expect(url.searchParams.has('view')).toBe(false);
      expect(url.searchParams.get('keep')).toBe('1');
    }
  });
});
