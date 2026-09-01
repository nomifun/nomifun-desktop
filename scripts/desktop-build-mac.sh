#!/usr/bin/env bash
# ============================================================================
# 打 macOS 桌面端安装包(.dmg),并汇总到 dist/desktop/。仅能在 macOS 上运行。
#
#   bun run build:mac                 # 默认只打 Universal 一个 DMG(不签名)
#   bun run build:mac --signed        # 默认 Universal,带 Developer ID 签名 + 公证
#   bun run build:mac arm intel       # 显式指定架构(可多选,空格分隔)
#   bun run build:mac --signed intel  # 只打 Intel,且签名+公证
#   bun run build:mac --config '{"bundle":{"createUpdaterArtifacts":true}}'
#                                     # 未知 --xxx 选项会原样透传给 tauri build
#   bun run build:mac arm --config '{"bundle":{"createUpdaterArtifacts":true}}'
#                                     # 架构参数仍放在 tauri build 参数之前
#
# 架构别名:
#   arm / aarch64 / silicon  -> aarch64-apple-darwin   (Apple Silicon 原生)
#   intel / x64 / x86_64     -> x86_64-apple-darwin     (Intel 原生 / M 系 Rosetta)
#   universal / all-arch     -> universal-apple-darwin  (二合一胖包,通吃两种 Mac)
#
# 缺失的 Rust 编译目标会自动 `rustup target add`。
#
# 签名(--signed)说明:
#   密钥/口令全部来自本地 apps/desktop/signing/.env.signing(已 gitignore,绝不入库),
#   与 build:signed 用同一份配置。签名在 tauri build 阶段由环境变量自动完成,公证在
#   每个 target 构建结束后对其 DMG 逐个提交 Apple 并 staple。文件不存在时直接报错。
#
# 注:Windows / Linux 包无法在 macOS 上交叉构建,请到对应系统上分别用
#     bun run build:win / build:linux。
# ============================================================================
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "❌ build:mac 只能在 macOS 上运行(当前: $(uname -s))。" >&2
  echo "   Windows 包用 build:win,Linux 包用 build:linux,且都需在对应系统上构建。" >&2
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CONF="apps/desktop/tauri.conf.json"
MAC_CONF="apps/desktop/tauri.macos.conf.json"
DIST="$ROOT/dist/desktop"
RELEASE_INPUT="$ROOT/vendor/codex-runtime/release-input.json"
RUNTIME_STAGE="$ROOT/target/nomifun-runtime"
CHECK_ONLY=0

# ── 解析参数:架构选择/开关归本脚本,未知 --xxx 起原样透传给 tauri build ─────
SELECT=()
PASSTHRU=()
SIGNED=0
seen_dashdash=0
for arg in "$@"; do
  if [[ "$seen_dashdash" -eq 1 ]]; then
    PASSTHRU+=("$arg")
  elif [[ "$arg" == "--" ]]; then
    seen_dashdash=1
  elif [[ "$arg" == "--signed" ]]; then
    SIGNED=1
  elif [[ "$arg" == "--check" || "$arg" == "--check-only" ]]; then
    CHECK_ONLY=1
  elif [[ "$arg" == --* ]]; then
    PASSTHRU+=("$arg")
    seen_dashdash=1
  else
    SELECT+=("$arg")
  fi
done

require_tool() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "❌ macOS packaging requires '$1'." >&2
    exit 1
  }
}

for tool in bun rustup shasum file lipo install node; do
  require_tool "$tool"
done

[[ -f "$RELEASE_INPUT" ]] || {
  echo "❌ missing vendored Codex Runtime release input: $RELEASE_INPUT" >&2
  exit 1
}
[[ -f "$ROOT/$MAC_CONF" ]] || {
  echo "❌ missing macOS Tauri resource overlay: $ROOT/$MAC_CONF" >&2
  exit 1
}

ensure_exact_case_path() {
  local path="$1"
  node - "$path" <<'NODE'
const fs = require('node:fs');
const path = require('node:path');
const target = path.resolve(process.argv[2]);
const parts = target.split(path.sep);
let current = parts[0] === '' ? path.sep : parts.shift();
for (const part of parts) {
  if (!part) continue;
  let names;
  try {
    names = fs.readdirSync(current);
  } catch (error) {
    console.error(`cannot inspect path component ${current}: ${error.message}`);
    process.exit(1);
  }
  if (!names.includes(part)) {
    const folded = names.find((name) => name.toLowerCase() === part.toLowerCase());
    console.error(
      folded
        ? `path case mismatch: expected ${part}, found ${folded} under ${current}`
        : `path component does not exist: ${part} under ${current}`,
    );
    process.exit(1);
  }
  current = path.join(current, part);
}
NODE
}

