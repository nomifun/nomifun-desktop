import { afterEach, beforeAll, describe, expect, mock, test } from 'bun:test';
import { cleanup, fireEvent, render, within } from '@testing-library/react';
import { createElement as h } from 'react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import type { SkillInfo } from '@/common/types/skill';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import SkillCard from './SkillCard';

const locale = createInstance();
beforeAll(async () => {
  await locale.init({ lng: 'en-US', resources: { 'en-US': { translation: { common, settings } } } });
});
afterEach(cleanup);

const custom: SkillInfo = { name: 'example-skill', description: 'Reusable instructions', location: '/skills/example/SKILL.md', is_custom: true, source: 'custom' };
function card(skill: SkillInfo, onOpenDetails = mock(() => {}), onDelete = mock(() => {}), isAutoInjected = false) {
  return h(I18nextProvider, { i18n: locale }, h(SkillCard, { skill, localeKey: 'en-US', isAutoInjected, onOpenDetails, onDelete }));
}

describe('SkillCard interaction ownership', () => {
  test('opens details with mouse or keyboard and keeps deletion a separate visible action', () => {
    const open = mock(() => {});
    const remove = mock(() => {});
    const view = render(card(custom, open, remove));
    const item = view.getByTestId('skill-card-example-skill');
    const details = within(item).getByRole('button', { name: /Skill Details: example-skill/ });
    fireEvent.click(details);
    fireEvent.keyDown(details, { key: 'Enter' });
    fireEvent.keyDown(details, { key: ' ' });
    expect(open).toHaveBeenCalledTimes(3);
    expect(open).toHaveBeenLastCalledWith(custom);

    fireEvent.click(within(item).getByRole('button', { name: 'Delete example-skill' }));
    expect(remove).toHaveBeenCalledWith(custom);
    expect(open).toHaveBeenCalledTimes(3);
    expect(within(item).queryByRole('switch')).toBeNull();
    expect(within(item).getByText('Custom')).toBeTruthy();
  });

  test('keeps built-in and auto-injected skills inspectable without delete or enable controls', () => {
    const builtin: SkillInfo = { ...custom, source: 'builtin', is_custom: false };
    const view = render(card(builtin));
    expect(view.queryByTestId('btn-delete-example-skill')).toBeNull();
    expect(view.getByText('Built-in')).toBeTruthy();
    expect(view.queryByRole('switch')).toBeNull();
    view.rerender(card(builtin, mock(() => {}), mock(() => {}), true));
    expect(view.getByText(settings.skillsHub.sourceAuto)).toBeTruthy();
    expect(view.getByRole('button', { name: /Skill Details: example-skill/ })).toBeTruthy();
    expect(view.queryByTestId('btn-delete-example-skill')).toBeNull();
  });
});
