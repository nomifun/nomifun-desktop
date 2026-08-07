import { describe, expect, test } from 'bun:test';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import { parsePresetId } from '@/common/types/ids';
import { buildPresetFromMarketPackage, classifyPresetPackageAddError } from './PresetPackageMarketSettings';

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
});

describe('preset package market errors', () => {
  const backendError = (code: string) =>
    new BackendHttpError({
      method: 'POST',
      path: '/api/skills/market/package/install',
      status: code === 'NOT_FOUND' ? 404 : 502,
      body: { success: false, code, error: 'redacted backend error' },
    });

  test('maps structured upstream failures to actionable messages', () => {
    expect(classifyPresetPackageAddError(backendError('TIMEOUT'))).toBe('timeout');
    expect(classifyPresetPackageAddError(backendError('BAD_GATEWAY'))).toBe('upstream');
    expect(classifyPresetPackageAddError(backendError('NOT_FOUND'))).toBe('not_found');
  });

  test('keeps unknown failures on the generic safe fallback', () => {
    expect(classifyPresetPackageAddError(backendError('INTERNAL_ERROR'))).toBe('generic');
    expect(classifyPresetPackageAddError(new TypeError('malformed response'))).toBe('generic');
  });
});
