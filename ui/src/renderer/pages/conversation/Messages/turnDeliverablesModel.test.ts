/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { IMessageToolCall } from '@/common/chat/chatLib';
import { parseMessageId, parsePersistedArtifactId } from '@/common/types/ids';
import type { PersistedToolArtifact } from '@/common/types/platform/toolCallTypes';
import type { TurnFileSummary } from '@/common/types/platform/turnFiles';
import { buildTurnFileItems, collectVerifiedTurnImages, isVerifiedImageDeliverable, type TurnGateInfo } from './turnDeliverablesModel';

const turn = parseMessageId('0190f5fe-7c00-7a00-8000-000000000001');
const messageId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000021');
const artifactId = parsePersistedArtifactId('0190f5fe-7c00-7a00-8000-000000000031');
const workspace = '/work/session';
const imageReceipt = (overrides: Partial<PersistedToolArtifact> = {}): PersistedToolArtifact => ({
  id: artifactId, kind: 'image', mime_type: 'image/png', path: `${workspace}/picture.png`,
  relative_path: 'picture.png', size_bytes: 1024, sha256: 'a'.repeat(64), ...overrides,
});
const tool = (artifacts: PersistedToolArtifact[], overrides: Partial<IMessageToolCall['content']> = {}): IMessageToolCall => ({
  id: 'local-tool', conversation_id: 'conv', type: 'tool_call', message_id: messageId,
  turn_id: turn, content: { call_id: 'call-1', name: 'image', status: 'completed', artifacts, ...overrides },
}) as unknown as IMessageToolCall;
const gates = (state: TurnGateInfo['state'] = 'completed') => new Map([[turn, { running: state === 'running', state }]]);
const images = (artifacts: PersistedToolArtifact[], state: TurnGateInfo['state'] = 'completed') =>
  collectVerifiedTurnImages([{ turnId: turn, toolMessages: [tool(artifacts)] }], gates(state), [workspace]).get(turn) ?? [];
const summary = (files: TurnFileSummary['files']): TurnFileSummary => ({ turn_id: turn, complete: true, files });

function file(path: string, operation: 'create' | 'modify' | 'delete' = 'create'): TurnFileSummary['files'][number] {
  return { identity: `workspace:${path}`, relative_path: path, file_name: path.split('/').at(-1)!, path: `${workspace}/${path}`, operation, size_bytes: 22 };
}

