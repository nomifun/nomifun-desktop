import '../../../../test/setup-dom.ts';
import { act, cleanup, renderHook } from '@testing-library/react';
import { Message } from '@arco-design/web-react';
import { afterEach, expect, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import type { ReactNode } from 'react';
import { I18nextProvider } from 'react-i18next';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import { mcpService } from '@/common/adapter/ipcBridge';
import type { IMcpServer } from '@/common/config/storage';
import { parseMcpServerId } from '@/common/types/ids';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import { useMcpConnection } from './useMcpConnection';

const locale = createInstance();
await locale.init({ lng: 'en-US', resources: { 'en-US': { translation: { settings } } } });
const wrapper = ({ children }: { children: ReactNode }) => <I18nextProvider i18n={locale}>{children}</I18nextProvider>;
const restores: Array<() => void> = [];
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach((restore) => restore()); });

const server: IMcpServer = {
  mcp_server_id: parseMcpServerId('0190f5fe-7c00-7a00-8000-000000000403'),
  name: 'slow-setup', enabled: false, transport: { type: 'http', url: 'https://example.test/mcp' },
  original_json: '{}', created_at: 1, updated_at: 1,
};

for (const sample of [
  { details: { phase: 'protocol_setup' }, expected: 'The owner could not complete setup.', absent: /120s|30s|undefined/ },
  { details: { phase: 'protocol_setup', timeout_seconds: 7 }, expected: '7s', absent: /30s|120s|undefined/ },
]) {
  test(`connection diagnostics use the owner's evidence: ${JSON.stringify(sample.details)}`, async () => {
    const errors = spyOn(Message, 'error').mockReturnValue(() => {});
    const probe = spyOn(mcpService.testMcpConnection, 'invoke').mockRejectedValue(new BackendHttpError({
      method: 'POST', path: '/api/mcp/test-connection', status: 504,
      body: { code: 'MCP_TIMEOUT', error: 'The owner could not complete setup.', details: sample.details },
    }));
    restores.push(() => errors.mockRestore(), () => probe.mockRestore());
    const hook = renderHook(() => useMcpConnection(), { wrapper });
    await act(async () => { await hook.result.current.handleTestMcpConnection(server); });
    expect(probe).toHaveBeenCalledTimes(1);
    expect(errors).toHaveBeenCalledTimes(1);
    const message = errors.mock.calls[0]?.[0];
    const content = typeof message === 'object' && message ? String(message.content) : String(message);
    expect(content).toContain(sample.expected);
    expect(content).not.toMatch(sample.absent);
    expect(hook.result.current.testingServers[server.mcp_server_id]).toBe(false);
  });
}
