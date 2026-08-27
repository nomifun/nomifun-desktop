import { describe, expect, test } from 'bun:test';
import { parseKnowledgeUrlDrafts } from './knowledgeUrlEntries';

describe('knowledge URL entry parsing', () => {
  test('trims values and applies browser rendering consistently', () => {
    expect(
      parseKnowledgeUrlDrafts(
        [{ url: '  https://example.com/docs  ', title: '  Product docs  ' }],
        true,
      ),
    ).toEqual({
      ok: true,
      entries: [{ url: 'https://example.com/docs', title: 'Product docs', rendered: true }],
    });
  });

  test('rejects non-http URLs and portable URL duplicates before submission', () => {
    expect(parseKnowledgeUrlDrafts([{ url: 'file:///tmp/notes', title: '' }], false)).toEqual({
      ok: false,
      reason: 'invalid',
      url: 'file:///tmp/notes',
    });
    expect(
      parseKnowledgeUrlDrafts(
        [
          { url: 'https://example.com', title: '' },
          { url: 'https://example.com/', title: 'Duplicate' },
        ],
        false,
      ),
    ).toEqual({
      ok: false,
      reason: 'duplicate',
      url: 'https://example.com/',
    });
  });

  test('enforces the caller-provided remaining capacity', () => {
    expect(
      parseKnowledgeUrlDrafts(
        [
          { url: 'https://example.com/a', title: '' },
          { url: 'https://example.com/b', title: '' },
        ],
        false,
        1,
      ),
    ).toEqual({ ok: false, reason: 'limit', limit: 1 });
  });
});
