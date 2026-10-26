import { afterEach, expect, spyOn, test } from 'bun:test';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { officeHub } from '@/common/adapter/officeHubBridge';
import type { OfficeQueryResult } from '@/common/types/officeHub';
import { useOfficeQuery } from './useOfficeHub';

afterEach(cleanup);
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(yes => { resolve = yes; });
  return { resolve, promise };
}
const result = (title: string): OfficeQueryResult => ({
  items: [], fields: [], has_more: false, next_cursor: null, warnings: [],
  document: { id: 'same-resource', title, content: title, url: null, edit_baseline: null, pending_operation_id: null, pending_status: null },
});
test('a replaced connection cannot receive a stale response for the same document URL', async () => {
  const first = deferred<OfficeQueryResult>(), second = deferred<OfficeQueryResult>();
  const query = spyOn(officeHub, 'query').mockImplementationOnce(() => first.promise).mockImplementationOnce(() => second.promise);
  try {
    const view = renderHook(({ owner }) => useOfficeQuery({ kind: 'document', document: 'same-resource' }, owner), { initialProps: { owner: 'application-one/user-one' } });
    await waitFor(() => expect(query).toHaveBeenCalledTimes(1));
    view.rerender({ owner: 'application-two/user-two' });
    await waitFor(() => expect(query).toHaveBeenCalledTimes(2));
    expect(query.mock.calls[0][1]?.aborted).toBe(true);
    await act(async () => { first.resolve(result('Old identity content')); });
    expect(view.result.current.data).toBeNull();
    await act(async () => { second.resolve(result('Current identity content')); });
    expect(view.result.current.data?.document?.title).toBe('Current identity content');
  } finally { query.mockRestore(); }
});
