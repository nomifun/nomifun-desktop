# ============================================================================
# 一键 Windows 发版：补齐 Windows 产物到「已存在的」GitHub Release。
#
#   bun run release:win              # 全自动：pull→校验→构建→合并→上传→提交推送→校验
#   bun run release:win -DryRun      # 只跑只读前置检查并打印计划，不构建/不上传/不推送
#   bun run release:win -NoPush      # 构建+上传，但不提交/推送 latest.json（改动留本地）
#   bun run release:win -SkipPull    # 跳过 git pull（本地已是目标版本时）
#
# 前提（一次性配置好后即可反复一键）：
#   1) 桌面版发布模型：Mac 侧先 `bun run build:mac` + 建好 GitHub Release（含 macOS 资产 +
#      latest.json 的 darwin 条目）。本脚本只负责在 Windows 上「补」Windows 条目。
#   2) apps/desktop/signing/nomifun-updater.key —— updater 私钥（keyID F3AA272E60AA7952），
#      从密钥库拷入；已被 gitignore。必须与 tauri.conf.json 内嵌 pubkey 匹配。
#   3) apps/desktop/signing/.env.release —— 内含 `GH_TOKEN=...`（repo 或 Contents:rw 权限的
#      GitHub PAT）；已被 gitignore。见 .env.release.example。也可改为先设 $env:GH_TOKEN。
#
# 说明：本轮不启用 Windows Authenticode 签名（手动安装包仍可能触发 SmartScreen / 未知发布者）。
#   自动更新验签用 Tauri updater 的 minisign 签名，与 Authenticode 无关，此脚本已覆盖。
#   有代码签名证书后再走 build:win --signed（需 pwsh 7+，见 desktop-build-win.ps1）。
# ============================================================================
param(
  [switch]$DryRun,
  [switch]$NoPush,
  [switch]$SkipPull
)
$ErrorActionPreference = 'Stop'

