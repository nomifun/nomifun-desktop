/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { I18nKey } from '@/renderer/services/i18n/i18n-keys';

/**
 * P3-N1 桌宠气泡浏览器动作 narration。
 *
 * 后端自研 browser-use 引擎的 `Browser` 工具经标准 `tool_call` 通道上桌宠气泡，
 * 但流式事件里 `description` 恒为 `None`（见 backend_output_sink），气泡过去只能压
 * 通用占位 `usingTools`，把具体动作（navigate/click/type…）和参数（url/ref/text）全
 * 丢弃。本模块复用裁决 ⑧——参照后端 `BrowserTool::describe`（tool.rs:1160）的人读
 * narration，在前端按 `args.action` 合成一条具体文案的 i18n key + 参数，替代通用占位。
 *
 * 只特化 `Browser` 工具；其它工具返回 `null`，调用方维持原 `usingTools` 占位与
 * 消散/stall 安全网行为不变。
 */

/** 浏览器工具线名（Rust `BrowserTool::name()` 返回 "Browser"）。 */
const BROWSER_TOOL_NAME = 'Browser';

export interface BrowserNarration {
  key: I18nKey;
  /** i18n 插值参数（除 `name` 由调用方补桌宠名外的动作专属参数）。 */
  params: Record<string, string>;
}

/** 取 URL 的可读 host（"https://a.example.com/x" → "a.example.com"）；解析失败回退原串截断。 */
const friendlyHost = (raw: string): string => {
  const url = raw.trim();
  if (!url) return '';
  try {
    return new URL(url).hostname || truncate(url, 40);
  } catch {
    // 没有协议头时补一个再试（"example.com/x" → host）。
    try {
      return new URL(`https://${url}`).hostname || truncate(url, 40);
    } catch {
      return truncate(url, 40);
    }
  }
};

const truncate = (s: string, max: number): string =>
  s.length > max ? `${s.slice(0, max)}…` : s;

/** 从三种工具事件数据形状里抽出 `{ name, args }`（args 含 action/url/ref/text…）。 */
const extractNameAndArgs = (
  data: unknown
): { name: string; args: Record<string, unknown> } | null => {
  if (!data || typeof data !== 'object') return null;

  // tool_call（nomi 引擎）：{ name, args, ... }
  const asToolCall = data as { name?: unknown; args?: unknown };
  if (typeof asToolCall.name === 'string' && asToolCall.args && typeof asToolCall.args === 'object') {
    return { name: asToolCall.name, args: asToolCall.args as Record<string, unknown> };
  }

  // acp_tool_call：{ update: { title, rawInput } }（兼容 snake_case raw_input）
  const asAcp = data as {
    update?: { title?: unknown; rawInput?: unknown; raw_input?: unknown };
  };
  if (asAcp.update && typeof asAcp.update === 'object') {
    const title = asAcp.update.title;
    const raw = asAcp.update.rawInput ?? asAcp.update.raw_input;
    if (typeof title === 'string' && raw && typeof raw === 'object') {
      return { name: title, args: raw as Record<string, unknown> };
    }
  }

  // tool_group：取首个 Browser 项（数组）。
  if (Array.isArray(data)) {
    const browserEntry = data.find(
      (e): e is { name?: unknown } => !!e && typeof e === 'object' && (e as { name?: unknown }).name === BROWSER_TOOL_NAME
    );
    if (browserEntry) {
      // tool_group 项不带结构化 args（只 description），无 action 细节 → 走通用 browser 占位。
      return { name: BROWSER_TOOL_NAME, args: {} };
    }
  }

  return null;
};

const str = (args: Record<string, unknown>, key: string): string =>
  typeof args[key] === 'string' ? (args[key] as string) : '';

/**
 * 解析浏览器动作 → 气泡 narration（i18n key + 参数）。
 *
 * @param data 流式事件的 `message.data`（ToolCallEventData / AcpToolCallEventData / ToolGroup 数组）
 * @returns Browser 工具 → 具体 narration；非 Browser 或无法识别 → `null`（调用方维持通用占位）
 */
export const browserNarrationFor = (data: unknown): BrowserNarration | null => {
  const extracted = extractNameAndArgs(data);
  if (!extracted || extracted.name !== BROWSER_TOOL_NAME) return null;

  const { args } = extracted;
  const action = str(args, 'action');

  switch (action) {
    case 'navigate':
    case 'open_link_new_tab': {
      const host = friendlyHost(str(args, 'url'));
      return host
        ? { key: 'nomi.companion.browser.navigate', params: { host } }
        : { key: 'nomi.companion.browser.busy', params: {} };
    }
    case 'click':
      return { key: 'nomi.companion.browser.click', params: {} };
    case 'type':
    case 'set_value':
      return { key: 'nomi.companion.browser.type', params: {} };
    case 'observe':
      return { key: 'nomi.companion.browser.observe', params: {} };
    case 'screenshot':
      return { key: 'nomi.companion.browser.screenshot', params: {} };
    case 'extract':
    case 'get_page_text':
      return { key: 'nomi.companion.browser.read', params: {} };
    case 'search_page':
    case 'scroll_to_text':
    case 'find_elements': {
      const q = truncate(str(args, 'query') || str(args, 'text') || str(args, 'selector'), 30);
      return q
        ? { key: 'nomi.companion.browser.search', params: { query: q } }
        : { key: 'nomi.companion.browser.busy', params: {} };
    }
    case 'download':
    case 'save_as_pdf':
      return { key: 'nomi.companion.browser.download', params: {} };
    case 'scroll':
    case 'press_key':
    case 'hover':
    case 'select_option':
    case 'wait':
    case 'wait_for':
    case 'back':
    case 'forward':
    case 'reload':
    case 'tabs':
    case 'switch_tab':
    case 'close_tab':
    case 'switch_frame':
    case 'cursor':
    case 'get_dropdown_options':
    case 'upload_file':
    case 'capabilities':
    case 'evaluate':
    default:
      // 其余动作（含未知/缺 action）→ 一句通用「正在浏览网页…」，仍比 usingTools 具体。
      return { key: 'nomi.companion.browser.busy', params: {} };
  }
};