validate_regular_executable() {
  local path="$1"
  [[ -f "$path" && ! -L "$path" ]] || {
    echo "❌ expected a regular non-symlink file: $path" >&2
    exit 1
  }
  ensure_exact_case_path "$path" || exit 1
  node - "$path" <<'NODE'
const fs = require('node:fs');
const target = process.argv[2];
const stat = fs.lstatSync(target);
if (!stat.isFile() || (stat.mode & 0o111) === 0 || (stat.mode & 0o022) !== 0) {
  console.error(`invalid executable permissions or file type: ${target}`);
  process.exit(1);
}
NODE
}

# 把别名规整成 rustc target triple
resolve_triple() {
  case "$1" in
    arm|aarch64|silicon|aarch64-apple-darwin)        echo "aarch64-apple-darwin" ;;
    intel|x64|x86_64|x86_64-apple-darwin)            echo "x86_64-apple-darwin" ;;
    universal|all-arch|universal-apple-darwin)       echo "universal-apple-darwin" ;;
    *) echo "❌ 未知架构: $1 (可选: arm / intel / universal)" >&2; exit 1 ;;
  esac
}

TRIPLES=()
if [[ "${#SELECT[@]}" -eq 0 ]]; then
  # 默认只打 Universal 一个胖包:原生通吃 Intel + Apple Silicon,体验与单架构包无异,
  # 只多占下载/磁盘体积,却省掉多轮编译与 Apple 公证等待。需要单架构包时显式指定。
  TRIPLES=(universal-apple-darwin)
else
  for s in "${SELECT[@]}"; do
    TRIPLES+=("$(resolve_triple "$s")")
  done
fi

# ── 确保所需 Rust target 已安装(universal 需要底层两个 target 都在) ──────────
ensure_target() {
  local t="$1"
  if ! rustup target list --installed | grep -qx "$t"; then
    echo "▶ 安装 Rust target: $t"
    rustup target add "$t"
  fi
}
for t in "${TRIPLES[@]}"; do
  if [[ "$t" == "universal-apple-darwin" ]]; then
    ensure_target aarch64-apple-darwin
    ensure_target x86_64-apple-darwin
  else
    ensure_target "$t"
  fi
done

runtime_target_id() {
  case "$1" in
    arm64) echo "macos_desktop_arm64" ;;
    x86_64) echo "macos_desktop_x64" ;;
    *) echo "❌ unsupported macOS runtime architecture: $1" >&2; exit 1 ;;
  esac
}

runtime_env_suffix() {
  case "$1" in
    arm64) echo "ARM64" ;;
    x86_64) echo "X64" ;;
    *) echo "❌ unsupported macOS runtime architecture: $1" >&2; exit 1 ;;
  esac
}

runtime_resource_dir() {
  case "$1" in
    arm64) echo "arm64" ;;
    x86_64) echo "x64" ;;
    *) echo "❌ unsupported macOS runtime architecture: $1" >&2; exit 1 ;;
  esac
}