describe('canonical Turn file projection', () => {
  test('renders script and command-generated document from the full query without transcript evidence', () => {
    const items = buildTurnFileItems(summary([file('build_docx.py'), file('骆驼祥子读后感.docx')]));
    expect(items.map(item => item.fileName)).toEqual(['build_docx.py', '骆驼祥子读后感.docx']);
    expect(items.every(item => item.sourceMessageIds.length === 0)).toBe(true);
  });

  test('preserves deletion and unavailable historical paths without filtering or inventing receipt proof', () => {
    const items = buildTurnFileItems(summary([file('deleted.docx', 'delete'), file('later-missing.pdf', 'modify')]));
    expect(items.map(item => item.operation)).toEqual(['delete', 'modify']);
    expect(items.every(item => item.tier === 'change')).toBe(true);
    expect(items.every(item => item.artifactId === undefined)).toBe(true);
  });

  test('keeps same-name files from different paths separate using server-owned identity', () => {
    const items = buildTurnFileItems(summary([file('a/report.docx'), file('b/report.docx')]));
    expect(items).toHaveLength(2);
    expect(items[0].identity).not.toBe(items[1].identity);
    expect(items.map(item => item.fileName)).toEqual(['report.docx', 'report.docx']);
  });

  test('one merged row keeps the source filename/locator and separates immutable publication metadata', () => {
    const source = file('original/report.docx', 'modify');
    source.published = { artifact_id: artifactId, path: '/published/canonical.docx', mime_type: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document', size_bytes: 555, sha256: 'b'.repeat(64) };
    const items = buildTurnFileItems(summary([source]));
    expect(items).toHaveLength(1);
    expect(items[0]).toMatchObject({ identity: source.identity, relativePath: source.relative_path,
      fileName: 'report.docx', absolutePath: `${workspace}/original/report.docx`, tier: 'change', sizeBytes: 22,
      publicationId: artifactId });
    expect(items[0].artifactId).toBeUndefined();
    expect(isVerifiedImageDeliverable(items[0])).toBe(false);
  });

  test('MIME and filename alone never grant admission to the verified image gallery', () => {
    const source = file('picture.png');
    source.published = { artifact_id: artifactId, path: '/published/image.png', mime_type: 'image/png', size_bytes: 55, sha256: 'b'.repeat(64) };
    const items = buildTurnFileItems(summary([source]));
    expect(isVerifiedImageDeliverable(items[0])).toBe(false);
  });

  test('the same committed image receipt is rendered once in its existing gallery', () => {
    const verified = images([imageReceipt()]);
    const source = file('picture.png');
    source.sha256 = imageReceipt().sha256;
    source.published = { artifact_id: 'a'.repeat(64), path: '/published/blob', mime_type: 'image/png', size_bytes: 1024, sha256: 'a'.repeat(64) };
    const items = buildTurnFileItems(summary([source]), verified);
    expect(items).toHaveLength(1);
    expect(isVerifiedImageDeliverable(items[0])).toBe(true);
    expect(items[0].sourceMessageIds).toEqual([messageId]);
  });

  test('shared content publication cannot hide another source filename alongside the verified gallery image', () => {
    const verified = images([imageReceipt()]);
    const published = { artifact_id: 'a'.repeat(64), path: '/published/shared-blob', mime_type: 'image/png', size_bytes: 1024, sha256: 'a'.repeat(64) };
    const original = { ...file('picture.png'), sha256: published.sha256, published };
    const renamedCopy = { ...file('another-name.png'), sha256: published.sha256, published };
    const items = buildTurnFileItems(summary([original, renamedCopy]), verified);
    expect(items).toHaveLength(2);
    expect(isVerifiedImageDeliverable(items[0])).toBe(true);
    expect(items[1]).toMatchObject({ identity: renamedCopy.identity, relativePath: 'another-name.png',
      fileName: 'another-name.png', absolutePath: `${workspace}/another-name.png`, publicationId: published.artifact_id, tier: 'change' });
    expect(items[1].artifactId).toBeUndefined();
  });

  test('Unix source paths differing only by trailing filename whitespace remain distinct despite identical bytes', () => {
    const exact = imageReceipt({ path: `${workspace}/picture.png `, relative_path: 'picture.png ' });
    const verified = images([exact]);
    expect(verified[0].absolutePath).toBe(`${workspace}/picture.png `);
    const other = { ...file('picture.png'), sha256: exact.sha256 };
    const items = buildTurnFileItems(summary([other]), verified);
    expect(items).toHaveLength(2);
    expect(items[0].absolutePath).toBe(`${workspace}/picture.png `);
    expect(items[1].absolutePath).toBe(`${workspace}/picture.png`);
    const matching = { ...file('picture.png '), sha256: exact.sha256 };
    expect(buildTurnFileItems(summary([matching]), verified)).toHaveLength(1);
  });

  test('snapshot overlap suppresses only the exact image bytes, retaining later modification or deletion', () => {
    const verified = images([imageReceipt()]);
    const observed = { ...file('picture.png'), sha256: imageReceipt().sha256 };
    expect(buildTurnFileItems(summary([observed]), verified)).toHaveLength(1);
    const changed = buildTurnFileItems(summary([{ ...observed, sha256: 'b'.repeat(64), operation: 'modify' }]), verified);
    expect(changed).toHaveLength(2);
    expect(changed[1].operation).toBe('modify');
    const deleted = buildTurnFileItems(summary([{ ...observed, operation: 'delete' }]), verified);
    expect(deleted).toHaveLength(2);
    expect(deleted[1].operation).toBe('delete');
  });
});

describe('verified image receipt admission', () => {
  test('retains successful receipts in closed failed and canceled Turns but excludes running Turns', () => {
    expect(images([imageReceipt()], 'failed')).toHaveLength(1);
    expect(images([imageReceipt()], 'canceled')).toHaveLength(1);
    expect(images([imageReceipt()], 'running')).toHaveLength(0);
  });

  test('requires completed tool settlement, strict image kind and image MIME', () => {
    expect(images([imageReceipt({ kind: 'file' })])).toHaveLength(0);
    expect(images([imageReceipt({ mime_type: 'text/plain' })])).toHaveLength(0);
    expect(collectVerifiedTurnImages([{ turnId: turn, toolMessages: [tool([imageReceipt()], { status: 'error' })] }], gates()).size).toBe(0);
    expect(collectVerifiedTurnImages([{ turnId: turn, toolMessages: [tool([imageReceipt()])] }], new Map()).size).toBe(0);
  });

  test('deduplicates by committed artifact ID, keeps distinct revisions, and never scans tool arguments', () => {
    const secondId = parsePersistedArtifactId('0190f5fe-7c00-7a00-8000-000000000032');
    const candidates = [{ turnId: turn, toolMessages: [tool([imageReceipt(), imageReceipt(), imageReceipt({ id: secondId })], { name: 'write', description: 'extra.png' })] }];
    const result = collectVerifiedTurnImages(candidates, gates()).get(turn);
    expect(result).toHaveLength(2);
    expect(result?.map(item => item.artifactId)).toEqual([artifactId, secondId]);
  });

  test('keeps Windows proof locator byte-for-byte while rendering workspace-relative path', () => {
    const path = '\\\\?\\C:\\DATA\\session\\image.png';
    const result = collectVerifiedTurnImages([{ turnId: turn, toolMessages: [tool([imageReceipt({ path, relative_path: 'image.png' })])] }], gates(), ['C:/DATA/session']).get(turn)!;
    expect(result[0].absolutePath).toBe(path);
    expect(result[0].relativePath).toBe('image.png');
  });
});
