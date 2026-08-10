/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Mini-app contract shared by the start page (creation), the conversation
 * preview panel (live rendering + solidify), and the /mini-apps library pages.
 *
 * A mini-app conversation is a normal Nomi conversation whose `extra` carries
 * `miniapp: true` plus a builder system prompt. The one and only artifact is a
 * single self-contained HTML document at {@link MINI_APP_FILE_NAME} in the
 * conversation workspace root. Design spec: docs/specs/2026-08-09-miniapps.zh.md
 */

import { getBaseUrl } from '@/common/adapter/httpBridge';
import type { MiniAppId } from '@/common/types/ids';

/** Workspace-root file the builder prompt pins the agent to. */
export const MINI_APP_FILE_NAME = 'miniapp.html';

/**
 * `conversations.extra` marker key set at create time by the start page.
 *
 * One marker is enough: the artifact path is a constant of this contract
 * ({@link MINI_APP_FILE_NAME}), so writing it into `extra` too would only create
 * a second source of truth that nothing ever reads.
 */
export const MINI_APP_EXTRA_FLAG = 'miniapp';

/**
 * Injected as `extra.system_prompt` (the Nomi engine's `custom` prompt
 * section). It persists with the conversation, so reopening the session keeps
 * the builder behavior without any client-side replay.
 */
export const MINI_APP_BUILDER_SYSTEM_PROMPT = `[NomiFun 小程序构建模式]
你正在为用户构建一个「小程序」— 独立、自包含的网页小工具。规则:
1. 产物永远是工作区根目录下的单个文件 ${MINI_APP_FILE_NAME},用文件工具创建与修改它。
2. ${MINI_APP_FILE_NAME} 必须完全自包含:内联全部 CSS 与 JavaScript;需要第三方库时可经 CDN 引入;不得依赖工作区内其他文件。
3. 界面追求现代、美观、可即时上手;无需任何构建步骤。
4. 需要持久化数据时优先使用 localStorage,键名加应用专属前缀;但沙箱可能禁用存储,所有存储读写必须包在 try/catch 里并在失败时优雅降级,核心功能不得依赖持久化。
5. 每一轮回复结束时 ${MINI_APP_FILE_NAME} 都必须是完整可运行的版本:首轮直接给出可用版本,之后按用户反馈迭代。
6. 除非用户明确要求,不创建其他文件;回复里简述改动即可,不要粘贴大段代码。`;

/** Max characters of the user's request quoted into the conversation name. */
export const MINI_APP_NAME_SNIPPET_LENGTH = 16;

/**
 * True when a conversation was launched through the start page's
 * "create mini-app" capability. Tolerant of the loose `extra` bag shape.
 */
export function isMiniAppConversation(extra: unknown): boolean {
  if (extra == null || typeof extra !== 'object') return false;
  return (extra as Record<string, unknown>)[MINI_APP_EXTRA_FLAG] === true;
}

/**
 * Absolute URL the runner/preview iframes load a solidified mini-app from.
 * The backend route is intentionally unauthenticated (iframe subresource
 * loads carry no trust header) and embed-whitelisted.
 */
export function resolveMiniAppServeUrl(miniappId: MiniAppId): string {
  return `${getBaseUrl()}/api/miniapps/${encodeURIComponent(miniappId)}/serve`;
}

/**
 * Sandbox grants for mini-app iframes. Shared by the preview and the runner so
 * the two surfaces can never drift apart.
 *
 * This deliberately DIVERGES from {@link HTMLRenderer}'s legacy policy: that one
 * keeps `allow-same-origin`, which — combined with `allow-scripts` — voids the
 * sandbox entirely. A mini-app document is AI-generated code and must run with an
 * opaque origin: with `srcDoc` (preview) or a same-origin serve URL (runner),
 * granting both flags would hand the generated script the host origin, its
 * cookies, and its `localStorage`. The trade-off is that storage APIs may throw
 * inside the frame, which the builder prompt requires every app to tolerate.
 */
export const MINI_APP_IFRAME_SANDBOX = 'allow-scripts allow-forms allow-popups allow-modals';
