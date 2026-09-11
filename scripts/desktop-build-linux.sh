#!/usr/bin/env bash
# ============================================================================
# 打 Linux 桌面端安装包(.deb / .AppImage / .rpm),汇总到 dist/desktop/。
# 每个真实 package 同时生成并验证 Host/package/legal release lock。
# 仅能在 Linux 上运行。
#
#   bun run build:linux               # 默认打当前机器架构(x64 或 arm64)
#   bun run build:linux x64           # 显式 x86_64
#   bun run build:linux arm64         # 显式 aarch64(见下方交叉编译警告)
#   bun run build:linux x64 arm64     # 两个都打
#   bun run build:linux --config apps/desktop/tauri.updater.conf.json
#                                     # 未知 --xxx 选项会原样透传给 tauri build
#   bun run build:linux -- --bundles deb
#                                     # `--` 之后的参数也会原样透传给 tauri build
#
# 架构别名:
#   x64   / x86_64        -> x86_64-unknown-linux-gnu
#   arm64 / aarch64 / arm -> aarch64-unknown-linux-gnu
#
# Linux 没有 macOS 那种「签名 + 公证」体系,故本脚本不涉及签名。
#
# ⚠️ 交叉编译警告:Tauri 的 Linux 包链接 webkit2gtk 等系统库,跨架构构建
#    (在 x64 上打 arm64,或反之)需要目标架构的 sysroot/交叉工具链,仅
#    `rustup target add` 不够。最稳妥是在「目标架构的机器或容器」上原生构建。
#    本脚本只对当前机器架构自动装 rust target;其它架构仅尝试,失败请改用原生环境。
#
# 注:macOS 包用 build:mac,Windows 包用 build:win,且都需在对应系统上构建。
# ============================================================================
set -euo pipefail

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "❌ build:linux 只能在 Linux 上运行(当前: $(uname -s))。" >&2
  echo "   macOS 包用 build:mac,Windows 包用 build:win,且都需在对应系统上构建。" >&2
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT"
CONF="apps/desktop/tauri.conf.json"
DIST="$ROOT/dist/desktop"
RELEASE_LOCK_TOOL="$ROOT/scripts/release/release-lock.mjs"

for tool in bun git rustup node; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "❌ Linux packaging requires '$tool'." >&2
    exit 1
  }
done
[[ -f "$RELEASE_LOCK_TOOL" ]] || {
  echo "❌ missing release-lock tool: $RELEASE_LOCK_TOOL" >&2
  exit 1
}

require_linux_build_deps() {
  local missing=()

  if ! command -v pkg-config >/dev/null 2>&1; then
    missing+=("pkg-config")
  else
    pkg-config --exists gtk+-3.0 || missing+=("libgtk-3-dev (pkg-config: gtk+-3.0)")
    pkg-config --exists webkit2gtk-4.1 || missing+=("libwebkit2gtk-4.1-dev (pkg-config: webkit2gtk-4.1)")
    pkg-config --exists gbm || missing+=("libgbm-dev (pkg-config: gbm)")
    pkg-config --exists librsvg-2.0 || missing+=("librsvg2-dev (pkg-config: librsvg-2.0)")
    if ! pkg-config --exists ayatana-appindicator3-0.1 && ! pkg-config --exists appindicator3-0.1; then
      missing+=("libayatana-appindicator3-dev 或 libappindicator3-dev (pkg-config: *appindicator3-0.1)")
    fi
  fi

  if [[ "${#missing[@]}" -gt 0 ]]; then
    echo "❌ Linux 打包依赖不完整:" >&2
    local item
    for item in "${missing[@]}"; do
      echo "   - $item" >&2
    done
    cat >&2 <<'EOF'

Debian/Ubuntu 可先安装:
  sudo apt-get install -y build-essential pkg-config libgtk-3-dev libwebkit2gtk-4.1-dev libgbm-dev libayatana-appindicator3-dev librsvg2-dev patchelf

说明:
  - libgbm-dev 提供 -lgbm 链接名与 gbm.pc。
  - libayatana-appindicator3-dev 提供 Tauri 托盘/AppIndicator 打包探测。
  - librsvg2-dev 提供 linuxdeploy GTK 插件需要的 librsvg-2.0.pc。
  - 本脚本会设置 APPIMAGE_EXTRACT_AND_RUN=1，让 linuxdeploy AppImage 在无 FUSE2 的构建机上也能运行。
EOF
    exit 1
  fi
}

# 当前机器架构对应的 triple(用于判断哪个是「原生」)
HOST_ARCH="$(uname -m)"
case "$HOST_ARCH" in
  x86_64)         HOST_TRIPLE="x86_64-unknown-linux-gnu" ;;
  aarch64|arm64)  HOST_TRIPLE="aarch64-unknown-linux-gnu" ;;
  *)              HOST_TRIPLE="" ;;
esac

# ── 解析参数:`--` 之前是架构选择,之后原样透传给 tauri build ──────────────────
SELECT=()
PASSTHRU=()
seen_dashdash=0
for arg in "$@"; do
  if [[ "$seen_dashdash" -eq 1 ]]; then
    PASSTHRU+=("$arg")
  elif [[ "$arg" == "--" ]]; then
    seen_dashdash=1
  elif [[ "$arg" == --* ]]; then
    PASSTHRU+=("$arg")
    seen_dashdash=1
  else
    SELECT+=("$arg")
  fi
