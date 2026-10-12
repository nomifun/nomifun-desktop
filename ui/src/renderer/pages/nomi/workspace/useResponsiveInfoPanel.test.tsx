/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback } from 'react';
import { act, cleanup, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, mock, test } from 'bun:test';
import { useContentSiderCollapse } from '@/renderer/components/layout/ContentSider';
import { useResponsiveInfoPanel } from './useResponsiveInfoPanel';

const STORAGE_KEY = 'test:nomi-responsive-info-panel';
const OriginalResizeObserver = globalThis.ResizeObserver;

class ObservedResize {
  static instances: ObservedResize[] = [];
  target: Element | null = null;
  disconnect = mock(() => {});
  constructor(private callback: ResizeObserverCallback) { ObservedResize.instances.push(this); }
  observe(target: Element) { this.target = target; }
  trigger() { this.callback([], this as unknown as ResizeObserver); }
}

beforeEach(() => {
  localStorage.removeItem(STORAGE_KEY);
  ObservedResize.instances = [];
  globalThis.ResizeObserver = ObservedResize as unknown as typeof ResizeObserver;
});

afterEach(() => {
  cleanup();
  localStorage.removeItem(STORAGE_KEY);
  globalThis.ResizeObserver = OriginalResizeObserver;
});

function mount(initialWidth: number, initiallyVisible = true) {
  let width = initialWidth;
  let state!: ReturnType<typeof useResponsiveInfoPanel>;
  function Harness({ visible }: { visible: boolean }) {
    const manual = useContentSiderCollapse(STORAGE_KEY, false);
    state = useResponsiveInfoPanel({ manuallyCollapsed: manual.collapsed, setManuallyCollapsed: manual.setCollapsed });
    const ref = useCallback((element: HTMLDivElement | null) => {
      if (element) element.getBoundingClientRect = () => ({ width } as DOMRect);
      state.containerRef(element);
    }, [state.containerRef]);
    return visible ? <div data-testid='workspace' ref={ref} /> : null;
  }
  const view = render(<Harness visible={initiallyVisible} />);
  return {
    get: () => state,
    resize(nextWidth: number) {
      width = nextWidth;
      act(() => ObservedResize.instances.at(-1)!.trigger());
    },
    toggle: () => act(() => state.toggle()),
    show: () => view.rerender(<Harness visible />),
    unmount: () => view.unmount(),
  };
}

describe('docked companion info panel', () => {
  test('measures the workspace and applies an 840/872 hysteresis without saving automatic hiding', () => {
    const workspace = mount(860);
    expect(workspace.get().collapsed).toBe(false);
    workspace.resize(839);
    expect(workspace.get().collapsed).toBe(true);
    expect(localStorage.getItem(STORAGE_KEY)).toBeNull();
    workspace.resize(840);
    expect(workspace.get().collapsed).toBe(true);
    workspace.resize(871);
    expect(workspace.get().collapsed).toBe(true);
    workspace.resize(872);
    expect(workspace.get().collapsed).toBe(false);
    workspace.resize(840);
    expect(workspace.get().collapsed).toBe(false);
    workspace.resize(839);
    expect(workspace.get().collapsed).toBe(true);
    expect(localStorage.getItem(STORAGE_KEY)).toBeNull();
  });

  test('preserves a manual collapse when resizing, returning to a wide workspace, and remounting', () => {
    const workspace = mount(1100);
    workspace.toggle();
    expect(localStorage.getItem(STORAGE_KEY)).toBe('collapsed');
    workspace.resize(810);
    workspace.resize(1200);
    expect(workspace.get().collapsed).toBe(true);
    workspace.unmount();
    const reopened = mount(1200);
    expect(reopened.get().collapsed).toBe(true);
    reopened.toggle();
    expect(reopened.get().collapsed).toBe(false);
    expect(localStorage.getItem(STORAGE_KEY)).toBe('expanded');
  });

  test('lets a manual restore survive narrow resizes but clears it after returning to the wide range', () => {
    const workspace = mount(820);
    expect(workspace.get().collapsed).toBe(true);
    workspace.toggle();
    expect(workspace.get().collapsed).toBe(false);
    workspace.resize(800);
    expect(workspace.get().collapsed).toBe(false);
    workspace.resize(872);
    expect(workspace.get().collapsed).toBe(false);
    workspace.resize(839);
    expect(workspace.get().collapsed).toBe(true);
    expect(localStorage.getItem(STORAGE_KEY)).toBe('expanded');
  });

  test('keeps 480px for chat and drops a manual restore when the workspace no longer fits the original side panel', () => {
    const workspace = mount(761);
    expect(workspace.get().canExpand).toBe(false);
    workspace.toggle();
    expect(workspace.get().collapsed).toBe(true);
    expect(localStorage.getItem(STORAGE_KEY)).toBeNull();
    workspace.resize(762);
    expect(workspace.get().canExpand).toBe(true);
    workspace.toggle();
    expect(workspace.get().collapsed).toBe(false);
    workspace.resize(761);
    expect(workspace.get().collapsed).toBe(true);
    workspace.resize(850);
    expect(workspace.get().collapsed).toBe(true);
  });

  test('starts observing when the workspace mounts after loading and disconnects on unmount', () => {
    const workspace = mount(820, false);
    expect(ObservedResize.instances).toHaveLength(0);
    workspace.show();
    expect(ObservedResize.instances).toHaveLength(1);
    expect(ObservedResize.instances[0]!.target).not.toBeNull();
    expect(workspace.get().collapsed).toBe(true);
    workspace.resize(1000);
    expect(workspace.get().collapsed).toBe(false);
    workspace.unmount();
    expect(ObservedResize.instances[0]!.disconnect).toHaveBeenCalledTimes(1);
  });
});
