import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const preview = readFileSync(new URL('./agent-workbench-preview.tsx', import.meta.url), 'utf8');
const fixture = readFileSync(new URL('./fixtures/agent-workbench-catalog.json', import.meta.url), 'utf8');
const selector = readFileSync(new URL('./GuidAgentSelectorPreview.tsx', import.meta.url), 'utf8');
const workspaceCss = readFileSync(new URL('../src/renderer/pages/agentSettings/AgentCapabilityWorkspace.module.css', import.meta.url), 'utf8');

describe('Agent Workbench visual acceptance harness', () => {
  test('serves the current Module catalog and derives the six-template count', () => {
    expect(preview).toContain("path === '/api/agent-catalog'");
    expect(preview).toContain('official_template_count: Object.keys(seed.templates).length');
    expect(preview).toContain('action_allowlist');
    expect(Array.isArray(JSON.parse(fixture))).toBe(true);
  });

  test('contains only current Module and slash Action identities', () => {
    for (const retired of [
      'fs.read', 'vcs.push', 'computer.observe', 'computer.input', 'a11y.observe',
      'schedule.store', 'robot.link', 'robot.audio', 'robot.device_tools',
    ]) {
      expect(fixture).not.toContain(`"${retired}"`);
    }
    for (const current of ['"computer"', '"computer/input"', '"robot"', '"robot/device"']) {
      expect(fixture).toContain(current);
    }
  });

  test('imports only current Guid selector surfaces', () => {
    expect(selector).toContain("components/chat/ChatModelSelector");
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
