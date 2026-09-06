import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const read = (url: URL) => readFileSync(url, 'utf8');

describe('Agent navigation', () => {
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

  test('execution-engine settings no longer links to Agent authoring', () => {
    const settingsPage = read(new URL('../settings/AgentSettings/index.tsx', import.meta.url));
    const settingsContent = read(
      new URL('../settings/AgentSettings/ExecutionEnginesSettingsContent.tsx', import.meta.url)
    );

    expect(settingsPage.includes('AgentModalContent')).toBe(false);
    expect(settingsPage.includes('ExecutionEnginesSettingsContent')).toBe(true);
    expect(settingsContent.includes('<LocalAgents />')).toBe(true);
    expect(settingsContent.includes('SettingsModal')).toBe(false);
    expect(settingsContent.includes('agentSettings.navigation')).toBe(false);
  });

  test('new AgentSession pages contain no legacy chat-container fallback', () => {
    const page = read(new URL('./AgentSessionPage.tsx', import.meta.url));
    const model = read(new URL('./model.ts', import.meta.url));
    const legacyType = 'Conver' + 'sation';
    expect(page.includes(legacyType)).toBe(false);
    expect(model.includes(legacyType)).toBe(false);
  });
});
