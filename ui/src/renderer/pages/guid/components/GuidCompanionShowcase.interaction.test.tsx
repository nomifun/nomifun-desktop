import '../../../../../test/setup-dom.ts';
import { act, cleanup, fireEvent, render, within } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import type { ICompanionWithStatus } from '@/common/adapter/ipcBridge';
import { parseCompanionId } from '@/common/types/ids';
import guid from '@/renderer/services/i18n/locales/en-US/guid.json';
import { GuidCompanionShowcaseView, type GuidCompanionShowcaseProps } from './GuidCompanionShowcase';
import CustomFigure from '../../companion/characters/CustomFigure';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { guid } } }, interpolation: { escapeValue: false } });
const id = (index: number) => parseCompanionId(`019f0000-0000-7000-8000-${String(index).padStart(12, '0')}`);
const companion = (index: number, enabled = false) => ({
  companion_id: id(index), name: `Companion ${index}`, character: 'mochi',
  appearance: { companion_enabled: enabled },
}) as ICompanionWithStatus;
const noop = () => {};
const mount = (props: Partial<GuidCompanionShowcaseProps> = {}) => render(<I18nextProvider i18n={i18n}>
  <GuidCompanionShowcaseView companions={[]} onRetry={noop} onCreate={noop} onManage={noop} onOpenChat={noop} onToggleFloating={noop} {...props} />
  <textarea aria-label='Draft' defaultValue='Keep my unsent work' />
</I18nextProvider>);
afterEach(() => { cleanup(); localStorage.clear(); });
const settle = () => act(async () => { await new Promise((resolve) => setTimeout(resolve, 30)); });

