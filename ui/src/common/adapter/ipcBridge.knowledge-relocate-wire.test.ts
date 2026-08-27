/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { afterEach, describe, expect, test } from 'bun:test';
import { parseKnowledgeBaseId, parseKnowledgeEntryId } from '../types/ids';
import { knowledge } from './ipcBridge';

const KNOWLEDGE_BASE_ID = '0190f5fe-7c00-7a00-8000-000000000701';
const REQUEST_ID = '0190f5fe-7c00-7a00-8000-000000000702';
const ENTRY_ID = '0190f5fe-7c00-7a00-8000-000000000703';
const DESTINATION_PARENT_ID = '0190f5fe-7c00-7a00-8000-000000000704';
const realFetch = globalThis.fetch;

afterEach(() => {
  globalThis.fetch = realFetch;
});

describe('knowledge relocate wire contract', () => {
  test('sends stable identity, revision, and exact prior content for editor CAS', async () => {
    let requestBody: unknown;
    globalThis.fetch = (async (_input: string | URL | Request, init?: RequestInit) => {
      requestBody = JSON.parse(String(init?.body));
      return new Response(
        JSON.stringify({
          success: true,
          data: { rel_path: 'archive/topic.md', entry_id: ENTRY_ID },
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } }
      );
    }) as typeof fetch;

    const result = await knowledge.writeFile.invoke({
      knowledge_base_id: parseKnowledgeBaseId(KNOWLEDGE_BASE_ID),
      path: 'drafts/topic.md',
      content: '# Updated',
      expected_content: '# Original',
      entry_id: parseKnowledgeEntryId(ENTRY_ID),
      expected_revision: 8,
    });

    expect(requestBody).toEqual({
      path: 'drafts/topic.md',
      content: '# Updated',
      expected_content: '# Original',
      entry_id: ENTRY_ID,
      expected_revision: 8,
    });
    expect(result).toEqual({ rel_path: 'archive/topic.md', entry_id: ENTRY_ID });
  });

  test('posts one no-clobber path relocation command and returns its prefix receipt', async () => {
    let requestUrl = '';
    let requestBody: unknown;
    globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
      requestUrl = String(input);
      requestBody = JSON.parse(String(init?.body));
      return new Response(
        JSON.stringify({
          success: true,
          data: {
            operation_id: 'move-1',
            entry_id: ENTRY_ID,
            old_path: 'drafts/topic.md',
            new_path: 'archive/topic.md',
            kind: 'file',
            moved_descendant_count: 0,
            revision: 4,
            tree_revision: 9,
            undo_token: 'relocate:move-1',
          },
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } }
      );
    }) as typeof fetch;

    const receipt = await knowledge.relocateTreeEntry.invoke({
      knowledge_base_id: parseKnowledgeBaseId(KNOWLEDGE_BASE_ID),
      source_path: 'drafts/topic.md',
      destination_parent_path: 'archive',
      request_id: REQUEST_ID,
      conflict_policy: 'reject',
      entry_id: parseKnowledgeEntryId(ENTRY_ID),
      destination_parent_id: parseKnowledgeEntryId(DESTINATION_PARENT_ID),
      expected_revision: 3,
    });

    expect(requestUrl.endsWith(`/api/knowledge/bases/${KNOWLEDGE_BASE_ID}/tree/relocate`)).toBe(true);
    expect(requestBody).toEqual({
      source_path: 'drafts/topic.md',
      destination_parent_path: 'archive',
      request_id: REQUEST_ID,
      conflict_policy: 'reject',
      entry_id: ENTRY_ID,
      destination_parent_id: DESTINATION_PARENT_ID,
      expected_revision: 3,
    });
    expect(receipt).toMatchObject({
      entry_id: ENTRY_ID,
      old_path: 'drafts/topic.md',
      new_path: 'archive/topic.md',
      revision: 4,
      tree_revision: 9,
    });
  });

  test('posts an opaque durable undo token without reconstructing paths client-side', async () => {
    let requestUrl = '';
    let requestBody: unknown;
    globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
      requestUrl = String(input);
      requestBody = JSON.parse(String(init?.body));
      return new Response(
        JSON.stringify({
          success: true,
          data: {
            operation_id: 'undo-1',
            entry_id: ENTRY_ID,
            old_path: 'archive/topic.md',
            new_path: 'drafts/topic.md',
            kind: 'file',
            moved_descendant_count: 0,
            revision: 5,
            tree_revision: 10,
          },
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } }
      );
    }) as typeof fetch;

    await knowledge.undoRelocateTreeEntry.invoke({
      knowledge_base_id: parseKnowledgeBaseId(KNOWLEDGE_BASE_ID),
      request_id: REQUEST_ID,
      undo_token: 'relocate:0190f5fe-7c00-7a00-8000-000000000799',
    });

    expect(requestUrl.endsWith(`/api/knowledge/bases/${KNOWLEDGE_BASE_ID}/tree/relocate/undo`)).toBe(true);
    expect(requestBody).toEqual({
      request_id: REQUEST_ID,
      undo_token: 'relocate:0190f5fe-7c00-7a00-8000-000000000799',
    });
  });
});
