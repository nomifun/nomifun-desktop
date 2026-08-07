/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const panelSource = readFileSync(new URL('./PresetListPanel.tsx', import.meta.url), 'utf8');
const presetCardSource = readFileSync(new URL('./PresetCard.tsx', import.meta.url), 'utf8');
const filterSource = readFileSync(new URL('./PresetTagFilterBar.tsx', import.meta.url), 'utf8');
const skillsSource = readFileSync(new URL('../SkillsHubSettings.tsx', import.meta.url), 'utf8');
const marketSource = readFileSync(new URL('../MarketSettingsPanel.tsx', import.meta.url), 'utf8');
const pluginSource = readFileSync(new URL('../../mcp/PluginSettingsPanel.tsx', import.meta.url), 'utf8');
const mcpInstalledSource = readFileSync(
  new URL('../../../components/settings/SettingsModal/contents/ToolsModalContent.tsx', import.meta.url),
  'utf8'
);

describe('Preset library compact toolbar', () => {
  test('keeps search, tag management, and creation in the filter action row', () => {
    expect(panelSource.includes("data-testid='input-search-preset'")).toBe(true);
    expect(panelSource.includes("data-testid='btn-manage-tags'")).toBe(true);
    expect(panelSource.includes("data-testid='btn-create-preset'")).toBe(true);
    expect(panelSource.includes('btn-search-toggle')).toBe(false);
  });

  test('uses dropdown facets and only echoes active selections below the toolbar', () => {
    expect(filterSource.includes("<Dropdown trigger='click' position='bl' droplist={menu}>")).toBe(true);
    expect(filterSource.includes("value={selectedSummary('audience')}")).toBe(true);
    expect(filterSource.includes("value={selectedSummary('scenario')}")).toBe(true);
    expect(filterSource.includes('{hasSelection && (')).toBe(true);
    expect(filterSource.includes('renderSelectedRow(')).toBe(true);
  });

  test('keeps preset and skill library surfaces transparent, outlined, and free of duplicate headings', () => {
    for (const source of [panelSource, skillsSource]) {
      expect(source.includes('border-[var(--color-border-2)] bg-transparent')).toBe(true);
      expect(source.includes('bg-fill-2 rounded-24px')).toBe(false);
    }

    expect(panelSource.includes("t('settings.presets', { defaultValue: 'Presets' })")).toBe(false);
    expect(skillsSource.includes("t('settings.skillsHub.gridTitle', { defaultValue: 'Skills' })")).toBe(false);
  });

  test('uses the shared enhanced-tools card background and border treatment for presets', () => {
    expect(presetCardSource.includes('rounded-16px border border-solid')).toBe(true);
    expect(presetCardSource.includes('border-[var(--color-border-2)] bg-[var(--color-bg-2)]')).toBe(true);
    expect(presetCardSource.includes('hover:border-[var(--color-primary-light-4)]')).toBe(true);
    expect(presetCardSource.includes('bg-[var(--color-bg-1)] hover:bg-[var(--color-fill-2)]')).toBe(false);
  });

  test('applies the same surface rule to every enhanced-tools market and MCP/plugin installed tab', () => {
    for (const source of [marketSource, pluginSource, mcpInstalledSource]) {
      expect(source.includes('border-[var(--color-border-2)]')).toBe(true);
      expect(source.includes('bg-transparent')).toBe(true);
    }

    expect(marketSource.includes('<h2')).toBe(false);
    expect(pluginSource.includes('<h2')).toBe(false);
    expect(marketSource.includes('bg-fill-2 rounded-24px')).toBe(false);
    expect(pluginSource.includes('bg-fill-2 rounded-24px')).toBe(false);
    expect(mcpInstalledSource.includes("data-testid='mcp-installed-surface'")).toBe(true);
  });

  test('keeps plain-market action groups on the description row', () => {
    expect(marketSource.includes("isMobile ? 'flex-col' : 'items-center justify-between'")).toBe(true);
    expect(marketSource.includes('{enableTagFilter ? marketIconActions : marketActions}')).toBe(true);
    expect(skillsSource.includes("isMobile ? 'flex-col' : 'items-center justify-between'")).toBe(true);
    expect(pluginSource.includes("className='flex items-center justify-between gap-12px mb-12px'")).toBe(true);
  });

  test('keeps tagged-market source controls on the filter row and moves reversed icon actions to the header', () => {
    const headerStart = marketSource.indexOf("data-testid={testId('{market}-header-row')}");
    const headerEnd = marketSource.indexOf('{isSearchVisible && (', headerStart);
    const headerBlock = marketSource.slice(headerStart, headerEnd);
    const filterStart = marketSource.indexOf('<PresetTagFilterBar', headerEnd);
    const filterEnd = marketSource.indexOf('/>', filterStart);
    const filterBlock = marketSource.slice(filterStart, filterEnd);
    const iconActionsStart = marketSource.indexOf('const marketIconActions');
    const iconActionsEnd = marketSource.indexOf('const marketActions', iconActionsStart);
    const iconActionsBlock = marketSource.slice(iconActionsStart, iconActionsEnd);

    expect(headerBlock.includes('{enableTagFilter ? marketIconActions : marketActions}')).toBe(true);
    expect(filterBlock.includes('actions={marketSourceSwitcher}')).toBe(true);
    expect(iconActionsBlock.indexOf("data-testid={testId('btn-search-{market}')}")).toBeLessThan(
      iconActionsBlock.indexOf("data-testid={testId('btn-sync-{market}')}")
    );
    expect(marketSource.includes("'ml-auto flex-none justify-end'")).toBe(true);
  });

  test('keeps one compact outer gap above every enhanced-tools surface', () => {
    for (const source of [panelSource, skillsSource, marketSource, pluginSource, mcpInstalledSource]) {
      expect(source.includes('mt-8px')).toBe(true);
    }
  });

  test('uses the same tighter vertical rhythm across enhanced-tools surfaces', () => {
    for (const source of [panelSource, skillsSource, marketSource]) {
      expect(source.includes("isMobile ? 'px-16px py-10px' : 'px-20px py-12px'")).toBe(true);
      expect(source.includes("className='flex flex-col gap-10px mb-12px'")).toBe(true);
    }

    expect(pluginSource.includes('px-16px py-10px md:px-20px md:py-12px')).toBe(true);
    expect(mcpInstalledSource.includes('py-[10px] md:py-[12px]')).toBe(true);
    expect(mcpInstalledSource.includes("className='flex flex-col gap-12px min-h-0'")).toBe(true);
  });

  test('keeps skill imports on the filter row and renders tag management as a tooltip icon', () => {
    const headerStart = skillsSource.indexOf("data-testid='skills-library-header-row'");
    const headerEnd = skillsSource.indexOf('{isSearchVisible && (', headerStart);
    const headerBlock = skillsSource.slice(headerStart, headerEnd);
    const filterStart = skillsSource.indexOf('<PresetTagFilterBar', headerEnd);
    const importActionsStart = skillsSource.indexOf("data-testid='skills-import-actions'", filterStart);

    expect(headerBlock.includes("data-testid='btn-refresh-skills'")).toBe(true);
    expect(headerBlock.includes("data-testid='btn-search-toggle'")).toBe(true);
    expect(headerBlock.indexOf("data-testid='btn-search-toggle'")).toBeLessThan(
      headerBlock.indexOf("data-testid='btn-refresh-skills'")
    );
    expect(headerBlock.includes("data-testid='btn-import-agent-skills'")).toBe(false);
    expect(importActionsStart).toBeGreaterThan(filterStart);
    expect(skillsSource.includes('manageTagsInlineIcon')).toBe(true);
    expect(skillsSource.includes("'ml-auto flex-none justify-end'")).toBe(true);
    expect(filterSource.includes("<Tooltip content={manageTagsLabel} position='top' mini>")).toBe(true);
    expect(filterSource.includes("'inline-flex h-34px w-34px")).toBe(true);
  });
});
