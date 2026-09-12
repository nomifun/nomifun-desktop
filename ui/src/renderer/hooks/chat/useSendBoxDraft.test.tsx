import { afterEach, describe, expect, test } from 'bun:test';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { SWRConfig } from 'swr';
import type { PropsWithChildren } from 'react';
import { parseConversationId } from '@/common/types/ids';
import { getSendBoxDraftHook } from './useSendBoxDraft';

const useDraft = getSendBoxDraftHook('nomi', {
  _type: 'nomi', content: '', atPath: [], uploadFile: [],
});
const firstId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000981');
const secondId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000982');

afterEach(cleanup);

describe('conversation draft storage', () => {
  test('preserves isolated drafts across switching and remounts, and clears explicitly', async () => {
    const cache = new Map();
    const wrapper = ({ children }: PropsWithChildren) => (
      <SWRConfig value={{ provider: () => cache, dedupingInterval: 0 }}>{children}</SWRConfig>
    );
    const hook = renderHook(({ id }) => useDraft(id), {
      initialProps: { id: firstId }, wrapper,
    });
    await act(async () => {
      hook.result.current.mutate((draft) => ({ ...draft, content: 'first' }));
      hook.result.current.mutate((draft) => ({ ...draft, uploadFile: ['one.txt'] }));
    });
    expect(hook.result.current.data?.content).toBe('first');
    expect(hook.result.current.data?.uploadFile).toEqual(['one.txt']);

    hook.rerender({ id: secondId });
    await act(async () => {
      hook.result.current.mutate((draft) => ({ ...draft, content: 'second' }));
    });
    expect(hook.result.current.data?.uploadFile).toEqual([]);
    hook.rerender({ id: firstId });
    await waitFor(() => expect(hook.result.current.data?.content).toBe('first'));
    expect(hook.result.current.data?.uploadFile).toEqual(['one.txt']);
    hook.unmount();

    // A fresh SWR cache must still hydrate from the conversation-owned store.
    cache.clear();
    const remounted = renderHook(() => useDraft(firstId), { wrapper });
    await waitFor(() => expect(remounted.result.current.data?.content).toBe('first'));
    await act(async () => { remounted.result.current.mutate(() => undefined); });
    expect(remounted.result.current.data).toBeUndefined();
    await act(async () => {
      remounted.result.current.mutate((draft) => ({ ...draft, content: 'new' }));
    });
    expect(remounted.result.current.data?.uploadFile).toEqual([]);
    await act(async () => { remounted.result.current.mutate(() => undefined); });
  });
});
