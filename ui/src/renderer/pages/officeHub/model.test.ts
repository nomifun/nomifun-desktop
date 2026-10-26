import { describe, expect, test } from 'bun:test';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import type { OfficeOperation, OfficeStatus } from '@/common/types/officeHub';
import { baseInputText, editableFieldValue, mutationTitle, operationKind, parseBaseLink, relatedAuthorization, relatedOperation, sameActor, scopesFor, selectFieldOptions, terminalAuth } from './model';

describe('compact office boundaries', () => {
  test('identity follows application and user, not display name', () => {
    const actor = { app_id: 'app', user_id: 'user', name: 'A' };
    expect(sameActor(actor, { ...actor, name: 'B' })).toBe(true);
    expect(sameActor(actor, { ...actor, user_id: 'other' })).toBe(false);
    expect(sameActor(actor, null)).toBe(false);
  });
  test('only missing exact capabilities contribute authorization scopes', () => {
    const status = { permissions: [{ capability: 'documents.create', granted: true, required_scopes: ['create'] }, { capability: 'documents.write', granted: false, required_scopes: ['edit'] }] } as OfficeStatus;
    expect(scopesFor(status, ['documents.write'])).toEqual(['edit']);
    expect(scopesFor(status, ['documents.create'])).toEqual([]);
  });
  test('opaque historic mutation evidence cannot become a current executable kind', () => {
    const old = { items: [{ mutation: { kind: 'document_update', document_id: 'doc' } }] } as unknown as OfficeOperation;
    expect(operationKind(old)).toBe('mixed');
    expect(mutationTitle({ title: { bad: 'shape' }, task_id: 'real-task' })).toBe('real-task');
    expect(mutationTitle({ title: ['untrusted'] })).toBe('');
  });
  test('structured related references preserve credential redaction', () => {
    const flow = '0190f5fe-7c00-7a00-8000-000000000901';
    const error = new BackendHttpError({ method: 'POST', path: '/api/office-hub/auth', status: 409, body: { details: { related_flow_id: flow, related_operation_id: 'original', authorization: 'Bearer private' } } });
    expect(relatedAuthorization(error)).toBe(flow);
    expect(relatedOperation(error)).toBe('original');
    expect(JSON.stringify(error.body)).not.toContain('private');
    expect(relatedAuthorization(new Error('related_flow_id=' + flow))).toBeNull();
  });
  test('cancelling remains observable while denied is terminal', () => {
    expect(terminalAuth('cancelling')).toBe(false);
    expect(terminalAuth('denied')).toBe(true);
  });
  test('Base URLs require official HTTPS and an exact table', () => {
    expect(parseBaseLink('https://team.feishu.cn/base/baseA?table=tblA')).toEqual({ base_token: 'baseA', table_id: 'tblA' });
    for (const value of ['http://team.feishu.cn/base/a?table=b', 'https://feishu.cn.evil.test/base/a?table=b', 'https://team.feishu.cn/wiki/a?table=b', 'https://team.feishu.cn/base/a', 'https://team.feishu.cn/base/a?table=b&table=c', 'https://team.feishu.cn/base/a?table=tbl%2Fbad']) expect(() => parseBaseLink(value)).toThrow();
  });
  test('typed Base inputs preserve platform values and names', () => {
    const field = { id: 'fld', name: 'Status', field_type: 'select', writable: true, options: [{ id: 'opt', name: 'Contacted' }, { id: 'opaque' }] };
    expect(selectFieldOptions(field)).toEqual([{ value: 'Contacted', label: 'Contacted' }]);
    expect(baseInputText(field, ['New', 'Contacted'])).toBe('["New","Contacted"]');
    expect(editableFieldValue(field, '["Contacted"]')).toEqual(['Contacted']);
    expect(editableFieldValue({ ...field, field_type: 'number' }, '12.5')).toBe(12.5);
    expect(() => editableFieldValue({ ...field, field_type: 'number' }, 'NaN')).toThrow();
  });
});