done

resolve_triple() {
  case "$1" in
    x64|x86_64|x86_64-unknown-linux-gnu)              echo "x86_64-unknown-linux-gnu" ;;
    arm64|aarch64|arm|aarch64-unknown-linux-gnu)      echo "aarch64-unknown-linux-gnu" ;;
    *) echo "❌ 未知架构: $1 (可选: x64 / arm64)" >&2; exit 1 ;;
  esac
}

TRIPLES=()
if [[ "${#SELECT[@]}" -eq 0 ]]; then
  if [[ -z "$HOST_TRIPLE" ]]; then
    echo "❌ 无法识别当前架构: $HOST_ARCH,请显式指定 x64 或 arm64。" >&2
    exit 1
  fi
  TRIPLES=("$HOST_TRIPLE")   # 默认只打当前机器架构
else
  for s in "${SELECT[@]}"; do
    TRIPLES+=("$(resolve_triple "$s")")
  done
fi

require_linux_build_deps
export APPIMAGE_EXTRACT_AND_RUN="${APPIMAGE_EXTRACT_AND_RUN:-1}"

ensure_target() {
  local t="$1"
  if ! rustup target list --installed | grep -qx "$t"; then
    echo "▶ 安装 Rust target: $t"
    rustup target add "$t"
  fi
}

validate_release_host() {
  local host="$1"
  [[ -f "$host" && ! -L "$host" && -x "$host" ]] || {
    echo "❌ expected a regular executable Linux Host: $host" >&2
    exit 1
  }
}

write_release_lock() {
  local target="$1"
  local host="$2"
  local package="$3"
  local output="$4"
  validate_release_host "$host"
  [[ -f "$package" && ! -L "$package" ]] || {
    echo "❌ cannot lock missing/non-regular Linux package: $package" >&2
    exit 1
  }
  bun "$RELEASE_LOCK_TOOL" create \
    --root "$ROOT" \
    --platform "$target" \
    --host "$host" \
    --package "$package" \
    --legal "$ROOT/LICENSE" \
    --legal "$ROOT/NOTICE" \
    --output "$output" >/dev/null
  bun "$RELEASE_LOCK_TOOL" verify --root "$ROOT" --lock "$output" >/dev/null
}

mkdir -p "$DIST"

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "将依次构建以下目标: ${TRIPLES[*]}"
echo "产物汇总目录: $DIST"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

COLLECTED=()
COLLECTED_LOCKS=()
for t in "${TRIPLES[@]}"; do
  ensure_target "$t"
  if [[ "$t" != "$HOST_TRIPLE" ]]; then
    echo "⚠️  $t 与当前机器架构($HOST_TRIPLE)不同,正在尝试交叉编译——"
    echo "    若链接 webkit2gtk 等系统库失败,请改到目标架构的原生环境/容器构建。"
  fi
  echo ""
  echo "▶▶▶ 构建 $t ..."
  # A previous --bundles run may have left packages for other formats or
  # versions. Never bind those bytes to this build's Host/source release lock.
  # Only remove generated Linux bundles for this target; keep other platforms
  # and the already collected dist artifacts untouched.
  bundle_dir="$ROOT/target/$t/release/bundle"
  if [[ -d "$bundle_dir" ]]; then
    find "$bundle_dir" -type f \( -name '*.deb' -o -name '*.AppImage' -o -name '*.rpm' -o -name '*.sig' \) -delete
  fi
  CI=true bun x tauri build --config "$CONF" --target "$t" ${PASSTHRU[@]+"${PASSTHRU[@]}"}

  # Linux 产物在 target/<triple>/release/bundle/{deb,appimage,rpm}/
  host="$ROOT/target/$t/release/nomifun-desktop"
  target_package_count=0
  while IFS= read -r -d '' pkg; do
    package="$DIST/$(basename "$pkg")"
    lock="$package.release-lock.json"
    cp -f "$pkg" "$package"
    write_release_lock "$t" "$host" "$package" "$lock"
    COLLECTED+=("$package")
    COLLECTED_LOCKS+=("$lock")
    target_package_count=$((target_package_count + 1))
  done < <(find "$bundle_dir" -type f \( -name '*.deb' -o -name '*.AppImage' -o -name '*.rpm' \) -print0 2>/dev/null)
  if [[ "$target_package_count" -eq 0 ]]; then
    echo "❌ Tauri did not produce any Linux Desktop package for $t." >&2
    exit 1
  fi
done

if [[ "${#COLLECTED[@]}" -eq 0 ]]; then
  echo "❌ Tauri did not produce any Linux Desktop package." >&2
  exit 1
fi

echo ""
echo "▶ 清理 Linux 构建后 debug/flycheck 中间产物(保留 release 安装包与 updater 签名)..."
bun scripts/prune-build.mjs --post

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "✅ 全部完成,安装包已汇总到 $DIST :"
for f in "${COLLECTED[@]}"; do
  size="$(du -h "$f" | cut -f1)"
  printf "   %-44s %s\n" "$(basename "$f")" "$size"
done
echo "Release locks:"
for f in "${COLLECTED_LOCKS[@]}"; do
  printf "   %s\n" "$(basename "$f")"
done
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