describe('home companion showcase', () => {
  test('empty and unavailable rosters are distinct; no sample companion is presented as user data', () => {
    let created = 0;
    const view = mount({ onCreate: () => created++ });
    expect(view.queryByText('Companion 1')).toBeNull();
    fireEvent.click(view.getAllByRole('button', { name: 'Create companion' })[0]);
    expect(created).toBe(1);
    view.unmount();
    let retries = 0;
    const error = mount({ error: new Error('offline'), onRetry: () => retries++ });
    expect(error.getByRole('alert').textContent).toContain('Could not load companions');
    expect(error.queryByRole('button', { name: 'Create companion' })).toBeNull();
    fireEvent.click(error.getByRole('button', { name: 'Retry' }));
    expect(retries).toBe(1);
  });
  test('selecting a figure previews it; only Open chat starts its conversation', async () => {
    const opened: string[] = [];
    const view = mount({ companions: [companion(1)], onOpenChat: (item) => opened.push(item.companion_id) });
    fireEvent.click(view.getByRole('button', { name: 'Companion 1' }));
    await settle();
    expect(opened).toEqual([]);
    fireEvent.click(within(document.body).getByRole('button', { name: 'Open chat' }));
    expect(opened).toEqual([id(1)]);
    expect((view.getByRole('textbox', { name: 'Draft' }) as HTMLTextAreaElement).value).toBe('Keep my unsent work');
  });
  test('search discovers partners outside the first pane and promotes the chosen partner', async () => {
    const view = mount({ companions: Array.from({ length: 8 }, (_, index) => companion(index + 1)) });
    expect(view.queryByRole('button', { name: 'Companion 8' })).toBeNull();
    fireEvent.click(view.getByRole('button', { name: 'All companions · 8' }));
    await settle();
    fireEvent.change(within(document.body).getByRole('textbox', { name: 'Search companions' }), { target: { value: 'Companion 8' } });
    fireEvent.click(within(document.body).getByRole('button', { name: 'Companion 8' }));
    await settle();
    expect(view.getByRole('button', { name: 'Companion 8' }).getAttribute('aria-pressed')).toBe('true');
    expect(within(document.body).queryByRole('textbox', { name: 'Search companions' })).toBeNull();
  });
  test('collapse preserves the draft and remembers the manual preference', () => {
    const view = mount({ companions: [companion(1)] });
    fireEvent.click(view.getByRole('button', { name: 'Collapse' }));
    expect(view.getByRole('button', { name: 'Expand' }).getAttribute('aria-expanded')).toBe('false');
    expect((view.getByRole('textbox', { name: 'Draft' }) as HTMLTextAreaElement).value).toBe('Keep my unsent work');
    view.unmount();
    expect(mount({ companions: [companion(1)] }).getByRole('button', { name: 'Expand' })).toBeTruthy();
  });
  test('desktop floating switches toggle the intended companion directly in both showcase and roster', async () => {
    const toggled: Array<[string, boolean]> = [];
    const opened: string[] = [];
    const view = mount({
      companions: [companion(1), companion(2, true)],
      onToggleFloating: (item, enabled) => toggled.push([item.companion_id, enabled]),
      onOpenChat: (item) => opened.push(item.companion_id),
    });
    const first = view.getByRole('switch', { name: 'Float Companion 1 on desktop' });
    expect(first.getAttribute('aria-checked')).toBe('false');
    fireEvent.click(first);
    expect(toggled).toEqual([[id(1), true]]);
    expect(opened).toEqual([]);
    expect(within(document.body).queryByRole('dialog', { name: 'Companion 1' })).toBeNull();

    fireEvent.click(view.getByRole('button', { name: 'All companions · 2' }));
    await settle();
    const roster = within(document.body).getByRole('dialog', { name: 'All companions · 2' });
    const second = within(roster).getByRole('switch', { name: 'Float Companion 2 on desktop' });
    expect(second.getAttribute('aria-checked')).toBe('true');
    fireEvent.click(second);
    expect(toggled).toEqual([[id(1), true], [id(2), false]]);
    expect(within(document.body).getByRole('dialog', { name: 'All companions · 2' })).toBeTruthy();
  });
  test('a pending desktop visibility change shows its target state and prevents a second click', () => {
    const toggled: boolean[] = [];
    const view = mount({ companions: [companion(1)], pendingFloating: { [id(1)]: true }, onToggleFloating: (_item, enabled) => toggled.push(enabled) });
    const toggle = view.getByRole('switch', { name: 'Float Companion 1 on desktop' });
    expect(toggle.getAttribute('aria-checked')).toBe('true');
    expect(toggle.hasAttribute('disabled')).toBe(true);
    fireEvent.click(toggle);
    expect(toggled).toEqual([]);
  });
  test('more actions operate on their own companion without selecting it or closing the parent prematurely', async () => {
    const managed: string[] = [];
    const view = mount({ companions: [companion(1), companion(2)], onManage: (selected) => { if (selected) managed.push(selected); } });
    fireEvent.click(view.getByRole('button', { name: 'All companions · 2' }));
    await settle();
    const parent = within(document.body).getByRole('dialog', { name: 'All companions · 2' });
    fireEvent.click(within(parent).getByRole('button', { name: 'More actions for Companion 2' }));
    await settle();
    expect(within(parent).getByRole('button', { name: 'Companion 2' }).getAttribute('aria-pressed')).toBe('false');
    const menu = within(parent).getByRole('dialog', { name: 'More actions for Companion 2' });
    fireEvent.click(within(menu).getByRole('button', { name: 'Manage companions' }));
    expect(managed).toEqual([id(2)]);
    expect(within(document.body).queryByRole('dialog', { name: 'All companions · 2' })).toBeNull();
  });
  test('the roster exposes a close button and preserves the selected highlight on reopening', async () => {
    const view = mount({ companions: [companion(1), companion(2)] });
    fireEvent.click(view.getByRole('button', { name: 'Companion 2' }));
    await settle();
    fireEvent.click(view.getByRole('button', { name: 'All companions · 2' }));
    await settle();
    const roster = within(document.body).getByRole('dialog', { name: 'All companions · 2' });
    expect(within(roster).getByRole('button', { name: 'Companion 2' }).getAttribute('aria-pressed')).toBe('true');
    fireEvent.click(within(roster).getByRole('button', { name: 'Close companion list' }));
    expect(within(document.body).queryByRole('dialog', { name: 'All companions · 2' })).toBeNull();
  });
  test('small full-body slots do not silently use the head crop, while avatars still do', () => {
    const props = { src: '/custom.png', aspect: 2.8, headBox: { x: .2, y: .1, w: .4, h: .6 }, size: 60, mood: 'content' as const, activity: 'idle' as const };
    const view = render(<CustomFigure {...props} displayMode='full' />);
    expect(view.container.querySelector('.nomi-cfig--bust')).toBeNull();
    expect((view.container.firstElementChild as HTMLElement).style.width).toBe('168px');
    view.rerender(<CustomFigure {...props} />);
    expect(view.container.querySelector('.nomi-cfig--bust')).not.toBeNull();
    expect((view.container.firstElementChild as HTMLElement).style.width).toBe('60px');
  });
});
