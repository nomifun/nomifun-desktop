import '../../../../test/setup-dom.ts';
import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, expect, test } from 'bun:test';
import { MemoryRouter, useLocation, useNavigate } from 'react-router-dom';
import { NavigationHistoryProvider, useNavigationHistory } from './NavigationHistoryContext';

afterEach(cleanup);
function mount() {
  return renderHook(() => ({ history: useNavigationHistory()!, navigate: useNavigate(), location: useLocation() }), {
    wrapper: ({ children }) => <MemoryRouter initialEntries={['/agent']}><NavigationHistoryProvider>{children}</NavigationHistoryProvider></MemoryRouter>,
  });
}

test('the production titlebar history preserves an explicit replacement across a batched departure and two back steps', () => {
  const v = mount();
  const snapshot = { agentEditorReturn: { preset: 'a', name: 'Unsaved name' } };
  act(() => {
    v.result.current.history.replaceCurrent('/agent?preset=a', snapshot);
    void v.result.current.navigate('/plugins/create/check', { state: { author: 'check' } });
  });
  act(() => { void v.result.current.navigate('/plugins/run/check?saved=1'); });
  act(() => v.result.current.history.back());
  expect(v.result.current.location.pathname).toBe('/plugins/create/check');
  expect(v.result.current.location.state).toEqual({ author: 'check' });
  act(() => v.result.current.history.back());
  expect(v.result.current.location.pathname + v.result.current.location.search).toBe('/agent?preset=a');
  expect(v.result.current.location.state).toEqual(snapshot);
  expect(v.result.current.history.canBack).toBe(false);
  act(() => v.result.current.history.forward());
  expect(v.result.current.location.pathname).toBe('/plugins/create/check');
  expect(v.result.current.location.state).toEqual({ author: 'check' });
  act(() => v.result.current.history.forward());
  expect(v.result.current.location.search).toBe('?saved=1');
  expect(v.result.current.history.canForward).toBe(false);
});

test('consuming same-route state replaces the stored value without another entry or later resurrection', () => {
  const v = mount();
  act(() => { void v.result.current.navigate('/agent', { replace: true, state: { transient: true } }); });
  expect(v.result.current.history.canBack).toBe(false);
  act(() => { void v.result.current.navigate('/models'); });
  act(() => v.result.current.history.back());
  expect(v.result.current.location.state).toEqual({ transient: true });
  act(() => { void v.result.current.navigate('/agent', { replace: true, state: null }); });
  act(() => v.result.current.history.forward());
  act(() => v.result.current.history.back());
  expect(v.result.current.location.state).toBeNull();
  expect(v.result.current.history.canBack).toBe(false);
});

test('history remains bounded to 50 entries and a new navigation discards forward entries', () => {
  const v = mount();
  for (let index = 1; index <= 55; index++) act(() => { void v.result.current.navigate(`/page/${index}`, { state: { index } }); });
  for (let index = 0; index < 49; index++) act(() => v.result.current.history.back());
  expect(v.result.current.location.pathname).toBe('/page/6');
  expect(v.result.current.location.state).toEqual({ index: 6 });
  expect(v.result.current.history.canBack).toBe(false);
  expect(v.result.current.history.canForward).toBe(true);
  act(() => { void v.result.current.navigate('/new-page'); });
  expect(v.result.current.history.canForward).toBe(false);
  act(() => v.result.current.history.back());
  expect(v.result.current.location.pathname).toBe('/page/6');
});
