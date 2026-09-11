/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL) => readFileSync(url, 'utf8');

describe('Guid resource cards placement', () => {
  test('renders resource cards in the stable primary stage and companion poster in the scroll discovery area', () => {
    const source = readSource(new URL('../GuidPage.tsx', import.meta.url));

    const inputIndex = source.indexOf('<GuidInputCard');
    const resourceIndex = source.indexOf('<GuidResourceCards', inputIndex);
    const primaryStageIndex = source.indexOf('className={styles.guidPrimaryStage}');
    const discoveryAreaIndex = source.indexOf('className={styles.guidDiscoveryArea}');
    const companionPreviewIndex = source.indexOf('<GuidCompanionPosterPreview', discoveryAreaIndex);

    expect(primaryStageIndex).toBeGreaterThan(-1);
    expect(inputIndex).toBeGreaterThan(-1);
    expect(resourceIndex).toBeGreaterThan(inputIndex);
    expect(discoveryAreaIndex).toBeGreaterThan(resourceIndex);
    expect(companionPreviewIndex).toBeGreaterThan(discoveryAreaIndex);
    expect(source.includes('onFillPrompt')).toBe(false);
  });

  test('anchors the primary stage so Agent-specific resource rows cannot shift the whole hero', () => {
    const styles = readSource(new URL('../index.module.css', import.meta.url));
    const primaryStage = styles.slice(
      styles.indexOf('.guidPrimaryStage {'),
      styles.indexOf('.guidDiscoveryArea {'),
    );

    expect(primaryStage.includes('justify-content: flex-start;')).toBe(true);
    expect(primaryStage.includes('padding: clamp(88px, 18vh, 180px) 10px 10px;')).toBe(true);
    expect(primaryStage.includes('margin-top: 0;')).toBe(true);
    expect(primaryStage.includes('margin-top: -5vh;')).toBe(false);
  });

  test('contains docs, promo video, and contact feedback cards without recent prompt data access', () => {
    const source = readSource(new URL('./GuidResourceCards.tsx', import.meta.url));

    expect(source.includes('https://www.nomifun.com/docs')).toBe(true);
    expect(source.includes('https://www.bilibili.com/video/BV1kwKZ6UE5X/')).toBe(true);
    expect(source.includes('https://youtu.be/AsEToBDFR9s')).toBe(true);
    expect(source.includes('https://youtu.be/gEDo5H0H0Pg')).toBe(false);
    expect(source.includes('https://www.nomifun.com/contact')).toBe(true);
    expect(source.includes('https://github.com/nomifun/nomifun-tauri/issues')).toBe(false);
    expect(source.includes('RECENT_PROMPT_LIMIT')).toBe(false);
    expect(source.includes('getConversationMessages')).toBe(false);
    expect(source.includes('useConversationHistoryContext')).toBe(false);
    expect(source.includes('useSWR')).toBe(false);
    expect(source.includes('onFillPrompt')).toBe(false);
  });

  test('companion poster renders real companion figures instead of status data', () => {
    const source = readSource(new URL('./GuidCompanionPosterPreview.tsx', import.meta.url));

    expect(source.includes('conversation.companionPoster.title')).toBe(true);
    expect(source.includes('CompanionAvatar')).toBe(true);
    expect(source.includes('useCompanions')).toBe(true);
    expect(source.includes('customFigureMetaOf')).toBe(true);
    expect(source.includes('activeSkillCount')).toBe(false);
    expect(source.includes('workspaceDir')).toBe(false);
    expect(source.includes('currentModelName')).toBe(false);
    expect(source.includes('guidCompanionStatusGrid')).toBe(false);
  });
});
