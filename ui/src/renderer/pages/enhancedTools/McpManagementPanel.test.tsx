import { afterEach, beforeAll, describe, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render, within } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { mcpService } from '@/common/adapter/ipcBridge';
import { EditorView } from '@codemirror/view';
import { Message } from '@arco-design/web-react';
import type { IMcpServer } from '@/common/config/storage';
import { parseMcpServerId } from '@/common/types/ids';
import { attachMcpMarketOrigin } from '@/renderer/services/mcp/mcpMarket';
import * as theme from '@/renderer/hooks/context/ThemeContext';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import McpManagementPanel, { type McpImportMode } from './McpManagementPanel';

const locale = createInstance();
beforeAll(async () => {
  await locale.init({ lng: 'en-US', resources: { 'en-US': { translation: { settings, common } } } });
});
const restore: Array<() => void> = [];
afterEach(() => {
  cleanup();
  restore.splice(0).reverse().forEach((dispose) => dispose());
});
const track = <T extends { mockRestore: () => void }>(spy: T): T => {
  restore.push(() => spy.mockRestore());
  return spy;
};

const servers: IMcpServer[] = Array.from({ length: 12 }, (_, index) => ({
  mcp_server_id: parseMcpServerId(`0190f5fe-7c00-7a00-8000-${String(index + 1).padStart(12, '0')}`),
  name: `server-${index + 1}`,
  description: `Description ${index + 1}`,
  enabled: false,
  transport: { type: 'stdio', command: 'fixture-command' },
  original_json: '{}', created_at: 1, updated_at: 1,
}));
const saveMcpServers = async () => {};
const onImportClose = () => {};
const panel = (searchQuery = '', mcpServers = servers, highlightServerId?: string, visible = true, importMode: McpImportMode | null = null) => (
  <I18nextProvider i18n={locale}>
    <McpManagementPanel mcpServers={mcpServers} saveMcpServers={saveMcpServers}
      searchQuery={searchQuery} importMode={importMode} onImportClose={onImportClose}
      highlightServerId={highlightServerId} visible={visible} />
  </I18nextProvider>
);

