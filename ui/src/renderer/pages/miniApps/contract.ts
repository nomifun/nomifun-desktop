/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Mini-app contract shared by the `/mini-apps` pages and the conversation preview
 * panel.
 *
 * Every mini-app conversation is an ORDINARY conversation in an ordinary
 * workspace (spec D16), so everything the model needs to know rides client-side
 * text from this module. There are two such texts and they are not
 * interchangeable:
 *
 *  - **Creating** (start page 创建小程序, and the import dialog's 「用会话改造」 exit)
 *    injects {@link MINI_APP_BUILDER_SYSTEM_PROMPT} as `extra.system_prompt` plus
 *    the {@link MINI_APP_EXTRA_FLAG} marker. The artifact is
 *    {@link MINI_APP_FILE_NAME} in the conversation's own workspace root, and the
 *    preview panel's 「发布为新的小程序」 is how it reaches the library.
 *  - **Iterating** (「继续迭代」 on a library card or the runner toolbar) provisions
 *    the app's working copy first and writes its ABSOLUTE path into the first
 *    message — {@link buildMiniAppIterateMessage}. That conversation gets no
 *    marker and no builder prompt: its artifact lives outside its workspace, so
 *    both would describe a file that is not there.
 *
 * Design specs: docs/specs/2026-08-09-miniapps.zh.md (v1),
 * docs/specs/2026-08-10-miniapps-v3-unified-conversations.zh.md (v3)
 */

import type { TFunction } from 'i18next';
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

/** Everything 「继续迭代」 has to tell the model about the app it is about to change. */
export interface MiniAppIterateTarget {
  name: string;
  miniAppId: MiniAppId;
  /** Absolute `source_path` straight from `POST /api/miniapps/{id}/workspace`. */
  sourcePath: string;
}

/**
 * First message of an iteration conversation (spec D19).
 *
 * The conversation is ordinary and its workspace has nothing to do with the app,
 * so this message is the ONLY thing that locates the source — hence the absolute
 * path, and hence "read the whole file first" (the model would otherwise rewrite
 * a document it never saw). It is user-visible content, not a system prompt, so
 * it goes through i18n.
 *
 * Composed here rather than at the two call sites (library card, runner toolbar)
 * so the two can never drift into telling the model different things.
 */
export const buildMiniAppIterateMessage = (target: MiniAppIterateTarget, t: TFunction): string =>
  t('miniApps.iterate.firstMessage', {
    name: target.name,
    id: target.miniAppId,
    path: target.sourcePath,
  });

/** Title of an iteration conversation, so the session list says which app it is. */
export const buildMiniAppIterateConversationName = (name: string, t: TFunction): string =>
  t('miniApps.iterate.conversationName', { name });

/**
 * True when a conversation was launched through the start page's
 * "create mini-app" capability. Tolerant of the loose `extra` bag shape.
 */
export function isMiniAppConversation(extra: unknown): boolean {
  if (extra == null || typeof extra !== 'object') return false;
  return (extra as Record<string, unknown>)[MINI_APP_EXTRA_FLAG] === true;
}

/**
 * Absolute URL the runner/preview iframes load a published mini-app from.
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
