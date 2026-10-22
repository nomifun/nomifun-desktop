import { afterEach, beforeAll, describe, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render, within } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { mcpService } from '@/common/adapter/ipcBridge';
import type { IMcpServer } from '@/common/config/storage';
import { parseMcpServerId } from '@/common/types/ids';
import * as agents from '@/renderer/hooks/agent/useAgents';
import * as theme from '@/renderer/hooks/context/ThemeContext';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import AddMcpServerModal from './AddMcpServerModal';

const locale = createInstance();
const restore: Array<() => void> = [];
beforeAll(async () => {
  await locale.init({ lng: 'en-US', resources: { 'en-US': { translation: { settings, common } } } });
});
afterEach(() => {
  cleanup();
  restore.splice(0).reverse().forEach((dispose) => dispose());
});
const track = <T extends { mockRestore: () => void }>(spy: T): T => {
  restore.push(() => spy.mockRestore());
  return spy;
};

function fixture(importMode: 'json' | 'oneclick', server?: IMcpServer) {
  const detecting = track(spyOn(agents, 'getAgents').mockResolvedValue([]));
  track(spyOn(mcpService.getAgentMcpConfigs, 'invoke').mockResolvedValue([]));
  track(spyOn(console, 'error').mockImplementation(() => {}));
  track(spyOn(theme, 'useThemeContext').mockReturnValue({
    theme: 'light', colorScheme: 'default', fontScale: 1,
    setTheme: async () => {}, setColorScheme: async () => {}, setFontScale: async () => {},
  }));
  const cancel = mock();
  const view = render(<I18nextProvider i18n={locale}>
    <AddMcpServerModal visible importMode={importMode} server={server} onCancel={cancel} onSubmit={async () => {}} />
  </I18nextProvider>);
  return { ...view, detecting, cancel };
}

describe('explicit MCP import mode', () => {
  test('one-click import stays discoverable when no external Agent is installed', async () => {
    const view = fixture('oneclick');
    expect(within(view.getByRole('dialog')).getByText(settings.mcpOneKeyImport)).toBeTruthy();
    expect(view.detecting).not.toHaveBeenCalled();
    await act(async () => fireEvent.click(view.getByRole('button', { name: settings.mcpNextStep })));
    expect(view.getByRole('alert').textContent).toBe(settings.mcpAgentUnavailable);
    expect(view.getByRole('button', { name: common.retry })).toBeTruthy();
    expect(view.queryByText(settings.mcpImportFromJSON)).toBeNull();
    expect(view.detecting).toHaveBeenCalledTimes(1);
  });

  test('JSON import opens directly without probing Agent availability', () => {
    const view = fixture('json');
    expect(within(view.getByRole('dialog')).getByText(settings.mcpImportFromJSON)).toBeTruthy();
    expect(view.detecting).not.toHaveBeenCalled();
  });

  test('editing always opens its configuration even after one-click import was selected', () => {
    const view = fixture('oneclick', {
      mcp_server_id: parseMcpServerId('0190f5fe-7c00-7a00-8000-000000000069'),
      name: 'fixture', enabled: false, transport: { type: 'stdio', command: 'fixture-command' },
      original_json: '{"mcpServers":{"fixture":{"command":"fixture-command"}}}', created_at: 1, updated_at: 1,
    });
    expect(within(view.getByRole('dialog')).getByText(settings.mcpEditServer)).toBeTruthy();
    expect(view.detecting).not.toHaveBeenCalled();
  });
});
