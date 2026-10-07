import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const read = (url: URL) => readFileSync(url, 'utf8');

describe('Agent navigation', () => {
  test('uses the localized workbench name for the expanded label and tooltip', () => {
    const entry = read(
      new URL(
        '../../components/layout/Sider/SiderNav/SiderAgentEntry.tsx',
        import.meta.url
      )
    );

    expect(
      entry.includes(
        "t('agentSettings.navigation.railTitle', { defaultValue: 'Agent Workbench' })"
      )
    ).toBe(true);
    expect(entry.match(/content=\{label\}/g)).toHaveLength(2);
    expect(entry.includes('>{label}</span>')).toBe(true);
  });

  test('the main router owns only the public Agent routes', () => {
    const router = read(new URL('../../components/layout/Router.tsx', import.meta.url));

    expect(
      router.includes(
        "const AgentSettingsPage = React.lazy(() => import('@renderer/pages/agentSettings'));"
      )
    ).toBe(true);
    expect(
      router.includes(
        "const AgentSessionPage = React.lazy(() => import('@renderer/pages/agentSession/AgentSessionPage'));"
      )
    ).toBe(true);
    expect(router.includes("path='/agent' element={withRouteFallback(AgentSettingsPage)}")).toBe(true);
    expect(
      router.includes(
        "path='/agent-sessions/:agentSessionId' element={withRouteFallback(AgentSessionPage)}"
      )
    ).toBe(true);

    expect(router.includes('LegacyAgentAuthoringRedirect')).toBe(false);
    expect(router.includes("path='/presets'")).toBe(false);
    expect(router.includes("path='/settings/agent-presets/*'")).toBe(false);
    expect(router.includes("path='/settings/agent'")).toBe(false);
  });

  test('Nomi Runtime diagnostics link to Agent Workbench without a selectable Plugin runtime', () => {
    const settingsPage = read(new URL('../settings/ExecutionEngines/index.tsx', import.meta.url));

    expect(settingsPage.includes('AgentModalContent')).toBe(false);
    expect(settingsPage.includes("to='/agent'")).toBe(true);
    expect(settingsPage.includes("to='/settings/javascript-runtime'")).toBe(false);
    expect(settingsPage.includes('ipcBridge.agentPlatform.runtime.get.invoke()')).toBe(true);
    expect(settingsPage.includes('NOMI_FAMILY')).toBe(false);
    expect(settingsPage.includes('nomifun.coding')).toBe(false);
    expect(settingsPage.includes('<Select')).toBe(false);
    expect(settingsPage.includes('<RuntimeManager />')).toBe(false);
    expect(settingsPage.includes('<LocalAgents />')).toBe(false);
    expect(settingsPage.includes('SettingsModal')).toBe(false);
  });

  test('historical Agent Session links redirect to the standard conversation without a second client', () => {
    const page = read(new URL('./AgentSessionPage.tsx', import.meta.url));
    expect(page.includes('parseConversationId(agentSessionId)')).toBe(true);
    expect(page.includes('<Navigate replace')).toBe(true);
    expect(page.includes('/conversation/')).toBe(true);
    expect(page.includes('Surface')).toBe(false);
    expect(page.includes('sessions.create')).toBe(false);
  });
});
