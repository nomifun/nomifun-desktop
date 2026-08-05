import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const companionSource = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');
const companionCss = readFileSync(new URL('./companion.css', import.meta.url), 'utf8');
const capturePolicySource = readFileSync(new URL('./companionCapturePolicy.ts', import.meta.url), 'utf8');
const deskGeometrySource = readFileSync(new URL('./deskRestoreGeometry.ts', import.meta.url), 'utf8');

describe('desktop companion chrome layout', () => {
  test('keeps the figure stage as the only chrome on the stage shell', () => {
    const stageIndex = companionSource.indexOf("className='nomi-companion-stage'");
    const figureIndex = companionSource.indexOf('ref={figureHitRef}');
    expect(stageIndex).toBeGreaterThan(-1);
    expect(figureIndex).toBeGreaterThan(stageIndex);
  });

  test('retains shared native expansion only for chat surfaces', () => {
    expect(companionSource.includes("type ExpandedWindowMode = 'chat'")).toBe(true);
    expect(companionSource.includes('expandedWindowSessionRef')).toBe(true);
    expect(companionSource.includes('syncExpandedWindow(expandedMode)')).toBe(true);
    expect(companionSource.includes('internalWindowLayoutRef.current || expandedWindowSessionRef.current')).toBe(true);
    expect(companionSource.includes('MAX_WINDOW_RESTORE_RETRIES')).toBe(true);
  });

  test('keeps the desk-restore geometry the pet window still depends on', () => {
    expect(deskGeometrySource.includes('resolveDeskRestoreLayout')).toBe(true);
    expect(deskGeometrySource.includes('pickHostMonitor')).toBe(true);
    expect(companionSource.includes("from './deskRestoreGeometry'")).toBe(true);
  });

  test('removes every suggestion surface from the pet window', () => {
    // The 建议 feature is gone end to end: no unread badge, no detached
    // memory-panel window, no suggestion state or bridge calls.
    expect(companionCss.includes('.nomi-companion-suggestions')).toBe(false);
    expect(companionCss.includes('.nomi-companion-badge')).toBe(false);
    expect(companionCss.includes('is-memory-panel-open')).toBe(false);
    expect(capturePolicySource.includes('showSuggestions')).toBe(false);
    expect(companionSource.includes('useDetachedMemoryPanel')).toBe(false);
    expect(companionSource.includes('memoryPanel')).toBe(false);
    expect(companionSource.includes('listSuggestions')).toBe(false);
    expect(companionSource.includes('decideSuggestion')).toBe(false);
    expect(companionSource.includes('onSuggestionCreated')).toBe(false);
    expect(companionSource.includes('onSuggestionDecided')).toBe(false);
    expect(companionSource.includes('clear-unread')).toBe(false);
    expect(/\bunread\b/.test(companionSource)).toBe(false);
  });

  test('only chat-saved memories pop the memory bubble', () => {
    // 共享记忆删除后，每条记忆都有主人 —— 包括后台 learner 蒸馏出来的那些
    // (`source: 'learn'`)、主人在工作区手输的和 agent 通过 MCP 写的
    // (`'manual'` / `'merge'`)。没有这道 source 闸门，桌宠会开始为没人要求通知的
    // 后台动作弹气泡。
    expect(companionSource.includes("if (m.source !== 'chat') return;")).toBe(true);
    expect(companionSource.includes('m.scope_companion_id !== companionId')).toBe(true);
  });
});