runtime_source_path() {
  local arch="$1"
  local variable="NOMIFUN_CODEX_RUNTIME_$(runtime_env_suffix "$arch")_PATH"
  local value="${!variable:-}"
  if [[ -z "$value" && -n "${NOMIFUN_CODEX_RUNTIME_DIR:-}" ]]; then
    value="$NOMIFUN_CODEX_RUNTIME_DIR/runtime/macos/$(runtime_resource_dir "$arch")/nomifun-codex-runtime"
  fi
  [[ -n "$value" ]] || {
    echo "❌ missing real Codex Runtime sidecar for macOS $arch." >&2
    echo "   Set $variable to the externally supplied native executable." >&2
    echo "   No sidecar is fabricated or substituted by this script." >&2
    exit 1
  }
  [[ "$value" == /* ]] || {
    echo "❌ $variable must be an absolute path: $value" >&2
    exit 1
  }
  printf '%s\n' "$value"
}

runtime_hello_path() {
  local arch="$1"
  local variable="NOMIFUN_CODEX_RUNTIME_$(runtime_env_suffix "$arch")_HELLO_PATH"
  local value="${!variable:-}"
  if [[ -z "$value" ]]; then
    value="$(runtime_source_path "$arch").hello.json"
  fi
  [[ "$value" == /* ]] || {
    echo "❌ $variable must be an absolute path: $value" >&2
    exit 1
  }
  printf '%s\n' "$value"
}

expected_sidecar_digest() {
  local target_id="$1"
  node - "$RELEASE_INPUT" "$target_id" <<'NODE'
const fs = require('node:fs');
const [manifestPath, targetId] = process.argv.slice(2);
const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
const digest = manifest.target_matrix?.[targetId]?.sidecar_artifact?.digest;
if (!digest) {
  console.error(`release input has no required sidecar digest for ${targetId}`);
  process.exit(1);
}
console.log(digest.toLowerCase());
NODE
}

validate_runtime_hello() {
  local arch="$1"
  local target_id
  target_id="$(runtime_target_id "$arch")"
  local hello_path
  hello_path="${2:-$(runtime_hello_path "$arch")}"
  [[ -f "$hello_path" && ! -L "$hello_path" ]] || {
    echo "❌ missing real Runtime hello metadata for macOS $arch: $hello_path" >&2
    echo "   Set NOMIFUN_CODEX_RUNTIME_$(runtime_env_suffix "$arch")_HELLO_PATH or place the exact .hello.json beside the sidecar." >&2
    exit 1
  }
  node - "$RELEASE_INPUT" "$hello_path" "$target_id" <<'NODE'
const fs = require('node:fs');
const crypto = require('node:crypto');
const [releasePath, helloPath, targetId] = process.argv.slice(2);
const release = JSON.parse(fs.readFileSync(releasePath, 'utf8'));
const hello = JSON.parse(fs.readFileSync(helloPath, 'utf8'));

function canonical(value) {
  if (Array.isArray(value)) return value.map(canonical);
  if (value && typeof value === 'object') {
    return Object.fromEntries(Object.keys(value).sort().map((key) => [key, canonical(value[key])]));
  }
  return value;
}
function digest(value) {
  return crypto.createHash('sha256').update(JSON.stringify(canonical(value))).digest('hex');
}
function sortedStrings(value, field) {
  if (!Array.isArray(value) || value.some((item) => typeof item !== 'string')) {
    throw new Error(`${field} must be an array of strings`);
  }
  return [...new Set(value)].sort();
}
function exact(actual, expected, field) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(`${field} does not match the pinned release contract`);
  }
}

try {
  const expectedKeys = [
    'runtime_release_digest', 'runtime_build_digest', 'fork_commit',
    'tracked_upstream_commit', 'protocol_version', 'protocol_schema_digest',
    'runtime_target', 'supported_profiles', 'native_features', 'native_actions',
    'full_auto', 'rpc_allowlist',
  ].sort();
  exact(Object.keys(hello).sort(), expectedKeys, 'hello fields');
  const target = release.target_matrix?.[targetId];
  if (!target?.runtime_target) throw new Error(`release input has no runtime target for ${targetId}`);
  exact(hello.runtime_release_digest, digest(release), 'runtime_release_digest');
  exact(hello.runtime_target, target.runtime_target, 'runtime_target');
  exact(hello.fork_commit, release.fork_commit, 'fork_commit');
  exact(hello.tracked_upstream_commit, release.tracked_upstream_commit, 'tracked_upstream_commit');
  exact(hello.protocol_version, release.protocol_version, 'protocol_version');
  exact(hello.protocol_schema_digest, release.protocol_schema_digest, 'protocol_schema_digest');
  exact(sortedStrings(hello.supported_profiles, 'supported_profiles'), sortedStrings(release.supported_profiles, 'supported_profiles'), 'supported_profiles');
  exact(hello.full_auto, release.full_auto, 'full_auto');
  exact(sortedStrings(hello.rpc_allowlist.methods, 'rpc_allowlist.methods'), sortedStrings(release.rpc_allowlist.methods, 'rpc_allowlist.methods'), 'rpc_allowlist.methods');
  exact(sortedStrings(hello.rpc_allowlist.experimental_methods, 'rpc_allowlist.experimental_methods'), [], 'rpc_allowlist.experimental_methods');
  if (!/^[0-9a-f]{64}$/i.test(hello.runtime_build_digest)) throw new Error('runtime_build_digest must be a SHA-256 hex digest');
  sortedStrings(hello.native_features, 'native_features');
  sortedStrings(hello.native_actions, 'native_actions');
} catch (error) {
  console.error(`❌ invalid Runtime hello metadata ${helloPath}: ${error.message}`);
  process.exit(1);
}
NODE
}

stage_runtime_sidecar() {
  local arch="$1"
  local target_id
  target_id="$(runtime_target_id "$arch")"
  local source
  source="$(runtime_source_path "$arch")"
  local hello
  hello="$(runtime_hello_path "$arch")"
  validate_regular_executable "$source"
  local archs
  archs="$(lipo -archs "$source" 2>/dev/null)" || {
    echo "❌ Runtime sidecar is not a valid macOS Mach-O executable: $source" >&2
    exit 1
  }
  local file_kind
  file_kind="$(file -b "$source")"
  [[ "$file_kind" == *"Mach-O"* ]] || {
    echo "❌ Runtime sidecar is not identified as Mach-O: $file_kind" >&2
    exit 1
  }
  case "$arch" in
    arm64) [[ "$archs" == "arm64" ]] || { echo "❌ arm64 sidecar has architectures: $archs" >&2; exit 1; } ;;
    x86_64) [[ "$archs" == "x86_64" ]] || { echo "❌ x86_64 sidecar has architectures: $archs" >&2; exit 1; } ;;
  esac
  validate_runtime_hello "$arch" "$hello"

  local expected actual destination destination_hello
  expected="$(expected_sidecar_digest "$target_id")"
  actual="$(shasum -a 256 "$source" | awk '{print tolower($1)}')"
  [[ "$actual" == "$expected" ]] || {
    echo "❌ Runtime sidecar SHA-256 mismatch for $target_id: expected $expected, got $actual" >&2
    exit 1
  }
  destination="$RUNTIME_STAGE/runtime/macos/$(runtime_resource_dir "$arch")/nomifun-codex-runtime"
  destination_hello="$destination.hello.json"
  mkdir -p "$(dirname "$destination")"
  install -m 755 "$source" "$destination"
  install -m 644 "$hello" "$destination_hello"
  actual="$(shasum -a 256 "$destination" | awk '{print tolower($1)}')"
  [[ "$actual" == "$expected" ]] || {
    echo "❌ staged Runtime sidecar digest changed during copy: $destination" >&2
    exit 1
  }
  [[ -x "$destination" && ! -L "$destination" && ! -L "$destination_hello" ]] || {
    echo "❌ staged Runtime resources have invalid permissions or symlinks" >&2
    exit 1
  }
  ensure_exact_case_path "$destination"
  ensure_exact_case_path "$destination_hello"
}

stage_runtime_resources() {
  [[ ! -L "$RUNTIME_STAGE" ]] || {
    echo "❌ refusing to replace symlinked Runtime staging root: $RUNTIME_STAGE" >&2
    exit 1
  }
  rm -rf "$RUNTIME_STAGE"
  mkdir -p "$RUNTIME_STAGE"
  local required=()
  for t in "${TRIPLES[@]}"; do
    if [[ "$t" == "universal-apple-darwin" ]]; then
      required+=(arm64 x86_64)
    elif [[ "$t" == "aarch64-apple-darwin" ]]; then
      required+=(arm64)
    else
      required+=(x86_64)
    fi
  done
  local seen=" "
  for arch in "${required[@]}"; do
    [[ "$seen" == *" $arch "* ]] && continue
    seen+="$arch "
    echo "▶ 校验并暂存真实 Codex Runtime sidecar: macOS $arch"
    stage_runtime_sidecar "$arch"
  done
}

verify_bundled_runtime() {
  local app="$1"
  local arch="$2"
  local target_id
  target_id="$(runtime_target_id "$arch")"
  local executable="$app/Contents/Resources/runtime/macos/$(runtime_resource_dir "$arch")/nomifun-codex-runtime"
  local hello="$executable.hello.json"
  validate_regular_executable "$executable"
  [[ -f "$hello" && ! -L "$hello" ]] || {
    echo "❌ packaged Runtime hello metadata missing or symlink: $hello" >&2
    exit 1
  }
  ensure_exact_case_path "$hello"
  local expected actual
  expected="$(expected_sidecar_digest "$target_id")"
  actual="$(shasum -a 256 "$executable" | awk '{print tolower($1)}')"
  [[ "$actual" == "$expected" ]] || {
    echo "❌ packaged Runtime sidecar digest mismatch for $target_id: expected $expected, got $actual" >&2
    exit 1
  }
  validate_runtime_hello "$arch" "$hello"
}

verify_macos_app() {
  local app="$1"
  local target="$2"
  local binary="$app/Contents/MacOS/nomifun-desktop"
  [[ -d "$app" && -f "$binary" ]] || {
    echo "❌ Tauri did not produce the expected macOS app: $app" >&2
    exit 1
  }
  local archs
  archs="$(lipo -archs "$binary" 2>/dev/null)" || {
    echo "❌ packaged app executable is not a valid Mach-O binary: $binary" >&2
    exit 1
  }
  if [[ "$target" == "universal-apple-darwin" ]]; then
    [[ "$archs" == *"arm64"* && "$archs" == *"x86_64"* ]] || {
      echo "❌ Universal app is missing arm64 or x86_64 slice: $archs" >&2
      exit 1
    }
    verify_bundled_runtime "$app" arm64
    verify_bundled_runtime "$app" x86_64
  elif [[ "$target" == "aarch64-apple-darwin" ]]; then
    [[ "$archs" == "arm64" ]] || { echo "❌ arm64 app has architectures: $archs" >&2; exit 1; }
    verify_bundled_runtime "$app" arm64
  else
    [[ "$archs" == "x86_64" ]] || { echo "❌ x86_64 app has architectures: $archs" >&2; exit 1; }
    verify_bundled_runtime "$app" x86_64
  fi
}

stage_runtime_resources
if [[ "$CHECK_ONLY" -eq 1 ]]; then
  echo "✅ macOS Runtime packaging preflight passed; no app or DMG was built."
  exit 0
fi

# ── 签名:加载本地密钥并做基本校验(公共库,与 build:signed 共用一份实现) ────────
# shellcheck source=lib/mac-signing.sh
source "$SCRIPT_DIR/lib/mac-signing.sh"

HAS_NOTARY=0
if [[ "$SIGNED" -eq 1 ]]; then
  load_signing_env "$ROOT"
  require_signing_identity
  detect_notary

  echo "▶ 签名身份: ${APPLE_SIGNING_IDENTITY:-(用 .p12: APPLE_CERTIFICATE)}"
  [[ "$HAS_NOTARY" -eq 1 ]] && echo "▶ 公证: 已启用,每个 target 构建后自动提交 Apple 并 staple"
fi

mkdir -p "$DIST"

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "将依次构建以下目标: ${TRIPLES[*]}"
[[ "$SIGNED" -eq 1 ]] && echo "签名: 开启 (公证: $([[ "$HAS_NOTARY" -eq 1 ]] && echo 开启 || echo 关闭))" || echo "签名: 关闭 (本地测试包)"
echo "产物汇总目录: $DIST"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

COLLECTED=()
for t in "${TRIPLES[@]}"; do
  echo ""
  echo "▶▶▶ 构建 $t ..."
  # bundle.targets 在 tauri.conf.json 里被钉成 ["nsis"](仅给 Windows 用),
  # macOS 上那是无效目标 —— 不覆盖的话 tauri 只编出二进制、不产 .app/.dmg。
  # 用第二个 --config 覆盖成 macOS 的 app+dmg(与 build:updater 同款叠加写法)。
  CI=true bun x tauri build --config "$CONF" --config "$MAC_CONF" \
    --config '{"bundle":{"targets":["app","dmg"]}}' \
    --target "$t" ${PASSTHRU[@]+"${PASSTHRU[@]}"}

  # tauri 把 DMG 放在 target/<triple>/release/bundle/dmg/*.dmg
  dmg_dir="$ROOT/target/$t/release/bundle/dmg"
  app="$ROOT/target/$t/release/bundle/macos/NomiFun.app"
  verify_macos_app "$app" "$t"

  # 先公证(staple 会原地改写 DMG),再拷贝到汇总目录,保证收的是带票据的包
  notarize_dmg_dir "$dmg_dir"

  while IFS= read -r -d '' dmg; do
    cp -f "$dmg" "$DIST/"
    COLLECTED+=("$DIST/$(basename "$dmg")")
  done < <(find "$dmg_dir" -maxdepth 1 -type f -name '*.dmg' -print0 2>/dev/null)
done

# COLLECTED 为空 = 这一轮没产出任何 DMG(多半 bundle.targets 不含 dmg)。
# bash 3.2 下 `set -u` 还会让空数组展开直接报 unbound variable,所以这里既兜底
# 又把真正的失败原因说清楚,而不是假装「✅ 全部完成」。
if [[ "${#COLLECTED[@]}" -eq 0 ]]; then
  echo "" >&2
  echo "❌ 没有收集到任何 DMG —— tauri build 没在 macOS 上产出安装包。" >&2
  echo "   回看上面 tauri build 的输出:应出现 Bundling …dmg / Finished N bundles。" >&2
  echo "   若没有,多半是 bundle.targets 不含 dmg(本脚本本应已用 --config 覆盖)。" >&2
  exit 1
fi

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "✅ 全部完成,DMG 已汇总到 $DIST :"
for f in "${COLLECTED[@]}"; do
  size="$(du -h "$f" | cut -f1)"
  printf "   %-40s %s\n" "$(basename "$f")" "$size"
done
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
