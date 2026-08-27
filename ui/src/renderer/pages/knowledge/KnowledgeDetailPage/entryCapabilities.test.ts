import { describe, expect, test } from 'bun:test';

import type { IKnowledgeTreeEntry } from '@/common/adapter/ipcBridge';
import { parseKnowledgeSourceId, parseKnowledgeSourceItemId } from '@/common/types/ids';
import {
  hasKnowledgeEntryCapability,
  isManagedKnowledgeEntry,
  knowledgeEntryRestrictionReason,
} from './entryCapabilities';

const managedSnapshot: IKnowledgeTreeEntry = {
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
    read_only_reason: 'Managed body',
  },
  source: {
    source_id: parseKnowledgeSourceId('0190f5fe-7c00-7a00-8000-000000000705'),
    source_item_id: parseKnowledgeSourceItemId('0190f5fe-7c00-7a00-8000-000000000706'),
    source_url: 'https://example.com',
    relationship: 'managed',
    sync_status: 'synced',
  },
};

describe('knowledge entry capabilities', () => {
  test('fails closed when capabilities are absent', () => {
    expect(hasKnowledgeEntryCapability(undefined, 'edit_content')).toBe(false);
    expect(
      hasKnowledgeEntryCapability(
        { ...managedSnapshot, capabilities: undefined },
        'relocate'
      )
    ).toBe(false);
  });

  test('keeps managed content read-only while allowing structural management', () => {
    expect(isManagedKnowledgeEntry(managedSnapshot)).toBe(true);
    expect(hasKnowledgeEntryCapability(managedSnapshot, 'edit_content')).toBe(false);
    expect(hasKnowledgeEntryCapability(managedSnapshot, 'relocate')).toBe(true);
    expect(hasKnowledgeEntryCapability(managedSnapshot, 'rename')).toBe(true);
    expect(hasKnowledgeEntryCapability(managedSnapshot, 'copy_as_editable')).toBe(true);
    expect(knowledgeEntryRestrictionReason(managedSnapshot, 'fallback')).toBe('fallback');
  });

  test('does not infer managed state from origin alone', () => {
    expect(isManagedKnowledgeEntry({ ...managedSnapshot, source: undefined })).toBe(false);
  });
});
