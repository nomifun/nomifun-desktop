/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * 桌面伙伴窗口共享全局 WS 广播；learn 循环产出的气泡/心情/学习事件由单个伙伴
 * （后端 scope 到默认体）发声。每个窗口只呈现属于自己的事件。
 * - `companion_id` 缺省（undefined/null）放行：兼容灰度期旧后端、以及将来真正的全局事件。
 * - 空串 `''` **不**放行：它永不等于真实 id，故 degenerate 空目标会抑制气泡而非复活风暴。
 */
export const isForCompanion = (
  evt: { companion_id?: string | null },
  companionId: string | null
): boolean => evt.companion_id == null || evt.companion_id === companionId;
