import { afterEach, describe, expect, test } from 'bun:test';
import { act, cleanup, render, waitFor } from '@testing-library/react';
import { createMemoryRouter, RouterProvider } from 'react-router-dom';
import LegacyEnhancedToolsRedirect from './LegacyEnhancedToolsRedirect';

afterEach(cleanup);

describe('legacy enhanced tools deep links', () => {
  test('replaces old skill links and preserves browser back and forward destinations', async () => {
    const router = createMemoryRouter([
      { path: '/conversation/:id', element: <div>Conversation</div> },
      { path: '/skills', element: <LegacyEnhancedToolsRedirect category='skills' /> },
      { path: '/enhanced-tools', element: <div>Enhanced Tools</div> },
    ], { initialEntries: ['/conversation/prior', {
      pathname: '/skills', search: '?tab=market&highlight=PDF%20Tools&keep=1', hash: '#details',
      state: { returnTo: '/conversation/prior' },
    }] });
    const view = render(<RouterProvider router={router} />);
    await waitFor(() => expect(router.state.location.pathname).toBe('/enhanced-tools'));
    const params = new URLSearchParams(router.state.location.search);
    expect(params.get('category')).toBe('skills');
    expect(params.get('view')).toBe('market');
    expect(params.get('highlight')).toBe('PDF Tools');
    expect(params.get('keep')).toBe('1');
    expect(router.state.location.hash).toBe('#details');
    expect(router.state.location.state).toEqual({ returnTo: '/conversation/prior' });

    await act(async () => { await router.navigate(-1); });
    expect(router.state.location.pathname).toBe('/conversation/prior');
    await act(async () => { await router.navigate(1); });
    expect(router.state.location.pathname).toBe('/enhanced-tools');
    expect(new URLSearchParams(router.state.location.search).get('view')).toBe('market');
    view.unmount();
    router.dispose();
  });

  test('old MCP market and settings links keep the intended classification', async () => {
    for (const [path, category] of [['/mcp', 'mcp'], ['/settings/skills-hub', 'skills'], ['/settings/capabilities', 'skills']] as const) {
      const router = createMemoryRouter([
        { path, element: <LegacyEnhancedToolsRedirect category={category} /> },
        { path: '/enhanced-tools', element: <div>Enhanced Tools</div> },
      ], { initialEntries: [`${path}?tab=market&keep=1`] });
      const view = render(<RouterProvider router={router} />);
      await waitFor(() => expect(router.state.location.pathname).toBe('/enhanced-tools'));
      const params = new URLSearchParams(router.state.location.search);
      expect(params.get('category')).toBe(category);
      expect(params.get('view')).toBe('market');
      expect(params.get('keep')).toBe('1');
      view.unmount();
      router.dispose();
    }
  });
});
