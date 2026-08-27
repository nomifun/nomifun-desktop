import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const detailSource = readFileSync(new URL('./KnowledgeDetailPage/index.tsx', import.meta.url), 'utf8');
const stylesSource = readFileSync(new URL('../../styles/arco-override.css', import.meta.url), 'utf8');

describe('Knowledge settings visual layout', () => {
  test('reuses the shared setting row and input for the knowledge base name', () => {
    expect(detailSource.includes('<NomiSettingList>')).toBe(true);
    expect(detailSource.match(/<NomiSettingRow/g)?.length).toBe(5);
    expect(detailSource.includes('<NomiInput')).toBe(true);
    expect(detailSource.includes('contentFit')).toBe(true);
    expect(detailSource.includes('knowledge-settings-identity-card')).toBe(false);
    expect(stylesSource.includes('.knowledge-settings-inline-input')).toBe(false);
  });

  test('makes external-folder mutation consent explicit and reversible', () => {
    expect(detailSource.includes('folderEditAccess')).toBe(true);
    expect(detailSource.includes("editTreeAccess === 'editable'")).toBe(true);
    expect(detailSource.includes("checked ? 'editable' : 'read_only'")).toBe(true);
  });

  test('places selectable tag chips and the save action below the description', () => {
    const descriptionIndex = detailSource.indexOf('<NomiSettingSection');
    const tagsIndex = detailSource.indexOf('knowledge-settings-tags-section');
    const saveIndex = detailSource.indexOf('onClick={() => void handleSaveInfo()}');

    expect(detailSource.includes('<NomiSettingSection')).toBe(true);
    expect(detailSource.includes('knowledge-settings-description-input')).toBe(true);
    expect(detailSource.includes('autoSize={{ minRows: 3, maxRows: 10 }}')).toBe(true);
    expect(tagsIndex).toBeGreaterThan(descriptionIndex);
    expect(saveIndex).toBeGreaterThan(tagsIndex);
    expect(stylesSource.includes('.knowledge-settings-tag-picker .knowledge-studio-tag-chip-check')).toBe(true);
    expect(stylesSource.includes('border-color: rgba(var(--primary-6), 0.52)')).toBe(true);
    expect(stylesSource.includes('min-height: 96px')).toBe(true);
    expect(stylesSource.includes('border-radius: 10px !important')).toBe(true);
    expect(detailSource.includes('knowledge-settings-save-button')).toBe(true);
  });

  test('uses neutral focus styling and the shared setting row for source details', () => {
    expect(detailSource.includes("className='knowledge-settings-source-row'")).toBe(true);
    expect(detailSource.includes("t('knowledge.detail.settings.webHint'")).toBe(true);
    expect(stylesSource.includes('.knowledge-settings-layout .nomi-input.arco-input-focus')).toBe(true);
    expect(stylesSource.includes('box-shadow: 0 0 0 2px var(--color-fill-2)')).toBe(true);
  });

  test('uses shared setting rows and compact buttons for dangerous actions', () => {
    expect(detailSource.includes('knowledge-settings-danger-section')).toBe(true);
    expect(detailSource.includes('knowledge-detail-danger-panel')).toBe(false);
    expect(detailSource.match(/size='mini'/g)?.length).toBeGreaterThanOrEqual(2);
    expect(detailSource.includes("size='mini' status='danger'")).toBe(true);
    expect(stylesSource.includes('.knowledge-settings-danger-section')).toBe(true);
    expect(stylesSource.includes('margin-top: -8px')).toBe(true);
  });
});
