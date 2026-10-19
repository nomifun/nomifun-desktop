import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const preview = readFileSync(new URL('./agent-workbench-preview.tsx', import.meta.url), 'utf8');
const fixture = readFileSync(new URL('./fixtures/agent-workbench-catalog.json', import.meta.url), 'utf8');
const canonicalActions = JSON.parse(readFileSync(new URL('../../crates/backend/nomifun-agent-contracts/contracts/tool-presentation.json', import.meta.url), 'utf8')) as Array<{
  action_id: string;
  capability_id: string;
}>;
const selector = readFileSync(new URL('./GuidAgentSelectorPreview.tsx', import.meta.url), 'utf8');
const workspaceCss = readFileSync(new URL('../src/renderer/pages/agentSettings/AgentCapabilityWorkspace.module.css', import.meta.url), 'utf8');

describe('Agent Workbench visual acceptance harness', () => {
  test('serves the current Module catalog and derives the six-template count', () => {
    expect(preview).toContain("path === '/api/agent-catalog'");
    expect(preview).toContain('official_template_count: Object.keys(seed.templates).length');
    expect(preview).toContain('action_allowlist');
    expect(Array.isArray(JSON.parse(fixture))).toBe(true);
  });

  test('uses canonical Module and Action identities for every preview grant', () => {
    const modules = JSON.parse(fixture) as Array<{ id: string; actions: Array<[string, string]> }>;
    for (const module of modules) {
      const currentActions = canonicalActions
        .filter((action) => action.capability_id === module.id)
        .map((action) => action.action_id);
      expect(currentActions.length).toBeGreaterThan(0);
      for (const [actionId] of module.actions) {
        expect(currentActions).toContain(actionId);
      }
    }
  });

  test('imports only current Guid selector surfaces', () => {
    expect(selector).toContain("components/chat/ChatModelSelector");
    expect(selector).toContain('useAgentPresets');
    expect(selector).not.toContain('GuidModelSelector');
    expect(selector).not.toContain("kind: 'default'");
  });

  test('constrains the production page to the inspected desktop viewport', () => {
    expect(preview).toContain('html,body,#root{margin:0;height:100%;min-height:0');
    expect(preview).toContain('.preview-frame{height:calc(100% - 30px)');
    expect(workspaceCss).toContain('grid-auto-rows: max-content;');
    expect(workspaceCss).toMatch(/\.moduleGrid\s*\{[\s\S]*?flex:\s*1;/);
  });
});
