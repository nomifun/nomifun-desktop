import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const companionSource = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');
const companionCss = readFileSync(new URL('./companion.css', import.meta.url), 'utf8');

describe('desktop companion chrome layout', () => {
  test('anchors unread badge to the figure stage instead of the viewport top', () => {
    const stageIndex = companionSource.indexOf("className='nomi-companion-stage'");
    const badgeIndex = companionSource.indexOf("className='nomi-companion-badge'");
    const figureIndex = companionSource.indexOf('ref={figureHitRef}');

    expect(stageIndex).toBeGreaterThan(-1);
    expect(badgeIndex).toBeGreaterThan(stageIndex);
    expect(figureIndex).toBeGreaterThan(badgeIndex);
  });

  test('defines a positioned figure stage for stable badge and suggestions anchoring', () => {
    expect(companionCss.includes('.nomi-companion-stage')).toBe(true);
    expect(companionCss.includes('position: relative;')).toBe(true);
    expect(companionCss.includes('top: 10px;')).toBe(false);
  });

  test('includes the memory panel in expanded native-window layout state', () => {
    expect(companionSource.includes('expandedWindowSessionRef')).toBe(true);
    expect(companionSource.includes("showSuggestions ? 'memory'")).toBe(true);
    expect(companionSource.includes('syncExpandedWindow(expandedMode)')).toBe(true);
  });

  test('does not persist programmatic expanded-window moves as the companion position', () => {
    expect(companionSource.includes('internalWindowLayoutRef.current || expandedWindowSessionRef.current')).toBe(true);
  });

  test('restores the captured desk rectangle even when the profile is being disabled', () => {
    const syncStart = companionSource.indexOf('const syncExpandedWindow');
    const restoreIndex = companionSource.indexOf('if (!mode)', syncStart);
    const enabledGuardIndex = companionSource.indexOf('if (!cfg || !cfg.appearance.companion_enabled)', syncStart);

    expect(restoreIndex).toBeGreaterThan(syncStart);
    expect(enabledGuardIndex).toBeGreaterThan(restoreIndex);
    expect(companionSource.includes('session.anchor.width, session.anchor.height')).toBe(true);
    expect(companionSource.includes('session.anchor.x, session.anchor.y')).toBe(true);
  });

  test('renders the memory panel as a sibling before the figure stage', () => {
    const panelIndex = companionSource.indexOf("id='nomi-companion-memory-panel'");
    const stageIndex = companionSource.indexOf("className='nomi-companion-stage'");

    expect(panelIndex).toBeGreaterThan(-1);
    expect(panelIndex).toBeLessThan(stageIndex);
  });

  test('uses semantic controls for the unread badge and suggestion items', () => {
    expect(companionSource.includes("aria-controls='nomi-companion-memory-panel'")).toBe(true);
    expect(companionSource.includes('aria-expanded={showSuggestions}')).toBe(true);
    expect(companionSource.includes("className='nomi-companion-suggestions__item'")).toBe(true);
  });

  test('shows full suggestion content inside a bounded scroller', () => {
    expect(companionCss.includes('max-height: var(--memory-panel-max-height')).toBe(true);
    expect(companionCss.includes('overflow-y: auto')).toBe(true);
    expect(companionCss.includes('-webkit-line-clamp')).toBe(false);
    expect(companionCss.includes('overflow-wrap: anywhere')).toBe(true);
  });

  test('uses monitor work areas and monitor-specific scale for expanded placement', () => {
    expect(companionSource.includes('monitor.workArea.position')).toBe(true);
    expect(companionSource.includes('monitor.workArea.size')).toBe(true);
    expect(companionSource.includes('monitor.scaleFactor')).toBe(true);
    expect(companionSource.includes('hostMonitorId')).toBe(true);
    expect(companionSource.includes('resolveDeskRestoreLayout')).toBe(true);
  });

  test('applies the calculated stage offset so edge clamping does not move the figure', () => {
    expect(companionSource.includes('memoryPanelStageShiftX')).toBe(true);
    expect(companionSource.includes("'--memory-stage-shift-x'" )).toBe(true);
    expect(companionCss.includes('translateX(var(--memory-stage-shift-x')).toBe(true);
  });

  test('checks the achieved native size before moving the expanded window', () => {
    const setSizeIndex = companionSource.indexOf('await win.setSize(new PhysicalSize(targetRect.width, targetRect.height))');
    const achievedIndex = companionSource.indexOf('const achieved = await win.outerSize()', setSizeIndex);
    const setPositionIndex = companionSource.indexOf(
      'await win.setPosition(new PhysicalPosition(targetRect.x, targetRect.y))',
      setSizeIndex
    );

    expect(setSizeIndex).toBeGreaterThan(-1);
    expect(achievedIndex).toBeGreaterThan(setSizeIndex);
    expect(setPositionIndex).toBeGreaterThan(achievedIndex);
  });

  test('keeps a bounded retry path when native desk-size restoration is delayed', () => {
    expect(companionSource.includes('expandedWindowRestoreRetriesRef')).toBe(true);
    expect(companionSource.includes('MAX_WINDOW_RESTORE_RETRIES')).toBe(true);
    expect(companionSource.includes('syncExpandedWindow(null)')).toBe(true);
  });

  test('cancels stale restore retries when a new expanded mode is requested', () => {
    expect(companionSource.includes('expandedWindowRequestedModeRef')).toBe(true);
    expect(companionSource.includes('expandedWindowRestoreTimerRef')).toBe(true);
    expect(companionSource.includes('clearTimeout(expandedWindowRestoreTimerRef.current)')).toBe(true);
    expect(companionSource.includes('expandedWindowRequestedModeRef.current !== null')).toBe(true);
  });
});
