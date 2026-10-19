import { afterEach, beforeAll, describe, expect, mock, test } from 'bun:test';
import { cleanup, fireEvent, render } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import type { IMcpServer } from '@/common/config/storage';
import { parseMcpServerId } from '@/common/types/ids';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import McpServerHeader from './McpServerHeader';
import McpServerItem from './McpServerItem';

const locale = createInstance();
beforeAll(async () => {
  await locale.init({ lng: 'en-US', resources: { 'en-US': { translation: { settings } } } });
});
afterEach(cleanup);
const server = (overrides: Partial<IMcpServer> = {}): IMcpServer => ({
  mcp_server_id: parseMcpServerId('0190f5fe-7c00-7a00-8000-000000000069'),
  name: 'alpha', enabled: false,
  transport: { type: 'stdio', command: 'fixture-command' },
  original_json: '{}', created_at: 1, updated_at: 1,
  ...overrides,
});
function fixture(enabled = false, isTogglingEnabled = false, isTestingConnection = false) {
  const current = server({ enabled });
  const toggle = mock(() => {});
  const check = mock(() => {});
  const collapse = mock(() => {});
  const view = render(
    <I18nextProvider i18n={locale}>
      <div onClick={collapse}>
        <McpServerHeader server={current} isTestingConnection={isTestingConnection} isTogglingEnabled={isTogglingEnabled}
          onTestConnection={check} onToggleEnabled={toggle} onEditServer={() => {}} onDeleteServer={() => {}} />
      </div>
    </I18nextProvider>
  );
  return { ...view, current, toggle, check, collapse };
}

describe('MCP management enabled control', () => {
  test('authorization and connection results remain separate', () => {
    const current = server({ transport: { type: 'http', url: 'https://mcp.example.test/mcp' }, last_test_status: 'error' });
    const view = render(<I18nextProvider i18n={locale}>
      <McpServerHeader server={current} isTestingConnection={false}
        oauthStatus={{ auth: { state: 'authorized', granted_scopes: [] }, isChecking: false }}
        onTestConnection={() => {}} onToggleEnabled={() => {}} onEditServer={() => {}} onDeleteServer={() => {}} />
    </I18nextProvider>);
    expect(view.getByTestId('mcp-auth-authorized').textContent).toBe(settings.mcpAuthenticated);
    expect(view.getByRole('img', { name: settings.mcpCheckFailedSimple })).toBeTruthy();
  });

  test('an in-progress login offers cancellation and browser continuation without marking a connection', () => {
    const current = server({ transport: { type: 'http', url: 'https://mcp.example.test/mcp' } });
    const cancel = mock();
    const resume = mock();
    const view = render(<I18nextProvider i18n={locale}>
      <McpServerHeader server={current} isTestingConnection={false} isLoggingIn
        oauthAttempt={{ attempt_id: 'fixture', status: 'waiting_for_user', authorization_url: 'https://auth.example.test/authorize' }}
        onOAuthCancel={cancel} onOAuthContinue={resume}
        onTestConnection={() => {}} onToggleEnabled={() => {}} onEditServer={() => {}} onDeleteServer={() => {}} />
    </I18nextProvider>);
    expect(view.getByText(settings.mcpOAuthWaiting)).toBeTruthy();
    expect(view.getByRole('img', { name: settings.mcpDisconnected })).toBeTruthy();
    fireEvent.click(view.getByRole('button', { name: settings.mcpOAuthContinue }));
    expect(resume).toHaveBeenCalledWith(current);
    const cancelButton = view.getAllByRole('button').find((button) => button.textContent?.includes('common.cancel') || button.textContent?.includes('Cancel'))!;
    fireEvent.click(cancelButton);
    expect(cancel).toHaveBeenCalledWith(current);
  });

  test.each([false, true])('shows enabled=%s with an accessible separate toggle', (enabled) => {
    const view = fixture(enabled);
    const control = view.getByRole('switch', { name: `${enabled ? 'Disable' : 'Enable'} alpha` });
    expect(control.getAttribute('aria-checked')).toBe(String(enabled));
    expect(view.getByText(enabled ? settings.mcpEnabled : settings.mcpDisabled)).toBeTruthy();
    fireEvent.click(control);
    expect(view.toggle).toHaveBeenCalledWith(view.current);
    expect(view.check).not.toHaveBeenCalled();
    expect(view.collapse).not.toHaveBeenCalled();
    expect(control.getAttribute('aria-checked')).toBe(String(enabled));
  });

  test('blocks toggle and manual check while a toggle is pending', () => {
    const view = fixture(false, true);
    const control = view.getByRole('switch', { name: 'Enable alpha' }) as HTMLButtonElement;
    const check = view.getByRole('button', { name: settings.mcpTestConnection }) as HTMLButtonElement;
    expect(control.disabled).toBe(true);
    expect(control.getAttribute('aria-busy')).toBe('true');
    expect(check.disabled).toBe(true);
    fireEvent.click(control);
    fireEvent.click(check);
    expect(view.toggle).not.toHaveBeenCalled();
    expect(view.check).not.toHaveBeenCalled();
  });

  test('blocks toggling during a manual connection check', () => {
    const view = fixture(false, false, true);
    const control = view.getByRole('switch', { name: 'Enable alpha' }) as HTMLButtonElement;
    expect(control.disabled).toBe(true);
    fireEvent.click(control);
    expect(view.toggle).not.toHaveBeenCalled();
  });

  test('switch Enter does not toggle the tools collapse or prevent normal activation', () => {
    const current = server();
    const toggle = mock(() => {});
    const collapse = mock(() => {});
    const view = render(
      <I18nextProvider i18n={locale}>
        <McpServerItem server={current} isCollapsed={false} isTestingConnection={false}
          onToggleCollapse={collapse} onTestConnection={() => {}} onToggleEnabled={toggle}
          onEditServer={() => {}} onDeleteServer={() => {}} />
      </I18nextProvider>
    );
    const control = view.getByRole('switch', { name: 'Enable alpha' });
    const header = view.getAllByRole('button').find((button) => button.hasAttribute('aria-expanded'))!;
    control.focus();
    expect(fireEvent.keyDown(control, { key: 'Enter', code: 'Enter', keyCode: 13 })).toBe(true);
    expect(collapse).not.toHaveBeenCalled();
    expect(header.getAttribute('aria-expanded')).toBe('false');
    // Native buttons produce a click for Enter; Happy DOM needs that activation
    // dispatched explicitly. Stopping propagation must not cancel the default.
    fireEvent.click(control);
    expect(toggle).toHaveBeenCalledWith(current);
    expect(collapse).not.toHaveBeenCalled();

    fireEvent.keyDown(header, { key: 'Enter', code: 'Enter', keyCode: 13 });
    expect(collapse).toHaveBeenCalledTimes(1);
  });
});
