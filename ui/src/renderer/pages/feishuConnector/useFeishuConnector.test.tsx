import { afterEach, expect, spyOn, test } from 'bun:test';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { feishuConnector } from '@/common/adapter/feishuConnectorBridge';
import type { FeishuQueryResult } from '@/common/types/feishuConnector';
import { useFeishuQuery } from './useFeishuConnector';

afterEach(cleanup);
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(yes => { resolve = yes; });
  return { resolve, promise };
}
const result = (title: string): FeishuQueryResult => ({
  items: [], fields: [], has_more: false, next_cursor: null, warnings: [title],
});
test('a replaced connection cannot receive a stale response for the same document list', async () => {
  const first = deferred<FeishuQueryResult>(), second = deferred<FeishuQueryResult>();
  const query = spyOn(feishuConnector, 'query').mockImplementationOnce(() => first.promise).mockImplementationOnce(() => second.promise);
  try {
    const view = renderHook(({ owner }) => useFeishuQuery({ kind: 'documents', query: '', filter: 'all', cursor: null }, owner), { initialProps: { owner: 'application-one/user-one' } });
    await waitFor(() => expect(query).toHaveBeenCalledTimes(1));
    view.rerender({ owner: 'application-two/user-two' });
    await waitFor(() => expect(query).toHaveBeenCalledTimes(2));
    expect(query.mock.calls[0][1]?.aborted).toBe(true);
    await act(async () => { first.resolve(result('Old identity content')); });
    expect(view.result.current.data).toBeNull();
    await act(async () => { second.resolve(result('Current identity content')); });
    expect(view.result.current.data?.warnings).toEqual(['Current identity content']);
  } finally { query.mockRestore(); }
});
