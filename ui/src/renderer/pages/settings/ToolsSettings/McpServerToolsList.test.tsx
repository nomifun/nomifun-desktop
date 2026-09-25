import { afterEach, beforeAll, describe, expect, test } from 'bun:test';
import { cleanup, render } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import type { IMcpServer } from '@/common/config/storage';
import { parseMcpServerId } from '@/common/types/ids';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import McpServerToolsList from './McpServerToolsList';

const locale = createInstance();

beforeAll(async () => {
  await locale.init({
    lng: 'en-US',
    resources: { 'en-US': { translation: { common, settings } } },
  });
});

afterEach(cleanup);

const makeServer = (overrides: Partial<IMcpServer> = {}): IMcpServer => ({
  mcp_server_id: parseMcpServerId('0190f5fe-7c00-7a00-8000-000000000069'),
  name: 'browser',
  enabled: true,
  transport: { type: 'stdio', command: 'browser-mcp' },
  created_at: 1,
  updated_at: 1,
  original_json: '{}',
  ...overrides,
});

const renderList = (server: IMcpServer) =>
  render(
    <I18nextProvider i18n={locale}>
      <McpServerToolsList server={server} />
    </I18nextProvider>
  );

describe('McpServerToolsList', () => {
  test('shows a one-line placeholder when a failed server has no tools', () => {
    const view = renderList(makeServer({ last_test_status: 'error', tools: [] }));

    expect(view.getByRole('status').textContent).toBe(settings.mcpNoToolsAfterFailure);
    expect(view.queryByTestId('mcp-tool-grid')).toBeNull();
  });

  test('lays tools out as a compact borderless two-column grid', () => {
    const view = renderList(
      makeServer({
        tools: [
          { name: 'browser_navigate', description: 'Navigate to a URL' },
          { name: 'browser_go_back', description: 'Go back to the previous page' },
          { name: 'browser_get_text', description: 'Get page text' },
        ],
      })
    );

    const grid = view.getByTestId('mcp-tool-grid');
    expect(grid.className).toContain('grid-cols-2');
    expect(view.getByTestId('mcp-tool-column-divider').getAttribute('aria-hidden')).toBe('true');

    const items = view.getAllByTestId('mcp-tool-item');
    expect(items).toHaveLength(3);
    expect(items.every((item) => !item.className.split(/\s+/).includes('border'))).toBe(true);
    expect(items.every((item) => item.className.includes('gap-6px') && item.className.includes('py-2px'))).toBe(true);

    const title = view.getByText('browser_navigate');
    expect(title.className).toContain('font-normal');
    expect(title.className).not.toContain('font-600');
  });
});