# Windows PowerShell 5.1 的控制台默认 OEM 代码页，先切 UTF-8 以正确输出中文；pwsh 7+ 无害。
try { [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 } catch {}

if ($null -ne $IsWindows) { $onWindows = [bool]$IsWindows } else { $onWindows = ($env:OS -eq 'Windows_NT') }
if (-not $onWindows) { Write-Error "release:win 只能在 Windows 上运行。macOS 用 build:mac，Linux 用 build:linux。"; exit 1 }

# 切到仓库根，后续全用相对路径。
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$Root      = Split-Path -Parent $ScriptDir
Set-Location $Root

$Repo        = 'nomifun/nomifun-tauri'
$Triple      = 'x86_64-pc-windows-msvc'
$KeyFile     = 'apps/desktop/signing/nomifun-updater.key'
$EnvRelease  = 'apps/desktop/signing/.env.release'
$UpdaterConf = 'apps/desktop/tauri.updater.conf.json'
$LatestJson  = 'apps/desktop/updater/latest.json'

function Fail($msg) { Write-Error $msg; exit 1 }

# ── gh CLI ──────────────────────────────────────────────────────────────────
function Resolve-Gh {
  $cmd = Get-Command gh -ErrorAction SilentlyContinue
  if ($cmd) { return $cmd.Source }
  $fallback = Join-Path $env:ProgramFiles 'GitHub CLI\gh.exe'
  if (Test-Path $fallback) { return $fallback }
  return $null
}
$gh = Resolve-Gh
if (-not $gh) { Fail "未找到 gh CLI。安装：winget install --id GitHub.cli -e --source winget" }

# ── GH_TOKEN：优先用已存在的环境变量，否则从 .env.release 读取 ─────────────────
if (-not $env:GH_TOKEN) {
  if (Test-Path $EnvRelease) {
    foreach ($line in Get-Content $EnvRelease) {
      $t = $line.Trim()
      if ($t -eq '' -or $t.StartsWith('#')) { continue }
      if ($t -match '^\s*GH_TOKEN\s*=\s*(.+)$')      { $env:GH_TOKEN = $Matches[1].Trim() }
      elseif ($t -match '^(ghp_|github_pat_)')        { $env:GH_TOKEN = $t }
    }
  }
}
if (-not $env:GH_TOKEN) {
  Fail "缺少 GH_TOKEN。请在 $EnvRelease 里填 `GH_TOKEN=...`（见 .env.release.example），或先设 `$env:GH_TOKEN。"
}

# ── updater 私钥 ─────────────────────────────────────────────────────────────
if (-not (Test-Path $KeyFile)) { Fail "缺少 updater 私钥 $KeyFile（从密钥库拷入，keyID F3AA272E60AA7952）。" }
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content $KeyFile -Raw
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ""

# ── 同步代码 ─────────────────────────────────────────────────────────────────
if (-not $SkipPull) {
  Write-Host "▶ git pull --ff-only origin main"
  git pull --ff-only origin main
  if ($LASTEXITCODE -ne 0) { Fail "git pull 失败（可能本地有分叉或未提交改动）。处理后重试，或加 -SkipPull。" }
}

# ── 单一真源版本号：根 Cargo.toml [workspace.package].version ─────────────────
function Read-WorkspaceVersion {
  $inSection = $false
  foreach ($line in Get-Content 'Cargo.toml') {
    $t = $line.Trim()
    if ($t.StartsWith('[')) { $inSection = ($t -eq '[workspace.package]'); continue }
    if ($inSection -and ($line -match '^\s*version\s*=\s*"([^"]+)"')) { return $Matches[1] }
  }
  return $null
}
$Version = Read-WorkspaceVersion
if (-not $Version) { Fail "无法从 Cargo.toml 读取 [workspace.package].version。" }
$Tag = "v$Version"

# ── token 有效性 + Release 必须已存在 ────────────────────────────────────────
$login = & $gh api user --jq '.login'
if ($LASTEXITCODE -ne 0) { Fail "GH_TOKEN 无效或无权限。请更新 $EnvRelease 里的 token。" }
Write-Host "▶ gh 认证 OK（账号 $login）"

& $gh release view $Tag --repo $Repo --json tagName | Out-Null
if ($LASTEXITCODE -ne 0) { Fail "Release $Tag 不存在。请先在 Mac 侧创建该 Release（发 macOS 包时）。" }

$NsisDir = "target/$Triple/release/bundle/nsis"
$Exe     = "$NsisDir/NomiFun_${Version}_x64-setup.exe"
$Sig     = "$Exe.sig"

Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
Write-Host "一键 Windows 发版计划"
Write-Host "  版本      : $Version   (tag $Tag)"
Write-Host "  仓库      : $Repo"
Write-Host "  目标产物  : $Exe (+ .sig)"
Write-Host "  清单      : $LatestJson （补 windows-x86_64 条目后 --clobber 上传）"
if ($NoPush) { Write-Host "  提交推送  : 关闭 (-NoPush)" } else { Write-Host "  提交推送  : 开启 (author=nomifun -> origin main)" }
Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

if ($DryRun) {
  Write-Host "✅ -DryRun：前置检查全部通过（gh / token / Release / 私钥 / 版本），未执行构建与上传。"
  exit 0
}

# ── 清理旧版本残留产物：避免 make:latest 扫到旧 .sig 选错文件 ─────────────────
if (Test-Path $NsisDir) {
  Get-ChildItem -Path $NsisDir -File |
    Where-Object { ($_.Extension -eq '.exe' -or $_.Extension -eq '.sig') -and ($_.Name -notlike "*$Version*") } |
    ForEach-Object { Write-Host "  清理旧产物: $($_.Name)"; Remove-Item $_.FullName -Force }
}

# ── 构建自动更新产物（NSIS .exe + .sig）──────────────────────────────────────
Write-Host "▶ 构建 Windows 自动更新产物（Rust release，耗时较长）..."
& bun run build:win --config $UpdaterConf
if ($LASTEXITCODE -ne 0) { Fail "构建失败。" }
if (-not (Test-Path $Exe)) { Fail "构建后未找到产物: $Exe" }
if (-not (Test-Path $Sig)) { Fail "构建后未找到 updater 签名: $Sig" }

# ── 合并 latest.json（补 windows-x86_64；notes 取当前版本小节）────────────────
Write-Host "▶ 合并 latest.json ..."
& bun run make:latest
if ($LASTEXITCODE -ne 0) { Fail "make:latest 失败。" }

$manifest = Get-Content $LatestJson -Raw | ConvertFrom-Json
if ($manifest.version -ne $Version) { Write-Warning "latest.json version($($manifest.version)) != $Version" }
if (-not $manifest.platforms.'windows-x86_64') { Fail "latest.json 缺少 windows-x86_64 条目，合并异常。" }
if ($manifest.notes -match 'No unreleased changes') { Write-Warning "latest.json notes 疑似占位——检查 CHANGELOG 是否有 ## $Tag 小节。" }

# ── 上传资产 + 覆盖 latest.json ──────────────────────────────────────────────
Write-Host "▶ 上传 Windows 资产到 Release $Tag ..."
& $gh release upload $Tag --repo $Repo $Exe $Sig $LatestJson --clobber
if ($LASTEXITCODE -ne 0) { Fail "上传失败。" }

# ── 提交 latest.json 回 main（author/committer = nomifun）────────────────────
if ($NoPush) {
  Write-Host "  -NoPush：跳过提交/推送，latest.json 改动留在本地。"
} else {
  git diff --quiet -- $LatestJson
  if ($LASTEXITCODE -ne 0) {
    Write-Host "▶ 提交 latest.json 回 main (author=nomifun) ..."
    git add $LatestJson
    git -c user.name=nomifun commit -m "chore(release): add Windows x64 updater entry to $Tag latest.json"
    if ($LASTEXITCODE -ne 0) { Fail "commit 失败。" }
    git push origin main
    if ($LASTEXITCODE -ne 0) { Fail "push 失败。" }
  } else {
    Write-Host "  latest.json 无变化，跳过提交。"
  }
}

# ── 发布后校验 ───────────────────────────────────────────────────────────────
Write-Host "▶ 发布后校验（updater 端点）..."
$endpoint = "https://github.com/$Repo/releases/latest/download/latest.json"
try {
  $resp = Invoke-WebRequest -Uri $endpoint -UseBasicParsing -MaximumRedirection 10
  $pub  = $resp.Content | ConvertFrom-Json
  Write-Host "  version   : $($pub.version)"
  Write-Host "  platforms : $($pub.platforms.PSObject.Properties.Name -join ', ')"
  Write-Host "  windows   : $($pub.platforms.'windows-x86_64'.url)"
  if ($pub.version -ne $Version) { Write-Warning "published version($($pub.version)) != $Version（CDN 可能有缓存延迟）。" }
  if (-not $pub.platforms.'windows-x86_64') { Write-Warning "published latest.json 缺少 windows-x86_64。" }
} catch {
  Write-Warning "校验拉取失败: $($_.Exception.Message)（CDN 缓存延迟，可稍后手动核对）。"
}

Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
Write-Host "✅ Windows 发版完成：$Tag"
Write-Host "   Release: https://github.com/$Repo/releases/tag/$Tag"
Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
