import { afterEach, expect, spyOn, test } from 'bun:test';
import { act, cleanup, renderHook } from '@testing-library/react';
import { feishuConnector } from '@/common/adapter/feishuConnectorBridge';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import { CANONICAL_UUID_V7 } from '@/common/types/ids';
import type { ExecuteRequest, FeishuMutation, FeishuOperation } from '@/common/types/feishuConnector';
import { useFeishuWrite } from './useFeishuWrite';

const actor = { app_id: 'app', user_id: 'user', name: 'User' };
const identity = { actor, profile: 'work', key: 'connection:work:app:user' };
const items: FeishuMutation[] = [{ kind: 'document_create', client_item_id: 'logical-create', title: 'A document' }];
function receipt(request: ExecuteRequest, status: FeishuOperation['status']): FeishuOperation {
  return { id: 'operation', request_id: request.request_id, connection_id: 'connection', actor, profile: 'work', status, items: request.items.map((mutation, index) => ({ id: String(index), mutation, target_key: 'target', status: status === 'partial' ? index === 0 ? 'succeeded' : 'unknown' : status, remote_id: null, remote_url: null, message: null, attempts: [], observations: [] })) };
}
afterEach(cleanup);
test('a lost response reads its exact request and never replays even changed input', async () => {
  let original!: ExecuteRequest;
  const execute = spyOn(feishuConnector, 'execute').mockImplementation(async request => { original = request; throw new Error('response lost'); });
  const read = spyOn(feishuConnector, 'request').mockImplementation(async () => receipt(original, 'unknown'));
  try {
    const view = renderHook(useFeishuWrite);
    await act(async () => { await view.result.current.save(items, identity); });
    expect(CANONICAL_UUID_V7.test(original.request_id)).toBe(true);
    expect(read.mock.calls[0][0]).toBe(original.request_id);
    expect(view.result.current.reset()).toBe(false);
    await act(async () => { await view.result.current.save([{ ...items[0], title: 'Changed input' } as FeishuMutation], identity); });
    expect(execute).toHaveBeenCalledTimes(1);
    expect(read).toHaveBeenCalledTimes(2);
    expect(view.result.current.locked).toBe(true);
  } finally { execute.mockRestore(); read.mockRestore(); }
});
test('positive not-executed proof permits another explicit save with a fresh request and stable logical ID', async () => {
  const execute = spyOn(feishuConnector, 'execute').mockImplementation(async request => receipt(request, 'not_executed'));
  try {
    const view = renderHook(useFeishuWrite);
    await act(async () => { await view.result.current.save(items, identity); });
    expect(execute).toHaveBeenCalledTimes(1);
    expect(view.result.current.locked).toBe(false);
    act(() => { expect(view.result.current.reset()).toBe(true); });
    expect(view.result.current.result).toBeNull();
    await act(async () => { await view.result.current.save(items, identity); });
    expect(execute.mock.calls[0][0].request_id).not.toBe(execute.mock.calls[1][0].request_id);
    expect(execute.mock.calls[1][0].items[0].client_item_id).toBe('logical-create');
    expect(execute.mock.calls[1][0].expected_actor).toEqual(actor);
  } finally { execute.mockRestore(); }
});
test('success stays locked and another save only observes the original request', async () => {
  let original!: ExecuteRequest;
  const execute = spyOn(feishuConnector, 'execute').mockImplementation(async request => { original = request; return receipt(request, 'succeeded'); });
  const read = spyOn(feishuConnector, 'request').mockImplementation(async () => receipt(original, 'succeeded'));
  try {
    const view = renderHook(useFeishuWrite);
    await act(async () => { await view.result.current.save(items, identity); });
    expect(view.result.current.reset()).toBe(false);
    await act(async () => { await view.result.current.save(items, identity); });
    expect(execute).toHaveBeenCalledTimes(1);
    expect(view.result.current.locked).toBe(true);
  } finally { execute.mockRestore(); read.mockRestore(); }
});
test('a missing original receipt is not proof of non-execution', async () => {
  const execute = spyOn(feishuConnector, 'execute').mockRejectedValue(new Error('connection lost'));
  const read = spyOn(feishuConnector, 'request').mockRejectedValue(new BackendHttpError({ method: 'GET', path: '/api/feishu-connector/operations/request/x', status: 404, body: { error: 'Not found' } }));
  try {
    const view = renderHook(useFeishuWrite);
    await act(async () => { await view.result.current.save(items, identity); });
    await act(async () => { await view.result.current.save(items, identity); });
    expect(execute).toHaveBeenCalledTimes(1);
    expect(view.result.current.locked).toBe(true);
  } finally { execute.mockRestore(); read.mockRestore(); }
});
test('a partially completed Base operation cannot reissue any of its original items', async () => {
  let original!: ExecuteRequest;
  const execute = spyOn(feishuConnector, 'execute').mockImplementation(async request => { original = request; return receipt(request, 'partial'); });
  const read = spyOn(feishuConnector, 'request').mockImplementation(async () => receipt(original, 'partial'));
  const batch: FeishuMutation[] = ['one','two'].map(id => ({ kind: 'base_update', client_item_id: id, base_token: 'base', table_id: 'table', record_id: id, fields: { fld: 'new' }, before: { fld: 'old' } }));
  try {
    const view = renderHook(useFeishuWrite);
    await act(async () => { await view.result.current.save(batch, identity); });
    await act(async () => { await view.result.current.save(batch, identity); });
    expect(execute).toHaveBeenCalledTimes(1); expect(read).toHaveBeenCalledTimes(1); expect(view.result.current.locked).toBe(true);
  } finally { execute.mockRestore(); read.mockRestore(); }
});
