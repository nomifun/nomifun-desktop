import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'fs';

const titlebarSource = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');
const languageMenuSource = readFileSync(new URL('./TitlebarLanguageMenu.tsx', import.meta.url), 'utf8');

describe('Titlebar instant icon tooltips', () => {
  test('uses the local instant hover tooltip for icon-only titlebar actions', () => {
    expect(titlebarSource.includes('InstantHoverTooltip')).toBe(true);
    expect(languageMenuSource.includes('InstantHoverTooltip')).toBe(true);
    expect(titlebarSource.includes("position='bottom'")).toBe(true);
    expect(languageMenuSource.includes("position='bottom'")).toBe(true);
  });

  test('does not use native title fallbacks for titlebar icon buttons', () => {
    expect(titlebarSource.includes('title={historyBackTooltip}')).toBe(false);
    expect(titlebarSource.includes('title={historyForwardTooltip}')).toBe(false);
    expect(titlebarSource.includes("title={t('terminal.newConversation')}")).toBe(false);
    expect(titlebarSource.includes("title={t('terminal.newTerminal')}")).toBe(false);
    expect(titlebarSource.includes('title={sessionToggleTooltip}')).toBe(false);
    expect(languageMenuSource.includes('title={label}')).toBe(false);
  });

  test('shows the workspace titlebar toggle on mobile only', () => {
    expect(titlebarSource.includes('const showWorkspaceButton = workspaceAvailable && Boolean(layout?.isMobile);')).toBe(
      true
    );
  });

  test('places the readable language selector after the left navigation actions', () => {
    const languageIndex = titlebarSource.indexOf('<TitlebarLanguageMenu');
    const sessionToggleIndex = titlebarSource.indexOf('tooltip: sessionToggleTooltip');

    expect(languageIndex).toBeGreaterThan(sessionToggleIndex);
    expect(languageMenuSource.includes('app-titlebar__language-button')).toBe(true);
    expect(languageMenuSource.includes('app-titlebar__language-name')).toBe(true);
    expect(languageMenuSource.includes('Translate')).toBe(false);
  });

  test('keeps shared history and quick-create navigation behind Creative Studio save gates', () => {
    expect(titlebarSource.includes('requestCreativeStudioBeforeLeave')).toBe(true);
    expect(titlebarSource.includes('navigateAfterCreativeStudioFlush')).toBe(true);
    expect(titlebarSource.includes('navigateAfterCreativeStudioFlush(() => navigationHistory?.back())')).toBe(true);
    expect(titlebarSource.includes('navigateAfterCreativeStudioFlush(() => navigationHistory?.forward())')).toBe(true);
  });
});
