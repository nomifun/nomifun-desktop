import { afterEach, expect, mock, test } from 'bun:test';
import { act, cleanup, renderHook } from '@testing-library/react';
import type { PropsWithChildren } from 'react';
import { createContext } from './createContext';

afterEach(cleanup);

test('preserves an explicit false initial value', () => {
  const [useValue, Provider] = createContext(() => true);
  const wrapper = ({ children }: PropsWithChildren) => <Provider initialValue={false}>{children}</Provider>;
  const { result } = renderHook(useValue, { wrapper });
  expect(result.current).toBe(false);
});

test('local updates survive parent rerenders and changes to the initialization prop', () => {
  const [useValue, Provider, useUpdate] = createContext(() => 10);
  let initial = 1;
  const wrapper = ({ children }: PropsWithChildren) => <Provider initialValue={initial}>{children}</Provider>;
  const hook = renderHook(() => ({ value: useValue(), update: useUpdate() }), { wrapper });
  act(() => {
    hook.result.current.update((value) => value + 1);
    hook.result.current.update((value) => value + 1);
  });
  expect(hook.result.current.value).toBe(3);
  initial = 50;
  hook.rerender();
  expect(hook.result.current.value).toBe(3);
});

test.each([0, '', null])('preserves explicit falsy initial value %p', (initialValue) => {
  const [useValue, Provider] = createContext<string | number | null>(() => 'default');
  const wrapper = ({ children }: PropsWithChildren) => <Provider initialValue={initialValue}>{children}</Provider>;
  expect(renderHook(useValue, { wrapper }).result.current).toBe(initialValue);
});

test('creates isolated defaults once per provider mount without JSON cloning', () => {
  const initialize = mock(() => ({ entries: new Map<string, string>() }));
  const [useValue, Provider, useUpdate] = createContext(initialize);
  // Context's out-of-provider fallback is initialized at definition time.
  initialize.mockClear();
  const first = renderHook(() => ({ value: useValue(), update: useUpdate() }), { wrapper: Provider });
  const second = renderHook(useValue, { wrapper: Provider });
  expect(initialize).toHaveBeenCalledTimes(2);
  expect(first.result.current.value).not.toBe(second.result.current);
  expect(first.result.current.value.entries).toBeInstanceOf(Map);
  act(() => first.result.current.update((value) => ({ entries: new Map(value.entries).set('one', 'first') })));
  first.rerender();
  expect(first.result.current.value.entries.get('one')).toBe('first');
  expect(second.result.current.entries.size).toBe(0);
  expect(initialize).toHaveBeenCalledTimes(2);
  first.unmount();
  const remounted = renderHook(useValue, { wrapper: Provider });
  expect(remounted.result.current.entries.size).toBe(0);
  expect(initialize).toHaveBeenCalledTimes(3);
});
