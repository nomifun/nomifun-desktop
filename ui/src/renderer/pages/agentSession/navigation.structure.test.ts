import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const read = (url: URL) => readFileSync(url, 'utf8');

describe('C6 canonical UI navigation', () => {
  test('main selects the canonical route host before the legacy router', () => {
    const main = read(new URL('../../main.tsx', import.meta.url));
    const routes = read(new URL('./CanonicalAgentRoutes.tsx', import.meta.url));

    expect(main.includes('isCanonicalAgentHashRoute(window.location.hash)')).toBe(true);
    expect(main.includes('<CanonicalAgentRoutes layout={layout} />')).toBe(true);
    expect(routes.includes("path='/settings/agent-presets/*'")).toBe(true);
    expect(routes.includes("path='/agent-sessions/:agentSessionId'")).toBe(true);
  });

  test('the real settings surface links to Agent Settings', () => {
    const settings = read(
      new URL(
        '../../components/settings/SettingsModal/contents/AgentModalContent.tsx',
        import.meta.url
      )
    );
    expect(settings.includes("window.location.hash = '/settings/agent-presets'")).toBe(true);
    expect(settings.includes('agentSettings.navigation.open')).toBe(true);
  });

  test('new AgentSession pages contain no legacy chat-container fallback', () => {
    const page = read(new URL('./AgentSessionPage.tsx', import.meta.url));
    const model = read(new URL('./model.ts', import.meta.url));
    const legacyType = 'Conver' + 'sation';
    expect(page.includes(legacyType)).toBe(false);
    expect(model.includes(legacyType)).toBe(false);
  });
});
