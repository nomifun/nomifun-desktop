/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 * Based on AionUi (https://github.com/iOfficeAI/AionUi)
 */

import { describe, expect, test } from 'bun:test';
import { fromApiConversation } from './apiModelMapper';

// 最小 ApiConversation 片段：只构造 mapper 关心的字段
const apiConv = (o: Record<string, unknown>) => ({ id: 'c1', name: 'conv', type: 'acp', created_at: 1, modified_at: 2, ...o });

type MappedExtra = { pinned?: boolean; pinned_at?: number; custom_workspace?: boolean } | null | undefined;
const extraOf = (raw: Record<string, unknown>): MappedExtra => (fromApiConversation(raw) as { extra?: MappedExtra }).extra;

describe('fromApiConversation 置顶镜像（DB 顶层 pinned 列 → extra）', () => {
  test('顶层列置顶 → 镜像进 extra（含服务端维护的 pinned_at）', () => {
    const extra = extraOf(apiConv({ pinned: true, pinned_at: 1712345678000, extra: {} }));
    expect(extra?.pinned).toBe(true);
    expect(extra?.pinned_at).toBe(1712345678000);
  });

  test('extra 为空/缺失时列置顶也能生成镜像', () => {
    const extra = extraOf(apiConv({ pinned: true, pinned_at: 100, extra: null }));
    expect(extra?.pinned).toBe(true);
    expect(extra?.pinned_at).toBe(100);
  });

  test('冲突时列优先：列置顶覆盖 extra.pinned=false，pinned_at 取列值', () => {
    const extra = extraOf(apiConv({ pinned: true, pinned_at: 200, extra: { pinned: false, pinned_at: 999 } }));
    expect(extra?.pinned).toBe(true);
    expect(extra?.pinned_at).toBe(200);
  });

  test('OR 兼容：列未置顶但旧数据仅 extra 置顶 → 不丢，pinned_at 保留 extra 来源', () => {
    const extra = extraOf(apiConv({ pinned: false, extra: { pinned: true, pinned_at: 300 } }));
    expect(extra?.pinned).toBe(true);
    expect(extra?.pinned_at).toBe(300);
  });

  test('两侧均未置顶 → 不注入 pinned key', () => {
    const extra = extraOf(apiConv({ pinned: false, extra: {} }));
    expect(extra && 'pinned' in extra).toBe(false);
  });

  test('列置顶但列 pinned_at 缺失 → 回退 extra.pinned_at', () => {
    const extra = extraOf(apiConv({ pinned: true, extra: { pinned: true, pinned_at: 400 } }));
    expect(extra?.pinned).toBe(true);
    expect(extra?.pinned_at).toBe(400);
  });

  test('custom_workspace 推导不受置顶镜像影响', () => {
    const extra = extraOf(apiConv({ pinned: true, pinned_at: 1, extra: { workspace: '/w/p1' } }));
    expect(extra?.custom_workspace).toBe(true);
    expect(extra?.pinned).toBe(true);
  });
});
