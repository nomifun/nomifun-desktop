import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { parsePresetId } from '@/common/types/ids';
import { buildPresetFromMarketPackage } from './PresetPackageMarketSettings';

describe('preset package market import payload', () => {
  test('keeps real package skills and applies expert package defaults', () => {
    const presetId = parsePresetId('0190f5fe-7c00-7a00-8000-000000000231');
    const preset = buildPresetFromMarketPackage(
      {
        name: 'Test Automation',
        description: 'Testing workflow package',
        instructions: '---\nname: tech-test-automation\nmetadata:\n  author: SkillHub\n---\n# Test Automation',
        skill_slugs: ['name', 'superpowers-tdd', 'description', 'superpowers-tdd', 'test-case-generator'],
      },
      'zh-CN',
      presetId
    );

    expect(preset.preset_id).toBe('0190f5fe-7c00-7a00-8000-000000000231');
    expect(preset.targets).toEqual(['conversation', 'execution_step']);
    expect(preset.included_skills).toEqual([
      { skill_name: 'superpowers-tdd', required: false },
      { skill_name: 'test-case-generator', required: false },
    ]);
    expect((preset.instructions || '').includes('metadata:')).toBe(true);
    expect(preset.instructions_i18n?.['zh-CN']).toBe(preset.instructions);
  });

  test('keeps installed skill names that are not market slugs', () => {
    const preset = buildPresetFromMarketPackage(
      {
        name: 'Localized Skills',
        description: 'Package with installed skill names',
        instructions: '# Localized Skills',
        skill_slugs: [
          '中文技能',
          'skill with space',
          '',
          '   ',
          '中文技能',
          '../escape',
          'folder\\escape',
          'bad\u0000name',
          'name',
        ],
      },
      'zh-CN',
      undefined,
      'installed'
    );

    expect(preset.included_skills).toEqual([
      { skill_name: '中文技能', required: false },
      { skill_name: 'skill with space', required: false },
    ]);
  });

  test('enables collaboration auto-selection after a successful expert package import', () => {
    const source = readFileSync(new URL('./PresetPackageMarketSettings.tsx', import.meta.url), 'utf8');

    expect(source.includes('parsePresetId(uuidv7())')).toBe(true);
    expect(source.includes('presets.setState.invoke')).toBe(true);
    expect(source.includes('auto_selectable: true')).toBe(true);
    expect(source.includes('settings.presetMarket.stateUpdateFailed')).toBe(true);
    expect(source.includes('Failed to refresh imported expert package list:')).toBe(true);
  });

  test('installs package skills before importing the preset', () => {
    const source = readFileSync(new URL('./PresetPackageMarketSettings.tsx', import.meta.url), 'utf8');

    expect(source.includes('installSkillMarketPackage.invoke')).toBe(true);
    expect(source.includes('skill_slugs: installedPackage.installed_skill_names')).toBe(true);
    expect(source.includes('resolveSkillMarketPackage.invoke')).toBe(false);
  });

  test('warns when expert package is added with partial skill install failures', () => {
    const source = readFileSync(new URL('./PresetPackageMarketSettings.tsx', import.meta.url), 'utf8');

    expect(source.includes('installedPackage.errors?.length')).toBe(true);
    expect(source.includes('settings.presetMarket.partialSkillInstall')).toBe(true);
    expect(source.includes('skills: failedSkillNames')).toBe(true);
    expect(source.includes('Message.warning')).toBe(true);
  });

  test('stops before importing a preset when every package skill fails', () => {
    const source = readFileSync(new URL('./PresetPackageMarketSettings.tsx', import.meta.url), 'utf8');
    const allFailedGuard = source.indexOf(
      'installedPackage.installed_skill_names.length === 0 && failedSkillCount > 0'
    );
    const presetImport = source.indexOf('ipcBridge.presets.import.invoke');

    expect(allFailedGuard).toBeGreaterThan(-1);
    expect(allFailedGuard).toBeLessThan(presetImport);
    expect(source.includes('settings.presetMarket.skillInstallFailed')).toBe(true);
  });
});
