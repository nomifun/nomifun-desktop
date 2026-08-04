/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

describe('Nomi companion tab order', () => {
  test('merges collection and learning into one shared tab with the requested section order', () => {
    const source = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');
    const learn = readFileSync(new URL('./tabs/LearnTab.tsx', import.meta.url), 'utf8');
    const zhNomi = JSON.parse(
      readFileSync(new URL('../../services/i18n/locales/zh-CN/nomi.json', import.meta.url), 'utf8')
    );
    const registry = source.match(/const SHARED_TABS = \[(.*?)\] as const;/s)?.[1] ?? '';

    expect(registry.includes("'collect'")).toBe(true);
    expect(registry.includes("'learn'")).toBe(false);
    expect(source.includes("<Tabs.TabPane key='learn'")).toBe(false);
    expect(source.includes('collectionSection={<CollectTab shared={shared} />}')).toBe(true);
    expect(source.includes("rawTabParam === 'learn' ? 'collect'")).toBe(true);
    expect(zhNomi.tabs.collect).toBe('学习&知识共享');

    const learningSection = learn.indexOf("t('nomi.learn.sectionTitle')");
    const collectionSection = learn.indexOf('{collectionSection}');
    const skillSection = learn.indexOf("t('nomi.evolve.sectionTitle')");
    const collaborationSection = learn.indexOf("t('nomi.collaboration.title')");
    const archiveSection = learn.indexOf("t('nomi.archive.title')");

    expect(learningSection).toBeGreaterThan(-1);
    expect(collectionSection).toBeGreaterThan(learningSection);
    expect(skillSection).toBeGreaterThan(collectionSection);
    expect(collaborationSection).toBeGreaterThan(skillSection);
    expect(archiveSection).toBeGreaterThan(collaborationSection);
  });

  test('uses compact divider rows with an outline only around each category', () => {
    const collect = readFileSync(new URL('./tabs/CollectTab.tsx', import.meta.url), 'utf8');
    const learn = readFileSync(new URL('./tabs/LearnTab.tsx', import.meta.url), 'utf8');
    const layout = readFileSync(new URL('../../components/base/NomiSettingLayout.tsx', import.meta.url), 'utf8');

    expect(collect.includes('divide-y')).toBe(false);
    expect(learn.includes('divide-y')).toBe(false);
    expect(layout.includes('overflow-hidden rd-10px border border-solid border-[var(--color-border-2)] bg-transparent'))
      .toBe(true);
    expect(layout.includes('React.Children.toArray(children).map')).toBe(true);
    expect(layout.includes("index === 0 ? undefined : 'border-t border-t-solid border-t-[var(--color-border-2)]'"))
      .toBe(true);
    expect(layout.includes("<div className='min-w-0 flex-1'>")).toBe(true);
    expect(layout.includes('max-w-[62%] shrink-0 items-center justify-end')).toBe(true);
    expect(layout.includes('max-[760px]:max-w-full max-[760px]:justify-start')).toBe(true);
    expect((learn.match(/<NomiSettingSection/g) ?? []).length).toBe(4);
    expect((learn.match(/<NomiSettingRow/g) ?? []).length).toBe(10);
    expect(collect.includes('<NomiSettingSection')).toBe(true);
    expect(collect.includes('<NomiSettingList>')).toBe(true);
    expect((learn.match(/<NomiInputNumber/g) ?? []).length).toBe(4);
    expect((learn.match(/<NomiSelect/g) ?? []).length).toBeGreaterThanOrEqual(2);
    expect(learn.includes('w-220px')).toBe(false);
    expect(learn.includes('w-260px')).toBe(false);
    expect(collect.includes('w-112px')).toBe(false);
    expect(collect.indexOf("t('nomi.collect.disableAll'"))
      .toBeLessThan(collect.indexOf('<NomiSettingList>'));
    expect((collect.match(/t\('nomi\.collect\.disableAll'/g) ?? []).length).toBe(1);
    expect(collect.includes("import { Attention } from '@icon-park/react'")).toBe(true);
    expect(collect.includes("text-[rgb(var(--danger-6))]")).toBe(true);
    expect(collect.includes("gap-16px whitespace-nowrap")).toBe(true);
  });

  test('places remote connection immediately after overview', () => {
    const source = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');
    const registry = source.match(/const COMPANION_TABS = \[(.*?)\] as const;/s)?.[1] ?? '';

    expect(registry.indexOf("'overview'")).toBeLessThan(registry.indexOf("'remote'"));
    expect(registry.indexOf("'remote'")).toBeLessThan(registry.indexOf("'memories'"));

    const overviewRadio = source.indexOf("<Radio value='overview'>");
    const remoteRadio = source.indexOf("<Radio value='remote'>");
    const memoriesRadio = source.indexOf("<Radio value='memories'>");

    expect(overviewRadio).toBeGreaterThan(-1);
    expect(remoteRadio).toBeGreaterThan(overviewRadio);
    expect(memoriesRadio).toBeGreaterThan(remoteRadio);
  });

  test('does not expose retired credentials or overview diary UI', () => {
    const source = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');
    const overview = readFileSync(new URL('./tabs/OverviewTab.tsx', import.meta.url), 'utf8');

    expect(source.includes("'secrets'")).toBe(false);
    expect(source.includes('SecretsTab')).toBe(false);
    expect(overview.includes('listLearnRuns')).toBe(false);
    expect(overview.includes('nomi.overview.diary')).toBe(false);
  });

  test('groups the desktop companion switch and chat model under basic configuration', () => {
    const overview = readFileSync(new URL('./tabs/OverviewTab.tsx', import.meta.url), 'utf8');
    const modelControl = readFileSync(new URL('./CompanionModelControl.tsx', import.meta.url), 'utf8');
    const zhNomi = JSON.parse(
      readFileSync(new URL('../../services/i18n/locales/zh-CN/nomi.json', import.meta.url), 'utf8')
    );

    const basicConfig = overview.indexOf("t('nomi.overview.basicConfig')");
    const companionSwitch = overview.indexOf("t('nomi.settings.companionEnabled')");
    const chatModel = overview.indexOf("t('nomi.chat.modelConfig')");

    expect(zhNomi.overview.basicConfig).toBe('基础配置');
    expect(basicConfig).toBeGreaterThan(-1);
    expect(companionSwitch).toBeGreaterThan(basicConfig);
    expect(chatModel).toBeGreaterThan(companionSwitch);
    expect(overview.includes("<NomiSettingSection title={t('nomi.overview.basicConfig')}")).toBe(true);
    expect((overview.match(/<NomiSettingRow/g) ?? []).length).toBe(3);
    expect(overview.includes('<CompanionModelControl companion={companion} showLabel={false} />')).toBe(true);
    expect(modelControl.includes('showLabel?: boolean')).toBe(true);
    expect(modelControl.includes('{showLabel && (')).toBe(true);
    expect((modelControl.match(/contentFit/g) ?? []).length).toBeGreaterThanOrEqual(2);
    expect(modelControl.includes('w-148px')).toBe(false);
    expect(modelControl.includes('w-176px')).toBe(false);
    expect(overview.includes('w-180px shrink-0 max-[760px]:w-160px')).toBe(true);
    expect(overview.includes("className='!items-start !px-12px !py-9px [&_.arco-alert-icon]:!mt-1px'")).toBe(true);
    expect(overview.includes("<span className='text-14px leading-20px font-600'>")).toBe(true);
    expect(overview.includes("className='flex flex-col items-center gap-4px'")).toBe(true);
    expect(overview.includes('size={88}')).toBe(true);
  });

  test('organizes companion settings into compact outlined categories', () => {
    const settings = readFileSync(new URL('./tabs/SettingsTab.tsx', import.meta.url), 'utf8');
    const picker = readFileSync(new URL('./CharacterPicker.tsx', import.meta.url), 'utf8');
    const presetControl = readFileSync(
      new URL('../../components/preset/PresetApplyControl.tsx', import.meta.url),
      'utf8'
    );
    const nomiInput = readFileSync(new URL('../../components/base/NomiInput.tsx', import.meta.url), 'utf8');
    const nomiSelect = readFileSync(new URL('../../components/base/NomiSelect.tsx', import.meta.url), 'utf8');
    const layout = readFileSync(new URL('../../components/base/NomiSettingLayout.tsx', import.meta.url), 'utf8');
    const arcoOverride = readFileSync(new URL('../../styles/arco-override.css', import.meta.url), 'utf8');
    const zhNomi = JSON.parse(
      readFileSync(new URL('../../services/i18n/locales/zh-CN/nomi.json', import.meta.url), 'utf8')
    );

    expect(zhNomi.settings.basicSection).toBe('基础配置');
    expect(settings.indexOf("t('nomi.settings.basicSection')"))
      .toBeLessThan(settings.indexOf("t('nomi.settings.character')"));
    expect(settings.indexOf("t('nomi.settings.character')"))
      .toBeLessThan(settings.indexOf("t('nomi.settings.persona')"));
    expect(settings.indexOf("t('nomi.settings.persona')"))
      .toBeLessThan(settings.indexOf("t('nomi.settings.deleteCompanion')"));
    expect(settings.includes("t('nomi.settings.behaviorSection')")).toBe(false);
    expect(settings.includes("t('nomi.settings.danger')")).toBe(false);
    expect((settings.match(/<NomiSettingSection/g) ?? []).length).toBe(2);
    expect((settings.match(/<NomiSettingList/g) ?? []).length).toBe(3);
    expect((settings.match(/<NomiSettingRow/g) ?? []).length).toBe(5);
    expect(layout.includes("<div className='min-w-0 flex-1'>")).toBe(true);
    expect(layout.includes('max-w-[62%] shrink-0 items-center justify-end')).toBe(true);
    expect(settings.includes('<CharacterPicker\n            compact')).toBe(true);
    expect(picker.includes('compact?: boolean')).toBe(true);
    expect(picker.includes('size={compact ? 64 : 84}')).toBe(true);
    expect(picker.includes("'grid-cols-4 gap-8px")).toBe(true);
    expect(settings.includes("<NomiSelect.Option value='lively'>")).toBe(true);
    expect(settings.includes("<Radio value='lively'>")).toBe(false);
    expect(settings.includes('footer={')).toBe(true);
    expect(settings.includes('autoSize={{ minRows: 1, maxRows: 3 }}')).toBe(true);
    expect(settings.includes("!bg-[var(--color-bg-1)] !border-[var(--color-border-2)]")).toBe(true);
    expect(settings.includes("nomi-quiet-hours-picker !h-36px !w-260px shrink-0 !bg-[var(--color-bg-1)]")).toBe(
      true
    );
    expect(settings.includes("max-[760px]:!w-full")).toBe(true);
    expect(arcoOverride.includes('.nomi-quiet-hours-picker.arco-picker-focused')).toBe(true);
    expect(arcoOverride.includes('background: rgba(var(--primary-rgb), 0.06) !important')).toBe(true);
    expect(settings.includes('<NomiInput contentFit value={nameDraft}')).toBe(true);
    expect(settings.includes('contentWidthUnits')).toBe(false);
    expect(settings.includes("import { Attention } from '@icon-park/react'")).toBe(true);
    expect(settings.includes("className='line-height-0 shrink-0 text-[rgb(var(--danger-6))]'"))
      .toBe(true);
    expect(settings.includes('<PresetApplyControl\n                compact')).toBe(true);
    expect(settings.includes('w-380px')).toBe(false);
    expect(presetControl.includes('compact?: boolean')).toBe(true);
    expect(presetControl.includes('contentFit={compact}')).toBe(true);
    expect(presetControl.includes('contentMaxWidth={260}')).toBe(true);
    expect(nomiInput.includes('contentFit?: boolean')).toBe(true);
    expect(nomiInput.includes('autoWidth={contentFit ? { minWidth: contentMinWidth, maxWidth: contentMaxWidth } : autoWidth}'))
      .toBe(true);
    expect(nomiInput.includes("style={{ ...(contentFit ? { flex: 'none' } : undefined), ...style }}")).toBe(true);
    expect(nomiSelect.includes('contentFit?: boolean')).toBe(true);
    expect(nomiSelect.includes("width: 'max-content', minWidth: contentMinWidth, maxWidth: contentMaxWidth")).toBe(
      true
    );
    expect(nomiSelect.includes('autoAlignPopupWidth: false')).toBe(true);
  });

  test('reuses the unified compact setting layout across companion configuration surfaces', () => {
    const knowledge = readFileSync(new URL('./tabs/KnowledgeTab.tsx', import.meta.url), 'utf8');
    const remote = readFileSync(new URL('./tabs/RemoteConnectSection.tsx', import.meta.url), 'utf8');
    const migrate = readFileSync(new URL('./tabs/MigrateTab.tsx', import.meta.url), 'utf8');
    const memories = readFileSync(new URL('./tabs/MemoriesTab.tsx', import.meta.url), 'utf8');
    const skills = readFileSync(new URL('./tabs/SkillsTab.tsx', import.meta.url), 'utf8');

    expect(knowledge.includes('<NomiSettingList>')).toBe(true);
    expect(knowledge.includes('<NomiSettingRow')).toBe(true);
    expect(remote.includes('<NomiSettingSection')).toBe(true);
    expect(remote.includes('<NomiSettingList>')).toBe(true);
    expect(remote.includes('<NomiSettingRow')).toBe(true);
    expect(migrate.includes('<NomiSettingList')).toBe(true);
    expect(migrate.includes('<NomiSettingRow')).toBe(true);
    expect(migrate.includes('contentFit')).toBe(true);
    expect(memories.includes('contentFit')).toBe(true);
    expect(skills.includes('contentFit')).toBe(true);
  });

  test('removes learning history and manual learning UI while preserving scheduled learning and skill evolution', () => {
    const learn = readFileSync(new URL('./tabs/LearnTab.tsx', import.meta.url), 'utf8');
    const bridge = readFileSync(new URL('../../../common/adapter/ipcBridge.ts', import.meta.url), 'utf8');

    expect(learn.includes('listLearnRuns')).toBe(false);
    expect(learn.includes('ICompanionLearnRun')).toBe(false);
    expect(learn.includes('<Table')).toBe(false);
    expect(bridge.includes('/api/companion/learn/runs')).toBe(false);
    expect(bridge.includes('listLearnRuns')).toBe(false);

    expect(learn.includes('ipcBridge.companion.runLearn.invoke()')).toBe(false);
    expect(learn.includes("t('nomi.learn.runNow')")).toBe(false);
    expect(learn.includes('sharedConfig.learn.enabled')).toBe(true);
    expect(learn.includes('sharedConfig.evolve.enabled')).toBe(true);
    expect(bridge.includes('onLearnFinished')).toBe(true);
    expect(bridge.includes('ICompanionLearnResult')).toBe(true);
  });

  test('replaces raw event viewing and manual clearing with an explicit retention policy', () => {
    const collect = readFileSync(new URL('./tabs/CollectTab.tsx', import.meta.url), 'utf8');
    const bridge = readFileSync(new URL('../../../common/adapter/ipcBridge.ts', import.meta.url), 'utf8');

    expect(collect.includes('rawEvents')).toBe(false);
    expect(collect.includes('loadRawEvents')).toBe(false);
    expect(bridge.includes('/api/companion/events/recent')).toBe(false);
    expect(bridge.includes("httpDelete<void, void>('/api/companion/events')")).toBe(false);

    expect(bridge.includes("eventStorage: httpGet<ICompanionEventStorageStatus, void>('/api/companion/events/storage')"))
      .toBe(true);
    expect(collect.includes('event_retention_days: retentionDraft')).toBe(true);
    expect(collect.includes('event_max_storage_mb: capacityDraft')).toBe(true);
    expect(collect.includes('lowerPolicyConfirm')).toBe(true);
    expect(collect.includes('onOk={applyStoragePolicy}')).toBe(true);
    expect(collect.includes('min={7}')).toBe(true);
    expect(collect.includes('max={365}')).toBe(true);
    expect(collect.includes('min={16}')).toBe(true);
    expect(collect.includes('max={512}')).toBe(true);

    // Unknown/failed status must not be reported as an empty zero-byte store.
    expect(collect.includes('storage?.total_bytes ?? 0')).toBe(false);
    expect(collect.includes('storageError && !storageLoading ?')).toBe(true);
    expect(collect.includes('refreshStorage(true)')).toBe(true);
    expect(collect.includes('storageUnavailable')).toBe(true);
    expect(collect.includes('storageLoading')).toBe(true);

    const enNomi = JSON.parse(
      readFileSync(new URL('../../services/i18n/locales/en-US/nomi.json', import.meta.url), 'utf8')
    );
    const zhNomi = JSON.parse(
      readFileSync(new URL('../../services/i18n/locales/zh-CN/nomi.json', import.meta.url), 'utf8')
    );
    expect(enNomi.collect.storedRange_one.includes('{{count}} daily file)')).toBe(true);
    expect(enNomi.collect.storedRange_other.includes('{{count}} daily files)')).toBe(true);
    expect(Object.keys(enNomi.collect).sort()).toEqual(Object.keys(zhNomi.collect).sort());
  });
});
