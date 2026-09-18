/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL) => readFileSync(url, 'utf8');

describe('ChatLayout advanced controls', () => {
  test('native browser surfaces do not inherit the file preview transform animation', () => {
    const source = readSource(new URL('./index.tsx', import.meta.url));
    expect(source).toContain("browserOpen ? 'browser-capability-surface' : 'preview-panel'");
  });
  test('keeps the generic Browser capability in the session tool rail at narrow desktop sizes', () => {
    const source = readSource(new URL('./index.tsx', import.meta.url));
    const css = readSource(new URL('./chat-layout.css', import.meta.url));
    expect(source).toContain("chat-layout-header-host");
    expect(source).toContain('workspaceAvailable={workspaceEnabled}');
    expect(source).toContain('browser={conversation_id ? {');
    expect(source).toContain('buttonRef: browserButton');
    expect(source).not.toContain('chat-browser-toggle');
    expect(css).toContain('container: conversation-header / inline-size');
    expect(css).toContain('@container conversation-header (max-width: 600px)');
    expect(css).toContain('order:2; flex:1 1 100%; min-width:0; flex-wrap:wrap');
    expect(css).not.toContain('(max-width: 768px)');
  });

  test('has no Browser-only Session handoff and does not hide Browser from nested AgentSessions', () => {
    const source = readSource(new URL('./index.tsx', import.meta.url));
    expect(source).not.toContain('initial-browser-open');
    expect(source).not.toContain("sessionStorage.getItem");
    expect(source).toContain('<BrowserPanel panelId={browserPanelId} agentSessionId={conversation_id}');
    expect(source).toContain('<BrowserLinkContext.Provider value={conversation_id && browserLinkAvailable ? openBrowserLink : null}>');
    expect(source).toContain('hostSurfaceAvailable={isDesktopRuntime}');
    expect(source).toContain('onLinkAvailabilityChange={setBrowserLinkAvailable}');
  });

  test('keeps the existing composer stop action in the header while browser focus hides chat', () => {
    const source = readSource(new URL('./index.tsx', import.meta.url));
    expect(source).toContain('<StopButtonHostContext.Provider value={browserFocus ? stopButtonHost : null}>');
    expect(source).toContain("{browserFocus && <div ref={setStopButtonHost} className='chat-focus-stop-host' />}");
    expect(source).toContain("display: browserFocus ? 'none' : undefined");
    expect(source).toContain('if (!browserFocus) return undefined');
    expect(source).toContain('document.getElementById(browserPanelId)');
    expect(source).toContain("if (active?.closest('[role=\"dialog\"], [aria-modal=\"true\"]')) return");
    expect(source).toContain('if (panel && !panel.contains(active)) panel.focus()');
  });

  test('keeps the stable header controls', () => {
    const source = readSource(new URL('./index.tsx', import.meta.url));

    expect(source.includes("<AutoWorkControl target={{ kind: 'conversation', id: conversation_id }} />")).toBe(true);
    expect(source.includes('IdmmControl')).toBe(false);
    expect(source.includes('(props.knowledgeEnabled ?? true) && (')).toBe(true);
    expect(source.includes("<KnowledgeControl target={{ kind: 'conversation', id: conversation_id }} />")).toBe(true);
  });

  test('does not let workspace file-tree events auto-expand the conversation right rail', () => {
    const source = readSource(new URL('./index.tsx', import.meta.url));

    expect(source.includes('autoExpandOnFiles: false')).toBe(true);
  });

  test('keeps the workspace tool rail at the far right of the expanded panel', () => {
    const source = readSource(new URL('./index.tsx', import.meta.url));
    const panelIndex = source.indexOf("className={classNames('!bg-1 relative chat-layout-right-sider layout-sider')}");
    const railIndex = source.indexOf('<WorkspaceToolRail');

    expect(panelIndex >= 0).toBe(true);
    expect(railIndex >= 0).toBe(true);
    expect(panelIndex < railIndex).toBe(true);
  });
});
