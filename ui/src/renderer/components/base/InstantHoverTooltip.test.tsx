/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../test/setup-dom.ts';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import InstantHoverTooltip from './InstantHoverTooltip';

afterEach(cleanup);

describe('InstantHoverTooltip', () => {
  test.each(['拖动调整位置', '删除伙伴'])('shows %s outside the roster scroll and stacking boundaries', async (content) => {
    const view = render(
      <div data-testid='scroll-pane' style={{ overflow: 'auto', transform: 'translateX(0)', height: 56, width: 240 }}>
        <InstantHoverTooltip content={content}>
          <button>{content}</button>
        </InstantHoverTooltip>
      </div>
    );
    const trigger = view.getByRole('button');
    expect(view.queryByRole('tooltip')).toBeNull();
    fireEvent.mouseEnter(trigger.parentElement!);
    const tooltip = view.getByRole('tooltip', { name: content });
    expect(document.body.contains(tooltip)).toBe(true);
    expect(view.getByTestId('scroll-pane').contains(tooltip)).toBe(false);
    expect(tooltip.style.position).toBe('fixed');
    await waitFor(() => expect(tooltip.style.opacity).toBe('1'));
    fireEvent.mouseLeave(trigger.parentElement!);
    expect(view.queryByRole('tooltip')).toBeNull();
  });

  test('shows on keyboard focus and removes its portal on blur or unmount', async () => {
    const view = render(<InstantHoverTooltip content='伙伴信息'><button>信息</button></InstantHoverTooltip>);
    const trigger = view.getByRole('button');
    await act(async () => trigger.focus());
    expect(view.getByRole('tooltip', { name: '伙伴信息' })).toBeTruthy();
    await act(async () => trigger.blur());
    expect(view.queryByRole('tooltip')).toBeNull();
    await act(async () => trigger.focus());
    const tooltip = view.getByRole('tooltip');
    view.unmount();
    expect(document.body.contains(tooltip)).toBe(false);
  });

  test('clears an open tooltip when disabled and waits for a new interaction after re-enabling', async () => {
    const content = (disabled: boolean) => <InstantHoverTooltip content='删除伙伴' disabled={disabled}><button>删除</button></InstantHoverTooltip>;
    const view = render(content(false));
    const trigger = view.getByRole('button');
    fireEvent.mouseEnter(trigger.parentElement!);
    await act(async () => {});
    expect(view.getByRole('tooltip')).toBeTruthy();
    view.rerender(content(true));
    expect(view.queryByRole('tooltip')).toBeNull();
    fireEvent.mouseEnter(trigger.parentElement!);
    await act(async () => {});
    expect(view.queryByRole('tooltip')).toBeNull();
    view.rerender(content(false));
    expect(view.queryByRole('tooltip')).toBeNull();
    fireEvent.mouseEnter(trigger.parentElement!);
    await act(async () => {});
    expect(view.getByRole('tooltip')).toBeTruthy();
  });

  test('keeps trigger layout classes separate from portal appearance and preserves the theme token', async () => {
    const view = render(
      <InstantHoverTooltip content='伙伴名字' position='left' className='ml-auto' tooltipClassName='nomi-companion-switcher__tooltip'>
        <button>切换</button>
      </InstantHoverTooltip>
    );
    const wrapper = view.getByRole('button').parentElement!;
    fireEvent.mouseEnter(wrapper);
    await act(async () => {});
    const tooltip = view.getByRole('tooltip');
    expect(wrapper.classList.contains('ml-auto')).toBe(true);
    expect(wrapper.classList.contains('nomi-companion-switcher__tooltip')).toBe(false);
    expect(tooltip.classList.contains('nomi-companion-switcher__tooltip')).toBe(true);
    expect(tooltip.classList.contains('ml-auto')).toBe(false);
    expect(tooltip.className).toContain('bg-[var(--color-tooltip-bg)]');
  });
});
