/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * 「用会话改造」 — the way out of a blocked import (spec D14).
 *
 * A fatal report means the app cannot be served as it stands: `/serve` returns
 * exactly ONE document, so anything that needs a second file, a bundler or a
 * server template has to be rewritten first. Rather than leaving the user at a
 * dead end, the dialog hands the same rule ids the report showed to a fresh Nomi
 * conversation and asks it to produce one self-contained `miniapp.html`.
 *
 * The instruction text lives here, next to the report catalogue, for the same
 * reason {@link MINI_APP_BUILDER_SYSTEM_PROMPT} lives in `contract.ts`: it is a
 * contract with the model, not UI copy the user reads, and it must stay in step
 * with the rule ids the validator emits.
 *
 * How the source reaches the model differs by intake, and that difference is the
 * whole reason this module takes a discriminated union:
 *  - `path` (desktop shell): the backend and the picker share a filesystem, so
 *    the absolute path is named and the model reads it with its file tools —
 *    including the sibling CSS/JS the single-document rule refuses to serve.
 *  - `html` (WebUI browser session): there is no path to hand over, so the
 *    document itself is inlined, clipped at
 *    {@link MINI_APP_IMPORT_INLINE_SOURCE_LIMIT} characters with an explicit
 *    truncation notice — a silent clip would make the model rewrite an app whose
 *    second half it never saw.
 */

import type { IApiMiniAppImportFinding } from '@/common/adapter/ipcBridge';
import { MINI_APP_BUILDER_SYSTEM_PROMPT, MINI_APP_FILE_NAME } from './contract';

/** Characters of an inlined document that ride the first message. */
export const MINI_APP_IMPORT_INLINE_SOURCE_LIMIT = 24_000;

/** Where the candidate came from — the two intakes the dialog supports. */
export type MiniAppImportConversionSource =
  | {
      kind: 'path';
      path: string;
      /**
       * Entry document text, when the renderer could read it.
       *
       * Belt and braces: the path alone is enough IF the session's file tools can
       * reach outside their own workspace, and inlining alone is enough only for a
       * single-file app (a folder's siblings still have to be read from disk).
       * Sending both costs one `fs.readFile` and leaves the model able to start
       * either way.
       */
      document?: string;
    }
  | { kind: 'html'; fileName: string; html: string };

/**
 * `extra.system_prompt` for the conversion thread: the ordinary builder rules
 * plus what makes this session a rewrite rather than a fresh build.
 *
 * It persists with the conversation, so later turns keep the single-file contract
 * without the client replaying anything.
 */
export const MINI_APP_IMPORT_CONVERSION_SYSTEM_PROMPT = `${MINI_APP_BUILDER_SYSTEM_PROMPT}

[导入改造任务]
用户已经写好一个网页应用,但它没通过小程序导入校验。你的任务是把它改写成符合上面规则的单文件小程序,而不是另做一个应用。

**先看这一条:来源目录是只读的。** 用户的原始项目就是他们的真实代码,不在任何 NomiFun 管理的目录里,也没有备份。你只能读它,绝对不能写、改名、移动或删除其中任何文件,也不要在里面执行会产生副作用的命令。你的全部产物只有一个:工作区根目录下的 ${MINI_APP_FILE_NAME}。

在这个前提下:
1. 先读懂原应用的界面与功能,再动手;保持原有交互与视觉,不要顺手"重新设计"。
2. 逐条解决用户消息里列出的校验失败项(每项都给了规则 id 与位置)。
3. 所有本地依赖必须内联进 ${MINI_APP_FILE_NAME}:CSS/JS 直接写进文档,小图片转 data URI,体积大的第三方库改用公网 CDN 地址。
4. 运行环境是 sandbox 且没有 allow-same-origin(来源不透明):cookie、同源请求、localStorage 都可能不可用或抛错,相关代码一律 try/catch 并优雅降级。
5. 最后用一段话说明你改了什么、哪些能力因为沙箱限制被降级。`;

/** Last path segment, tolerant of both separators and of a trailing one. */
export function miniAppImportSourceBaseName(source: MiniAppImportConversionSource): string {
  if (source.kind === 'html') return source.fileName;
  const trimmed = source.path.replace(/[\\/]+$/, '');
  const cut = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'));
  return cut >= 0 ? trimmed.slice(cut + 1) : trimmed;
}

/** `- rule_id [severity] → detail`, the report as the model reads it. */
function formatFindingLines(findings: readonly IApiMiniAppImportFinding[]): string {
  return findings
    .map((finding) => {
      const detail = finding.detail?.trim();
      return `- ${finding.rule_id} [${finding.severity}]${detail ? ` → ${detail}` : ''}`;
    })
    .join('\n');
}

/**
 * The conversation's first message.
 *
 * Names the rule ids verbatim (they are the same strings the report rendered, so
 * the user can match what they were told against what the model was told) and
 * restates the one hard requirement: the product is ONE self-contained document.
 */
export function buildMiniAppImportConversionPrompt(params: {
  source: MiniAppImportConversionSource;
  findings: readonly IApiMiniAppImportFinding[];
}): string {
  const { source, findings } = params;
  const sourceBlock =
    source.kind === 'path'
      ? buildPathSourceBlock(source.path, source.document)
      : buildInlineSourceBlock(source.fileName, source.html);
  const findingBlock =
    findings.length > 0
      ? `导入校验没通过,失败项如下(规则 id / 严重级别 / 位置):\n${formatFindingLines(findings)}`
      : '导入校验没有给出具体失败项,请按单文件自包含的要求整体检查一遍。';

  return [
    `请把我已有的网页应用改造成 NomiFun 小程序:产物必须是工作区根目录下的单个自包含文件 ${MINI_APP_FILE_NAME}。`,
    sourceBlock,
    findingBlock,
    `改造完成后 ${MINI_APP_FILE_NAME} 必须是一个能直接打开就跑的完整文档:不引用任何本地文件、不需要构建步骤、不依赖服务端渲染。我会在预览里点「发布为小程序」把它收进小程序库。`,
  ].join('\n\n');
}

function buildPathSourceBlock(path: string, document?: string): string {
  const head = `来源路径(只读;请用文件工具读取它以及它引用的本地 CSS/JS/资源,不要写入其中任何文件):\n${path}`;
  const text = document?.trim();
  if (!text) return head;
  return `${head}\n\n入口文件当前内容如下,可以直接从它开始改:\n\n${fencedHtml(text)}`;
}

function buildInlineSourceBlock(fileName: string, html: string): string {
  return `来源文件 ${fileName} 的内容如下:\n\n${fencedHtml(html)}`;
}

/** Fenced source, clipped with a notice — a silent clip would mislead the model. */
function fencedHtml(html: string): string {
  const clipped = html.length > MINI_APP_IMPORT_INLINE_SOURCE_LIMIT;
  const body = clipped ? html.slice(0, MINI_APP_IMPORT_INLINE_SOURCE_LIMIT) : html;
  const notice = clipped
    ? `\n(源码过长,以上只是前 ${MINI_APP_IMPORT_INLINE_SOURCE_LIMIT} 个字符;缺失部分请先读原文件或向我确认,不要凭猜测补写。)`
    : '';
  return `\`\`\`html\n${body}\n\`\`\`${notice}`;
}
