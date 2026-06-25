/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, it } from 'vitest';
import { isForCompanion } from './eventScope';

describe('isForCompanion', () => {
  it('命中本伙伴 id', () => expect(isForCompanion({ companion_id: 'c1' }, 'c1')).toBe(true));
  it('拒绝其他伙伴 id', () => expect(isForCompanion({ companion_id: 'c2' }, 'c1')).toBe(false));
  it('缺省(undefined/null)放行——兼容旧后端/全局事件', () => {
    expect(isForCompanion({}, 'c1')).toBe(true);
    expect(isForCompanion({ companion_id: null }, 'c1')).toBe(true);
  });
  it('空串目标抑制(不复活风暴)', () => expect(isForCompanion({ companion_id: '' }, 'c1')).toBe(false));
});
