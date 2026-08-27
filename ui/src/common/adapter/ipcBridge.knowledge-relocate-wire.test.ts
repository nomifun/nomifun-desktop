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
const SOURCE_ID = '0190f5fe-7c00-7a00-8000-000000000705';
const SOURCE_ITEM_ID = '0190f5fe-7c00-7a00-8000-000000000706';
const realFetch = globalThis.fetch;

afterEach(() => {
  globalThis.fetch = realFetch;
});

describe('knowledge relocate wire contract', () => {
  test('preserves stable entry policy metadata on file listings', async () => {
    globalThis.fetch = (async () =>
      new Response(
        JSON.stringify({
          success: true,
          data: [
            {
              entry_id: ENTRY_ID,
              revision: 7,
              origin: 'url_snapshot',
              rel_path: 'archive/captured.md',
              size: 120,
              modified_at: null,
              capabilities: {
                read_content: true,
                edit_content: false,
                rename: true,
                relocate: true,
                accept_children: false,
                delete_entry: false,
                remove_source: true,
                refresh_source: true,
                detach_source: true,
                copy_as_editable: true,
                export_entry: true,
                edit_metadata: true,
                read_only_reason: 'Managed by its web source.',
              },
              source: {
                source_id: SOURCE_ID,
                source_item_id: SOURCE_ITEM_ID,
                source_url: 'https://example.com',
                relationship: 'managed',
                sync_status: 'synced',
              },
            },
          ],
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } }
      )) as typeof fetch;

    const files = await knowledge.listFiles.invoke({
      knowledge_base_id: parseKnowledgeBaseId(KNOWLEDGE_BASE_ID),
    });

    expect(files[0].entry_id).toBe(ENTRY_ID);
    expect(files[0].capabilities?.read_only_reason).toBe('Managed by its web source.');
    expect(files[0].source?.source_item_id).toBe(SOURCE_ITEM_ID);
  });

  test('sends the selected destination for web capture content', async () => {
    let requestBody: unknown;
    globalThis.fetch = (async (_input: string | URL | Request, init?: RequestInit) => {
      requestBody = JSON.parse(String(init?.body));
      return new Response(
        JSON.stringify({
          success: true,
          data: {
            type: 'web',
            added: 1,
            duplicates: 0,
            fetched: 1,
            failed: 0,
            errors: [],
          },
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } }
      );
    }) as typeof fetch;

    await knowledge.addContent.invoke({
      knowledge_base_id: parseKnowledgeBaseId(KNOWLEDGE_BASE_ID),
      type: 'web',
      entries: [{ url: 'https://example.com/docs' }],
      destination_parent_path: 'research/web',
      destination_parent_id: parseKnowledgeEntryId(DESTINATION_PARENT_ID),
    });

    expect(requestBody).toEqual({
      type: 'web',
      entries: [{ url: 'https://example.com/docs' }],
      destination_parent_path: 'research/web',
      destination_parent_id: DESTINATION_PARENT_ID,
    });
  });

  test('preserves authoritative capabilities on content reads', async () => {
    globalThis.fetch = (async () =>
      new Response(
        JSON.stringify({
          success: true,
          data: {
            entry_id: ENTRY_ID,
            revision: 4,
            rel_path: 'archive/captured.md',
            content: '# Captured',
            size: 10,
            modified_at: null,
            capabilities: {
              read_content: true,
              edit_content: false,
              rename: true,
              relocate: true,
              accept_children: false,
              delete_entry: false,
              remove_source: true,
              refresh_source: true,
              detach_source: true,
              copy_as_editable: true,
              export_entry: true,
              edit_metadata: true,
              read_only_reason: 'Managed by its web source.',
            },
            source: {
              source_id: SOURCE_ID,
              source_item_id: SOURCE_ITEM_ID,
              source_url: 'https://example.com',
              relationship: 'managed',
              sync_status: 'synced',
            },
          },
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } }
      )) as typeof fetch;

    const file = await knowledge.readFile.invoke({
      knowledge_base_id: parseKnowledgeBaseId(KNOWLEDGE_BASE_ID),
      path: 'archive/captured.md',
    });

    expect(file.entry_id).toBe(ENTRY_ID);
    expect(file.capabilities?.edit_content).toBe(false);
    expect(file.capabilities?.relocate).toBe(true);
    expect(file.source?.relationship).toBe('managed');
  });

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

  test('routes entry source actions by stable identity and preserves action metadata', async () => {
    const calls: Array<{ url: string; body: unknown }> = [];
    globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
      calls.push({
        url: String(input),
        body: init?.body ? JSON.parse(String(init.body)) : undefined,
      });
      return new Response(
        JSON.stringify({
          success: true,
          data: {
            entry: {
              entry_id: ENTRY_ID,
              revision: 12,
              name: 'captured.md',
              rel_path: 'archive/captured.md',
              is_dir: false,
              is_file: true,
              modified_at: null,
              origin: 'url_snapshot',
              capabilities: {
                read_content: true,
                edit_content: false,
                rename: true,
                relocate: true,
                accept_children: false,
                delete_entry: false,
                remove_source: true,
                refresh_source: true,
                detach_source: true,
                copy_as_editable: true,
                export_entry: true,
                edit_metadata: true,
              },
              source: {
                source_id: SOURCE_ID,
                source_item_id: SOURCE_ITEM_ID,
                source_url: 'https://example.com',
                relationship: 'managed',
                sync_status: 'synced',
              },
            },
          },
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } }
      );
    }) as typeof fetch;

    const common = {
      knowledge_base_id: parseKnowledgeBaseId(KNOWLEDGE_BASE_ID),
      entry_id: parseKnowledgeEntryId(ENTRY_ID),
      expected_revision: 11,
    };
    const refreshed = await knowledge.refreshEntrySource.invoke(common);
    await knowledge.detachEntrySource.invoke(common);
    await knowledge.removeEntrySource.invoke(common);
    await knowledge.copyEntryAsEditable.invoke({
      ...common,
      destination_parent_path: 'archive',
      destination_parent_id: parseKnowledgeEntryId(DESTINATION_PARENT_ID),
      new_name: 'captured-copy.md',
    });

    expect(calls.map((call) => call.url.split('/').at(-1))).toEqual([
      'refresh-source',
      'detach-source',
      'remove-source',
      'copy-as-editable',
    ]);
    expect(calls[0].body).toEqual({ expected_revision: 11 });
    expect(calls[3].body).toEqual({
      expected_revision: 11,
      destination_parent_path: 'archive',
      destination_parent_id: DESTINATION_PARENT_ID,
      new_name: 'captured-copy.md',
    });
    expect(refreshed.entry?.entry_id).toBe(ENTRY_ID);
    expect(refreshed.entry?.capabilities?.edit_content).toBe(false);
    expect(refreshed.entry?.source?.relationship).toBe('managed');
  });

  test('deletes files and folders with stable identity CAS when available', async () => {
    const urls: string[] = [];
    globalThis.fetch = (async (input: string | URL | Request) => {
      urls.push(String(input));
      return new Response(JSON.stringify({ success: true, data: null }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }) as typeof fetch;

    const common = {
      knowledge_base_id: parseKnowledgeBaseId(KNOWLEDGE_BASE_ID),
      path: 'archive/topic.md',
      entry_id: parseKnowledgeEntryId(ENTRY_ID),
      expected_revision: 12,
    };
    await knowledge.deleteFile.invoke(common);
    await knowledge.deleteFolder.invoke({ ...common, path: 'archive' });

    for (const url of urls) {
      expect(url.includes(`entry_id=${ENTRY_ID}`)).toBe(true);
      expect(url.includes('expected_revision=12')).toBe(true);
    }
  });
});
