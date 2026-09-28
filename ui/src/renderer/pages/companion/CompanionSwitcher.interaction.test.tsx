import '../../../../test/setup-dom.ts';
import { useState } from 'react';
import { act, cleanup, fireEvent, render, within } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import type { ICompanionWithStatus } from '@/common/adapter/ipcBridge';
import { parseCompanionId, type CompanionId } from '@/common/types/ids';
import nomi from '@/renderer/services/i18n/locales/en-US/nomi.json';
import CompanionSwitcher from './CompanionSwitcher';

const i18n = createInstance();
await i18n.init({ lng: 'en-US', resources: { 'en-US': { translation: { nomi } } }, interpolation: { escapeValue: false } });
const id = (n: number) => parseCompanionId(`019f0000-0000-7000-8000-${String(n).padStart(12, '0')}`);
const companions = Array.from({ length: 6 }, (_, n) => ({
  companion_id: id(n), name: `Partner ${n}`, character: 'mochi',
}) as ICompanionWithStatus);
const triggerName = i18n.t('nomi.companion.switchCompanion');
const switchName = (n: number) => i18n.t('nomi.companion.switchTo', { name: `Partner ${n}` });

function mount({ roster = companions, pending = null, current = id(0) }: {
  roster?: ICompanionWithStatus[]; pending?: CompanionId | null; current?: CompanionId;
} = {}) {
  const switched: CompanionId[] = [];
  let showAllCount = 0;
  function Harness() {
    const [open, setOpen] = useState(false);
    return <I18nextProvider i18n={i18n}>
      <CompanionSwitcher companionId={current} profile={null} roster={roster} switchingCompanionId={pending}
        open={open} onOpenChange={setOpen} onSwitch={(value) => switched.push(value)} onShowAll={() => showAllCount++} />
      <input aria-label='Draft' defaultValue='Keep this draft' />
    </I18nextProvider>;
  }
  return { ...render(<Harness />), switched, showAllCount: () => showAllCount };
}
afterEach(cleanup);

describe('compact vertical companion switcher', () => {
  test('shows three alternatives and routes overflow to the existing full picker', () => {
    const view = mount();
    const trigger = view.getByRole('button', { name: triggerName });
    expect(trigger.getAttribute('aria-expanded')).toBe('false');
    expect(view.queryByRole('group')).toBeNull();
    fireEvent.click(trigger);
    const rail = view.getByRole('group');
    expect(within(rail).getAllByRole('button')).toHaveLength(4);
    expect(within(rail).queryByRole('button', { name: switchName(0) })).toBeNull();
    expect(view.getByRole('button', { name: switchName(3) })).toBeTruthy();
    expect(view.queryByRole('button', { name: switchName(4) })).toBeNull();
    expect(rail.hasAttribute('data-companion-hit')).toBe(true);
    fireEvent.click(view.getByText('+2'));
    expect(view.showAllCount()).toBe(1);
    expect(view.switched).toEqual([]);
    expect(trigger.getAttribute('aria-expanded')).toBe('false');
    expect((view.getByRole('textbox') as HTMLInputElement).value).toBe('Keep this draft');
  });

  test('switches the chosen companion, closes the rail and restores trigger focus', () => {
    const view = mount();
    const trigger = view.getByRole('button', { name: triggerName });
    fireEvent.click(trigger);
    const choice = view.getByRole('button', { name: switchName(2) });
    fireEvent.mouseEnter(choice.parentElement!);
    const tooltip = view.getByRole('tooltip', { name: 'Partner 2' });
    expect(tooltip.textContent).toBe('Partner 2');
    expect(tooltip.className).toContain('right-[calc(100%+8px)]');
    fireEvent.mouseLeave(choice.parentElement!);
    expect(view.queryByRole('tooltip', { name: 'Partner 2' })).toBeNull();
    fireEvent.click(choice);
    expect(view.switched).toEqual([id(2)]);
    expect(view.queryByRole('group')).toBeNull();
    expect(document.activeElement).toBe(trigger);
    expect(view.container.querySelector('[data-companion-hit]')).toBeNull();
  });

  test('dismisses on outside pointer, Escape and loss of native window focus', () => {
    const view = mount();
    const trigger = view.getByRole('button', { name: triggerName });
    fireEvent.click(trigger);
    fireEvent.pointerDown(view.getByRole('textbox'));
    expect(trigger.getAttribute('aria-expanded')).toBe('false');
    fireEvent.click(trigger);
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(trigger.getAttribute('aria-expanded')).toBe('false');
    expect(document.activeElement).toBe(trigger);
    fireEvent.click(trigger);
    fireEvent.blur(window);
    expect(trigger.getAttribute('aria-expanded')).toBe('false');
  });

  test('repeated hover and keyboard focus changes keep the open rail and avatars mounted', () => {
    const view = mount();
    const trigger = view.getByRole('button', { name: triggerName });
    fireEvent.click(trigger);
    const rail = view.getByRole('group');
    const first = view.getByRole('button', { name: switchName(1) });
    const second = view.getByRole('button', { name: switchName(2) });
    const firstAvatar = first.firstElementChild;
    for (let index = 0; index < 10; index++) {
      fireEvent.mouseEnter(first.parentElement!);
      expect(view.getByRole('tooltip', { name: 'Partner 1' })).toBeTruthy();
      fireEvent.mouseLeave(first.parentElement!);
      fireEvent.mouseEnter(second.parentElement!);
      act(() => second.focus());
      fireEvent.mouseLeave(second.parentElement!);
      act(() => first.focus());
      expect(view.getByRole('group')).toBe(rail);
      expect(first.firstElementChild).toBe(firstAvatar);
      expect(trigger.getAttribute('aria-expanded')).toBe('true');
      expect(rail.hasAttribute('data-companion-hit')).toBe(true);
    }
    expect(view.switched).toEqual([]);
    expect(view.showAllCount()).toBe(0);
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(view.queryByRole('tooltip', { name: 'Partner 1' })).toBeNull();
  });

  test('keeps navigation in the column and prevents switching while a switch is pending', () => {
    const view = mount();
    fireEvent.click(view.getByRole('button', { name: triggerName }));
    const first = view.getByRole('button', { name: switchName(1) });
    act(() => first.focus());
    fireEvent.keyDown(first, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(view.getByRole('button', { name: switchName(2) }));
    view.unmount();
    const pending = mount({ pending: id(2) });
    fireEvent.click(pending.getByRole('button', { name: triggerName }));
    const rail = pending.getByRole('group');
    for (const button of within(rail).getAllByRole('button')) expect(button.hasAttribute('disabled')).toBe(true);
    fireEvent.click(pending.getByRole('button', { name: switchName(1) }));
    expect(pending.switched).toEqual([]);
  });

  test('handles a missing current roster entry and omits the entry for a lone companion', () => {
    const view = mount({ current: id(99), roster: companions.slice(0, 4) });
    fireEvent.click(view.getByRole('button', { name: triggerName }));
    expect(view.getByText('+1')).toBeTruthy();
    view.unmount();
    expect(mount({ roster: [companions[0]] }).queryByRole('button')).toBeNull();
  });
});