describe('MCP management in the enhanced-tools page', () => {
  test('search starts at the first matching page and clearing restores all servers', () => {
    const view = render(panel());
    const installed = within(view.getByTestId('mcp-installed-surface'));
    expect(installed.getByText('server-10')).toBeTruthy();
    expect(installed.queryByText('server-11')).toBeNull();
    fireEvent.click(within(view.getByTestId('mcp-server-pagination')).getByText('2'));
    expect(installed.getByText('server-12')).toBeTruthy();

    view.rerender(panel('DESCRIPTION 1'));
    expect(installed.getByText('server-1')).toBeTruthy();
    expect(installed.getByText('server-12')).toBeTruthy();
    expect(installed.queryByText('server-2')).toBeNull();
    expect(view.queryByTestId('mcp-server-pagination')).toBeNull();
    view.rerender(panel('missing-server'));
    expect(installed.getByText(settings.mcpNoServersFound)).toBeTruthy();
    view.rerender(panel());
    expect(installed.getByText('server-1')).toBeTruthy();
    expect(installed.queryByText('server-11')).toBeNull();
  });

  test('search accepts both market presentation and the original server identity', () => {
    const current = {
      ...servers[0]!,
      original_json: attachMcpMarketOrigin('{}', { id: 'mcpworld:fixture', name: 'Readable Market Title' }),
    };
    const view = render(panel('readable market', [current]));
    expect(view.getByText('Readable Market Title')).toBeTruthy();
    view.rerender(panel(current.name, [current]));
    expect(view.getByText('Readable Market Title')).toBeTruthy();
  });

  test('market return locates its row once without locking subsequent pagination', async () => {
    const highlighted = servers[11]!.mcp_server_id;
    const view = render(panel('', servers, highlighted));
    const installed = within(view.getByTestId('mcp-installed-surface'));
    expect(installed.getByText('server-12')).toBeTruthy();
    fireEvent.click(within(view.getByTestId('mcp-server-pagination')).getByText('1'));
    expect(installed.getByText('server-1')).toBeTruthy();
    await act(async () => {
      view.rerender(panel('', servers.map((server) => ({ ...server, updated_at: 2 })), highlighted));
    });
    expect(installed.getByText('server-1')).toBeTruthy();
    expect(installed.queryByText('server-12')).toBeNull();
  });

  test('an explicit imported row takes priority when its previous search is cleared', () => {
    const imported: IMcpServer = {
      ...servers[0]!, name: 'newly-imported-server',
      mcp_server_id: parseMcpServerId('0190f5fe-7c00-7a00-8000-000000000099'),
    };
    const view = render(panel('unmatched-search'));
    view.rerender(panel('', [...servers, imported], imported.mcp_server_id));
    expect(within(view.getByTestId('mcp-installed-surface')).getByText(imported.name)).toBeTruthy();
    expect(within(view.getByTestId('mcp-server-pagination')).getByText('2').getAttribute('aria-current')).toBe('true');
  });

  test.each([1, 2])('importing %s MCP server(s) from a filtered list reports exact persisted identities', async (count) => {
    track(spyOn(theme, 'useThemeContext').mockReturnValue({
      theme: 'light', colorScheme: 'default', fontScale: 1,
      setTheme: async () => {}, setColorScheme: async () => {}, setFontScale: async () => {},
    }));
    const imported = servers.slice(0, count).map((server, index) => ({ ...server, name: `new-import-${index}`, enabled: true }));
    track(spyOn(mcpService.createServer, 'invoke').mockResolvedValue(imported[0]!));
    track(spyOn(mcpService.importServers, 'invoke').mockResolvedValue(imported));
    const testing = track(spyOn(mcpService.testMcpConnection, 'invoke').mockResolvedValue({ success: true, tools: [] }));
    const onImported = mock((_rows: IMcpServer[]) => {});
    const view = render(<I18nextProvider i18n={locale}>
      <McpManagementPanel mcpServers={servers} saveMcpServers={saveMcpServers}
        searchQuery='unmatched-search' importMode='json' onImportClose={onImportClose} onImported={onImported} />
    </I18nextProvider>);
    expect(within(view.getByTestId('mcp-installed-surface')).getByText(settings.mcpNoServersFound)).toBeTruthy();
    const editor = EditorView.findFromDOM(view.getByRole('textbox'))!;
    act(() => editor.dispatch({ changes: {
      from: 0, to: editor.state.doc.length,
      insert: JSON.stringify({ mcpServers: Object.fromEntries(imported.map((server) => [server.name, { command: 'fixture-command' }])) }),
    } }));
    await act(async () => fireEvent.click(view.getByRole('button', { name: common.confirm })));
    expect(onImported).toHaveBeenCalledTimes(1);
    expect(onImported).toHaveBeenCalledWith(imported);
    imported.forEach((server, index) => expect(onImported.mock.calls[0]![0][index]).toBe(server));
    expect(testing).toHaveBeenCalledTimes(count);
  });

  test('editing a matching server preserves the search without reporting a new import', async () => {
    track(spyOn(Message, 'success').mockReturnValue(() => {}));
    track(spyOn(Message, 'error').mockReturnValue(() => {}));
    track(spyOn(theme, 'useThemeContext').mockReturnValue({
      theme: 'light', colorScheme: 'default', fontScale: 1,
      setTheme: async () => {}, setColorScheme: async () => {}, setFontScale: async () => {},
    }));
    const current = servers[0]!;
    const updating = track(spyOn(mcpService.updateServer, 'invoke').mockResolvedValue(current));
    const testing = track(spyOn(mcpService.testMcpConnection, 'invoke').mockResolvedValue({ success: true, tools: [] }));
    const onImported = mock((_rows: IMcpServer[]) => {});
    const view = render(<I18nextProvider i18n={locale}>
      <McpManagementPanel mcpServers={[current]} saveMcpServers={saveMcpServers}
        searchQuery={current.name} importMode={null} onImportClose={onImportClose} onImported={onImported} />
    </I18nextProvider>);
    fireEvent.click(within(view.getByTestId('mcp-installed-surface')).getByRole('button', { name: settings.mcpEditServer }));
    fireEvent.click(await view.findByRole('menuitem', { name: settings.mcpEditServer }));
    const editor = EditorView.findFromDOM(view.getByRole('textbox'))!;
    act(() => editor.dispatch({ changes: {
      from: 0, to: editor.state.doc.length,
      insert: JSON.stringify({ mcpServers: { [current.name]: { command: 'edited-fixture-command' } } }),
    } }));
    await act(async () => fireEvent.click(view.getByRole('button', { name: common.confirm })));
    expect(updating.mock.calls[0]![0].mcp_server_id).toBe(current.mcp_server_id);
    expect(testing).toHaveBeenCalledTimes(1);
    expect(onImported).not.toHaveBeenCalled();
  });

  test.each(['close', 'category'] as const)('an old edit completion cannot close a newer same-server editor after %s', async (action) => {
    track(spyOn(Message, 'success').mockReturnValue(() => {}));
    track(spyOn(Message, 'error').mockReturnValue(() => {}));
    track(spyOn(theme, 'useThemeContext').mockReturnValue({
      theme: 'light', colorScheme: 'default', fontScale: 1,
      setTheme: async () => {}, setColorScheme: async () => {}, setFontScale: async () => {},
    }));
    let resolveUpdate!: (server: IMcpServer) => void;
    const pending = new Promise<IMcpServer>((resolve) => { resolveUpdate = resolve; });
    const updating = track(spyOn(mcpService.updateServer, 'invoke').mockReturnValue(pending));
    const testing = track(spyOn(mcpService.testMcpConnection, 'invoke').mockResolvedValue({ success: true, tools: [] }));
    const current = servers[0]!;
    const currentServers = [current];
    const view = render(panel('', currentServers));
    const openEditor = async () => {
      fireEvent.click(within(view.getByTestId('mcp-installed-surface')).getByRole('button', { name: settings.mcpEditServer }));
      fireEvent.click(await view.findByRole('menuitem', { name: settings.mcpEditServer }));
    };
    const changeCommand = (command: string) => {
      const editor = EditorView.findFromDOM(view.getByRole('textbox'))!;
      act(() => editor.dispatch({ changes: {
        from: 0, to: editor.state.doc.length,
        insert: JSON.stringify({ mcpServers: { [current.name]: { command } } }),
      } }));
    };
    await openEditor();
    changeCommand('old-pending-command');
    await act(async () => fireEvent.click(view.getByRole('button', { name: common.confirm })));
    expect(updating).toHaveBeenCalledTimes(1);

    if (action === 'close') {
      fireEvent.click(view.getByRole('button', { name: common.cancel }));
      await openEditor();
    } else {
      view.rerender(panel('', currentServers, undefined, false));
      view.rerender(panel('', currentServers));
    }
    changeCommand('new-unsaved-command');
    await act(async () => { resolveUpdate(current); });
    expect(view.getByRole('dialog')).toBeTruthy();
    expect(EditorView.findFromDOM(view.getByRole('textbox'))!.state.doc.toString()).toContain('new-unsaved-command');
    expect(testing).toHaveBeenCalledTimes(1);
    expect(updating).toHaveBeenCalledTimes(1);
  });

  test.each(['json', 'oneclick'] as const)('inactive MCP category hides its %s import portal without closing the owner', (mode) => {
    track(spyOn(theme, 'useThemeContext').mockReturnValue({
      theme: 'light', colorScheme: 'default', fontScale: 1,
      setTheme: async () => {}, setColorScheme: async () => {}, setFontScale: async () => {},
    }));
    const view = render(panel('', servers, undefined, true, mode));
    expect(view.getByRole('dialog')).toBeTruthy();
    view.rerender(panel('', servers, undefined, false, mode));
    expect(view.queryByRole('dialog')).toBeNull();
    view.rerender(panel('', servers, undefined, true, mode));
    expect(view.getByRole('dialog')).toBeTruthy();
  });

  test.each(['edit', 'delete', 'oauth'] as const)('inactive MCP category hides and restores its %s portal', async (action) => {
    track(spyOn(theme, 'useThemeContext').mockReturnValue({
      theme: 'light', colorScheme: 'default', fontScale: 1,
      setTheme: async () => {}, setColorScheme: async () => {}, setFontScale: async () => {},
    }));
    track(spyOn(mcpService.getMcpAuth, 'invoke').mockResolvedValue({ state: 'required', granted_scopes: [] }));
    const current: IMcpServer[] = [{
      ...servers[0]!,
      original_json: '{"mcpServers":{"server-1":{"command":"fixture-command"}}}',
      ...(action === 'oauth' ? { transport: { type: 'http', url: 'https://mcp.example.test/mcp' } as const } : {}),
    }];
    const view = render(panel('', current));
    fireEvent.click(within(view.getByTestId('mcp-installed-surface')).getByRole('button', { name: settings.mcpEditServer }));
    const menuLabel = action === 'edit' ? settings.mcpEditServer : action === 'delete' ? settings.mcpDeleteServer : settings.mcpOAuthConfigureClient;
    fireEvent.click(await view.findByRole('menuitem', { name: menuLabel }));
    expect(view.getByRole('dialog')).toBeTruthy();
    view.rerender(panel('', current, undefined, false));
    expect(view.queryByRole('dialog')).toBeNull();
    view.rerender(panel('', current));
    expect(view.getByRole('dialog')).toBeTruthy();
  });
});
