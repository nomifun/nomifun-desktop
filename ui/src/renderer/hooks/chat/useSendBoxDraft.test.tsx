import '../../../../test/setup-dom.ts';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, expect, test } from 'bun:test';
import type { ReactNode } from 'react';
import { SWRConfig } from 'swr';
import { parseConversationId } from '@/common/types/ids';
import { getSendBoxDraftHook } from './useSendBoxDraft';

afterEach(cleanup);
const useDraft = getSendBoxDraftHook('nomi', { _type: 'nomi', content: '', atPath: [], uploadFile: [] });
const wrapper = ({ children }: { children: ReactNode }) => <SWRConfig value={{ provider: () => new Map() }}>{children}</SWRConfig>;

test('preserves isolated drafts across switching and remounts, and clears explicitly', async () => {
  const firstId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000981');
  const secondId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000982');
  const cache = new Map();
  const scopedWrapper = ({ children }: { children: ReactNode }) => (
    <SWRConfig value={{ provider: () => cache, dedupingInterval: 0 }}>{children}</SWRConfig>
  );
  const hook = renderHook(({ id }) => useDraft(id), {
    initialProps: { id: firstId }, wrapper: scopedWrapper,
  });
  await act(async () => {
    hook.result.current.mutate(draft => ({ ...draft, content: 'first' }));
    hook.result.current.mutate(draft => ({ ...draft, uploadFile: ['one.txt'] }));
  });
  expect(hook.result.current.data?.content).toBe('first');
  expect(hook.result.current.data?.uploadFile).toEqual(['one.txt']);

  hook.rerender({ id: secondId });
  await act(async () => {
    hook.result.current.mutate(draft => ({ ...draft, content: 'second' }));
  });
  expect(hook.result.current.data?.uploadFile).toEqual([]);
  hook.rerender({ id: firstId });
  await waitFor(() => expect(hook.result.current.data?.content).toBe('first'));
  expect(hook.result.current.data?.uploadFile).toEqual(['one.txt']);
  hook.unmount();

  // A fresh SWR cache must still hydrate from the conversation-owned store.
  cache.clear();
  const remounted = renderHook(() => useDraft(firstId), { wrapper: scopedWrapper });
  await waitFor(() => expect(remounted.result.current.data?.content).toBe('first'));
  await act(async () => { remounted.result.current.mutate(() => undefined); });
  expect(remounted.result.current.data).toBeUndefined();
  await act(async () => {
    remounted.result.current.mutate(draft => ({ ...draft, content: 'new' }));
  });
  expect(remounted.result.current.data?.uploadFile).toEqual([]);
  await act(async () => { remounted.result.current.mutate(() => undefined); });
});

test('a late original draft mutation cannot follow the hook into another Session cache key', async () => {
  const first = parseConversationId('0190f5fe-7c00-7a00-8000-000000000977');
  const second = parseConversationId('0190f5fe-7c00-7a00-8000-000000000978');
  const hook = renderHook(({ id }) => useDraft(id), { wrapper, initialProps: { id: first } });
  act(() => hook.result.current.mutate(draft => ({ ...draft, content: 'Original draft' })));
  await waitFor(() => expect(hook.result.current.data?.content).toBe('Original draft'));
  const originalMutate = hook.result.current.mutate;
  hook.rerender({ id: second });
  act(() => hook.result.current.mutate(draft => ({ ...draft, content: 'Keep new Session draft' })));
  await waitFor(() => expect(hook.result.current.data?.content).toBe('Keep new Session draft'));
  act(() => originalMutate(draft => ({ ...draft, content: 'Late original Session update' })));
  await act(async () => {});
  expect(hook.result.current.data?.content).toBe('Keep new Session draft');
  hook.rerender({ id: first });
  await waitFor(() => expect(hook.result.current.data?.content).toBe('Late original Session update'));
});
