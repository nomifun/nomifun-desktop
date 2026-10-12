import { describe, expect, test } from 'bun:test';
import { BackendHttpError, BackendRequestError } from '@/common/adapter/httpBridge';
import { knowledgeErrorText, normalizeKnowledgeError } from './knowledgeError';

function http(code: string, status = 500, error = 'Internal error: failed to finalize export file: 拒绝访问。 (os error 5)') {
  return new BackendHttpError({
    method: 'POST',
    path: '/api/knowledge/bases/base-id/export',
    status,
    body: { success: false, code, error },
  });
}

describe('knowledge error presentation', () => {
  test('hides raw backend text and never infers permission from an OS error string', () => {
    const presentation = normalizeKnowledgeError(http('INTERNAL_ERROR'), 'export');
    expect(presentation.title).toBe('资料导出失败');
    expect(presentation.message).toBe('导出文件未能保存。请重新选择 ZIP 文件的保存位置后再试。');
    expect(presentation.message).not.toMatch(/Backend|500|os error|权限/);
    expect(presentation.technicalDetails).toContain('/api/knowledge/bases/base-id/export');
    expect(presentation.technicalDetails).toContain('os error 5');
  });

  test('uses stable scoped codes for actionable export recovery', () => {
    expect(knowledgeErrorText(http('KNOWLEDGE_EXPORT_ACCESS_DENIED'), 'export')).toContain('写入权限');
    expect(knowledgeErrorText(http('KNOWLEDGE_EXPORT_STORAGE_FULL'), 'export')).toContain('空间不足');
    expect(knowledgeErrorText(http('KNOWLEDGE_EXPORT_DESTINATION_INVALID', 400), 'export')).toContain('完整保存路径');
    expect(knowledgeErrorText(http('KNOWLEDGE_EXPORT_DESTINATION_EXISTS', 409), 'export')).toContain('同名文件');
    const sourceFailure = new BackendHttpError({
      method: 'POST', path: '/api/knowledge/bases/base-id/export', status: 500,
      body: { code: 'KNOWLEDGE_EXPORT_ACCESS_DENIED', error: 'Permission denied', details: { stage: 'read_files' } },
    });
    expect(knowledgeErrorText(sourceFailure, 'export')).toContain('源目录及文件的访问权限');
    expect(knowledgeErrorText(sourceFailure, 'export')).not.toContain('保存位置');
  });

  test('transport failure does not claim that an export or mutation had no effect', () => {
    const timeout = normalizeKnowledgeError(new BackendRequestError('timeout', 'Backend request timed out'), 'export');
    const network = normalizeKnowledgeError(new BackendRequestError('network', 'connection lost'), 'operation');
    expect(timeout.message).toContain('先检查保存位置是否已生成文件');
    expect(network.message).toContain('暂时无法确认操作结果');
    expect(network.message).toContain('知识库当前状态');
  });

  test('preserves redaction for technical copy and hides unknown object payloads', () => {
    const failure = new BackendHttpError({
      method: 'POST', path: '/api/knowledge/export?token=query-secret', status: 500,
      body: { code: 'INTERNAL_ERROR', error: 'fetch failed: Bearer private-bearer', details: { api_key: 'private-key' } },
    });
    const details = normalizeKnowledgeError(failure, 'export').technicalDetails;
    expect(details).not.toContain('query-secret');
    expect(details).not.toContain('private-bearer');
    expect(details).not.toContain('private-key');
    expect(details).toContain('[REDACTED]');
    expect(normalizeKnowledgeError({ password: 'secret' }).technicalDetails).toBe('Unknown error');
  });

  test('normalization is idempotent and records already handled authentication', () => {
    const presentation = normalizeKnowledgeError(new Error('unexpected failure'), 'load');
    expect(normalizeKnowledgeError(presentation)).toBe(presentation);
    const handled = { ...http('FORBIDDEN', 403), name: 'BackendHttpError', authExpiredHandled: true, message: 'authentication expired' };
    expect(normalizeKnowledgeError(handled).handled).toBe(true);
  });
});
