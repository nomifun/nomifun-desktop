import { describe, expect, test } from 'bun:test';
import type { IKnowledgeBinding } from '@/common/adapter/ipcBridge';
import { removeBaseFromBinding } from './KnowledgeConsumersSection';

const binding = (overrides: Partial<IKnowledgeBinding> = {}): IKnowledgeBinding => ({
  enabled: true,
  writeback: true,
  writeback_mode: 'direct',
  writeback_eagerness: 'aggressive',
  channel_write_enabled: true,
  kb_ids: ['kb_a', 'kb_b'],
  ...overrides,
});

describe('knowledge consumer unmount binding transform', () => {
  test('removes only the requested base and preserves binding policy fields', () => {
    expect(removeBaseFromBinding(binding(), 'kb_a')).toEqual({
      enabled: true,
      writeback: true,
      writeback_mode: 'direct',
      writeback_eagerness: 'aggressive',
      channel_write_enabled: true,
      kb_ids: ['kb_b'],
    });
  });

  test('turns the binding off when the last mounted base is removed', () => {
    expect(removeBaseFromBinding(binding({ kb_ids: ['kb_a'] }), 'kb_a')).toEqual({
      enabled: false,
      writeback: true,
      writeback_mode: 'direct',
      writeback_eagerness: 'aggressive',
      channel_write_enabled: true,
      kb_ids: [],
    });
  });

  test('keeps a non-empty binding enabled when the requested base is not present', () => {
    expect(removeBaseFromBinding(binding(), 'kb_missing')).toEqual(binding());
  });
});
