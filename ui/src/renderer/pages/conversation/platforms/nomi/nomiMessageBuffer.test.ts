import { describe, expect, test } from 'bun:test';
import {
  MAX_NOMI_MESSAGE_BUFFER_BYTES,
  MAX_NOMI_MESSAGE_BUFFER_ENTRIES,
  MAX_NOMI_MESSAGE_BUFFER_ENTRY_BYTES,
  NomiMessageBufferStore,
  isNomiTextReplacement,
  rememberBoundedNomiCronId,
} from './nomiMessageBuffer';

const inactive = () => false;

describe('NomiMessageBufferStore', () => {
  test('bounds entry count and evicts oldest inactive buffers', () => {
    const store = new NomiMessageBufferStore();
    for (let index = 0; index < MAX_NOMI_MESSAGE_BUFFER_ENTRIES + 4; index += 1) {
      store.append(`message-${index}`, 'x', undefined, inactive);
    }

    expect(store.size).toBe(MAX_NOMI_MESSAGE_BUFFER_ENTRIES);
    expect(store.has('message-0')).toBe(false);
    expect(store.has(`message-${MAX_NOMI_MESSAGE_BUFFER_ENTRIES + 3}`)).toBe(true);
  });

  test('bounds total bytes without evicting an active-turn buffer', () => {
    const store = new NomiMessageBufferStore();
    const activeMessageId = 'active-message';
    const largeChunk = 'a'.repeat(MAX_NOMI_MESSAGE_BUFFER_ENTRY_BYTES);
    const isActive = (messageId: string) => messageId === activeMessageId;

    store.append(activeMessageId, largeChunk, 'active-turn', isActive);
    store.append('old-message', 'b'.repeat(MAX_NOMI_MESSAGE_BUFFER_ENTRY_BYTES), undefined, isActive);
    store.append('new-message', 'c'.repeat(MAX_NOMI_MESSAGE_BUFFER_ENTRY_BYTES), undefined, isActive);

    expect(store.has(activeMessageId)).toBe(true);
    expect(store.byteSize).toBeLessThanOrEqual(MAX_NOMI_MESSAGE_BUFFER_BYTES);
  });

  test('evicts inactive buffers when the byte budget is already full', () => {
    const store = new NomiMessageBufferStore();
    const entryCount =
      MAX_NOMI_MESSAGE_BUFFER_BYTES / MAX_NOMI_MESSAGE_BUFFER_ENTRY_BYTES;
    for (let index = 0; index < entryCount; index += 1) {
      expect(
        store.append(
          `full-${index}`,
          'x'.repeat(MAX_NOMI_MESSAGE_BUFFER_ENTRY_BYTES),
          undefined,
          inactive
        )
      ).toBe(true);
    }
    expect(store.byteSize).toBe(MAX_NOMI_MESSAGE_BUFFER_BYTES);

    expect(store.append('new-entry', 'new', undefined, inactive)).toBe(true);
    expect(store.has('full-0')).toBe(false);
    expect(store.get('new-entry')?.content).toBe('new');
    expect(store.byteSize).toBeLessThanOrEqual(MAX_NOMI_MESSAGE_BUFFER_BYTES);
  });

  test('replacement can evict inactive buffers from a full byte budget', () => {
    const store = new NomiMessageBufferStore();
    const entryCount =
      MAX_NOMI_MESSAGE_BUFFER_BYTES / MAX_NOMI_MESSAGE_BUFFER_ENTRY_BYTES;
    for (let index = 0; index < entryCount; index += 1) {
      store.append(
        `full-${index}`,
        'x'.repeat(MAX_NOMI_MESSAGE_BUFFER_ENTRY_BYTES),
        undefined,
        inactive
      );
    }

    expect(store.replace('replacement', 'final', undefined, inactive)).toBe(true);
    expect(store.has('full-0')).toBe(false);
    expect(store.get('replacement')?.content).toBe('final');
    expect(store.byteSize).toBeLessThanOrEqual(MAX_NOMI_MESSAGE_BUFFER_BYTES);
  });

  test('version-checked cleanup preserves a newer fragment', () => {
    const store = new NomiMessageBufferStore();
    store.append('same-id', 'first', undefined, inactive);
    const taken = store.take('same-id');
    expect(taken).toBeDefined();

    store.append('same-id', 'late', undefined, inactive);
    expect(store.deleteIfVersion('same-id', taken!.version)).toBe(false);
    expect(store.get('same-id')?.content).toBe('late');
  });

  test('replacement overwrites instead of appending and advances the version', () => {
    const store = new NomiMessageBufferStore();
    store.append('same-id', 'draft', undefined, inactive);
    const before = store.get('same-id')!;

    expect(store.replace('same-id', 'final', undefined, inactive)).toBe(true);
    const after = store.get('same-id')!;
    expect(after.content).toBe('final');
    expect(after.version).toBeGreaterThan(before.version);
  });

  test('an empty replacement is retained as a versioned tombstone', () => {
    const store = new NomiMessageBufferStore();
    store.append('same-id', 'draft', undefined, inactive);
    const before = store.get('same-id')!;

    expect(store.replace('same-id', '', undefined, inactive)).toBe(true);
    const after = store.get('same-id')!;
    expect(after.content).toBe('');
    expect(after.version).toBeGreaterThan(before.version);
    expect(store.byteSize).toBe(0);
  });

  test('never splits a non-BMP Unicode scalar at the byte boundary', () => {
    const store = new NomiMessageBufferStore();
    const prefix = 'a'.repeat(MAX_NOMI_MESSAGE_BUFFER_ENTRY_BYTES - 3);

    expect(store.append('unicode', `${prefix}😀`, undefined, inactive)).toBe(true);
    const buffered = store.get('unicode')!;
    expect(buffered.content).toBe(prefix);
    expect(buffered.truncated).toBe(true);
    expect(buffered.content.charCodeAt(buffered.content.length - 1)).not.toBe(0xd83d);
  });

  test('a rejected late fragment invalidates the snapshot and a complete replacement recovers', () => {
    const store = new NomiMessageBufferStore();
    expect(
      store.append(
        'bounded',
        'a'.repeat(MAX_NOMI_MESSAGE_BUFFER_ENTRY_BYTES),
        undefined,
        inactive
      )
    ).toBe(true);
    const complete = store.get('bounded')!;
    expect(complete.truncated).toBe(false);

    expect(store.append('bounded', 'late', undefined, inactive)).toBe(false);
    const truncated = store.get('bounded')!;
    expect(truncated.version).toBeGreaterThan(complete.version);
    expect(truncated.truncated).toBe(true);

    expect(store.replace('bounded', 'final', undefined, inactive)).toBe(true);
    const recovered = store.get('bounded')!;
    expect(recovered.content).toBe('final');
    expect(recovered.truncated).toBe(false);
    expect(recovered.version).toBeGreaterThan(truncated.version);
  });
});

describe('isNomiTextReplacement', () => {
  test('recognizes top-level and rich-data replacement markers', () => {
    expect(isNomiTextReplacement({ replace: true, data: 'ignored' })).toBe(true);
    expect(isNomiTextReplacement({ data: { replace: true, content: 'final' } })).toBe(true);
    expect(isNomiTextReplacement({ data: { replace: false, content: 'chunk' } })).toBe(false);
    expect(isNomiTextReplacement({ data: null })).toBe(false);
  });
});

describe('rememberBoundedNomiCronId', () => {
  test('keeps the processed-id set bounded and refreshes recency', () => {
    const ids = new Set<string>();
    rememberBoundedNomiCronId(ids, 'a', 2);
    rememberBoundedNomiCronId(ids, 'b', 2);
    rememberBoundedNomiCronId(ids, 'a', 2);
    rememberBoundedNomiCronId(ids, 'c', 2);

    expect([...ids]).toEqual(['a', 'c']);
  });
});
