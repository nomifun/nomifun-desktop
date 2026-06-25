#!/usr/bin/env bash
# 重新生成 vendored Playwright InjectedScript 的预编译 bundle。
#
# 这是**手动**步骤（DESIGN §24 / §6「build 管线」）：产物 `dist/injected.js` 已 check-in
# 进仓，`cargo build` 直接 `include_str!` 它，**绝不**在 cargo build 时依赖 Node/bun。
# 仅在「升级 vendored PW 源」时才需重跑本脚本（换源 → 重 bundle → insta 快照 diff 人审）。
#
# 依赖：bun（自带 esbuild，`bunx esbuild`）。亦可换成 `npx esbuild`（需 Node + esbuild）。
#
# 取源：Playwright Apache-2.0，固定 commit 见 NOTICE。
# vendor 布局（esbuild 别名解析，对齐 PW tsconfig paths）：
#   @isomorphic/* -> vendor/isomorphic/*
#   @injected/*   -> vendor/injected/src/*
#   @protocol/*   -> vendor/protocol/src/*   （channels.d.ts 是 type-only，编译期擦除）
#
# 关键 config（照搬 PW utils/generate_injected.js + 我们的混合架构需要）：
#   --bundle              内联触达的 @isomorphic 子集（不抽子集，整包 vendor，esbuild tree-shake）
#   --format=iife         单 IIFE，注入到 Chromium isolated world 时整体 eval
#   --global-name=...     IIFE 返回值挂到该全局，注入后 `new <global>.InjectedScript(...)`
#   --target=es2019       与 PW 一致
#   --loader:.css=text    highlight.css 作字符串内联（PW inlineCSSPlugin 等价）
#   browserName 在运行时由 Rust 构造 InjectedScript 时固定为 'chromium'（WebKit/FF 分支
#   dead-at-runtime），故无需 esbuild define。
set -euo pipefail
cd "$(dirname "$0")"

GLOBAL_NAME="__nomiInjectedExports"
ENTRY="vendor/injected/src/injectedScript.ts"
OUT="dist/injected.js"

echo "Building $OUT from $ENTRY (global=$GLOBAL_NAME)..."
bunx esbuild "$ENTRY" \
  --bundle \
  --format=iife \
  --global-name="$GLOBAL_NAME" \
  --platform=browser \
  --target=es2019 \
  --alias:@isomorphic=./vendor/isomorphic \
  --alias:@injected=./vendor/injected/src \
  --alias:@protocol=./vendor/protocol/src \
  --loader:.css=text \
  --legal-comments=none \
  --outfile="$OUT.body"

# 在产物头部加 attribution banner（产物头部带署名，DESIGN「许可」要求）。
{
  cat <<'BANNER'
/*
 * NomiFun bundled Playwright InjectedScript.
 *
 * This file is a GENERATED bundle of vendored Playwright sources
 * (packages/injected + packages/isomorphic), Apache-2.0 licensed,
 * Copyright (c) Microsoft Corporation. See injected/NOTICE for the pinned
 * upstream commit and attribution. cssTokenizer is CC0 (public domain).
 *
 * DO NOT EDIT BY HAND. Regenerate via injected/build.sh after updating
 * injected/vendor/. browserName is fixed to 'chromium' at construction time
 * (WebKit/Firefox branches are dead-at-runtime).
 */
BANNER
  cat "$OUT.body"
} > "$OUT"
rm -f "$OUT.body"

echo "Wrote $OUT ($(wc -c < "$OUT") bytes). Exposes ${GLOBAL_NAME}.InjectedScript."
