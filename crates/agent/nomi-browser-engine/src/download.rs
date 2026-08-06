//! **E4：下载沙箱 + 可执行 denylist + 不自动打开 + Win MOTW**（DESIGN §16 / P2 裁决⑩）。
//!
//! 三条铁律（与 DESIGN §16「下载沙箱」一行对齐）：
//! 1. **隔离落点**：下载经 `Browser.setDownloadBehavior{behavior:"allowAndName"}` 落进
//!    **per-pet workspace 的隔离 `downloads/` 子目录**（[`download_dir`]）——**绝不**落用户真实
//!    Downloads。`allowAndName` 让 Chrome 用下载 GUID 命名落盘文件，规避「同名覆盖 / 路径穿越」
//!    类攻击（攻击者无法预测/控制最终文件名）。
//! 2. **不自动打开**：下载完成**绝不** shell-execute / 启动 / 打开文件。本模块只把文件留在隔离
//!    目录 + 打 MOTW 标记；是否进一步处理由上层（用户）显式发起。
//! 3. **可执行红线**：命中 [`is_executable_denylist`] 的下载，在 **yolo / companion 无 UI 场景也
//!    fail-closed 拒绝**（裁决⑧/⑩「会话红线」）。E4 提供 **denylist 纯函数** + **拒绝判定**
//!    （[`reject_executable_download`] 返 [`BrowserError::Blocked`]）；真正的 facade enforcement
//!    接线由 F1（`BrowserTool::execute` 独立 fail-closed 门）落实——见
//!    `TODO(E4->F1-enforce-download-redline)`。
//!
//! **Win MOTW**（Mark-of-the-Web，Windows 专属）：下载文件写 `Zone.Identifier`
//! [Alternate Data Stream](https://learn.microsoft.com/windows/win32/fileio/file-streams)
//! （ADS），内容 `[ZoneTransfer]\r\nZoneId=3`（3 = Internet 区），触发 SmartScreen 信誉检查 /
//! Office「受保护的视图」。见 [`write_motw`]。**macOS 等价**：显式设 `com.apple.quarantine`
//! xattr（Gatekeeper），见 [`write_motw`] 的 mac 分支。**Linux** 无内核级等价 → no-op（仅靠隔离
//! 目录 + denylist + 不自动打开）——登记在
//! `docs/superpowers/specs/browser-use/PLATFORM-VERIFICATION.md` 的 `TODO(verify-linux)`。

use std::path::{Path, PathBuf};

use base64::Engine as _;

use crate::engine::BrowserError;

/// per-pet workspace 下的隔离下载子目录名。下载**只**落 `<workspace>/<DOWNLOAD_SUBDIR>/`，
/// 与会话产物/知识库等其它子目录平级隔离，且**绝不**指向用户真实 Downloads。
pub const DOWNLOAD_SUBDIR: &str = "downloads";

/// MOTW（Mark-of-the-Web）`Zone.Identifier` ADS 的内容。
///
/// `ZoneId=3` = `URLZONE_INTERNET`（来自 Internet）——这是触发 Windows SmartScreen 信誉检查 +
/// Office「受保护的视图」的关键值。`[ZoneTransfer]` 段头 + CRLF 行尾是 Windows 约定格式
/// （NTFS ADS 里 Explorer/Office 解析的就是这个 INI 形态）。
pub const MOTW_ZONE_INTERNET: &str = "[ZoneTransfer]\r\nZoneId=3\r\n";

/// 解析 per-pet workspace 的隔离下载目录 `<workspace>/downloads`。
///
/// `Browser.setDownloadBehavior` 的 `downloadPath` 用它（绝对路径）。调用方负责确保它存在
/// （[`ensure_download_dir`]）。**红线**：本函数只在传入的 workspace 下拼子目录——传入的
/// workspace 必须是 per-pet 隔离 workspace（companion.rs 的 `{companion_id}/workspace`），
/// **绝不**是用户 Downloads / Home。
pub fn download_dir(workspace: &Path) -> PathBuf {
    workspace.join(DOWNLOAD_SUBDIR)
}

/// best-effort mkdir 隔离下载目录并返回其绝对路径字符串（`setDownloadBehavior.downloadPath` 用）。
///
/// 失败不 panic：返回路径串照常（chrome 落盘时会自己尝试建，或下载失败由事件层观测到）——但
/// 会 `warn`。`downloadPath` CDP 要求绝对路径，故对相对 workspace 先 canonicalize 不可行（目录可能
/// 还不存在），改为：建目录成功后用 `dunce`/原样 lossy 串（与 companion.rs `ensure_workspace_dir`
/// 同范式：`to_string_lossy`）。
pub fn ensure_download_dir(workspace: &Path) -> String {
    let dir = download_dir(workspace);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(error = %e, dir = %dir.display(), "create browser download dir failed");
    }
    dir.to_string_lossy().into_owned()
}

/// **可执行 / 脚本扩展名 denylist（纯逻辑，本模块重点单测）**。
///
/// 命中 = 该文件名以一个**可执行 / 可被解释执行 / 危险**的扩展名结尾（大小写不敏感）。覆盖：
/// - Windows PE / 安装包 / 快捷方式：`exe msi scr com pif cpl dll sys drv`
/// - 脚本宿主：`bat cmd ps1 psm1 ps1xml vbs vbe js jse wsf wsh hta`
/// - 跨平台脚本 / 归档可执行：`sh bash zsh py pyc pl rb jar app reg`
/// - 其它已知滥用面：`lnk gadget inf scf url` 等
///
/// **多重扩展名处理**（裁决⑩点名 `foo.txt.exe`）：判定只看**最后一个** `.` 后的 token——
/// `foo.txt.exe` 命中（最后是 `.exe`），`foo.exe.txt` 不命中（最后是 `.txt`，已被 OS 当文本）。
/// 这与 Windows「按最后扩展名决定关联程序」的真实执行语义一致：危险的是「最终会被当可执行打开」
/// 的尾扩展名。
///
/// 无扩展名（`README` / `noext`）**不命中**——无尾扩展名则 OS 不会自动当可执行启动（且
/// `allowAndName` 会用 GUID 命名，进一步消解）。
pub fn is_executable_denylist(filename: &str) -> bool {
    // 取最后一个 '.' 之后的扩展名（多重扩展名只看尾部，见 doc）。无 '.' → 无扩展名 → 不命中。
    // 注意：用 rsplit_once 而非 Path::extension，避免 `foo.` 之类边角；同时显式处理路径分隔符
    //（传入可能含目录前缀的 suggestedFilename），只取最后一段文件名再取扩展名。
    let name = filename
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(filename)
        .trim();
    let Some((_stem, ext)) = name.rsplit_once('.') else {
        return false; // 无扩展名
    };
    if ext.is_empty() {
        return false; // 形如 "foo."（尾点无扩展名）
    }
    let ext = ext.to_ascii_lowercase();
    DENY_EXTENSIONS.contains(&ext.as_str())
}

/// 可执行 / 脚本 / 危险扩展名集合（小写，无前导点）。比对前把候选 ext `to_ascii_lowercase`
/// 故此处只列小写。来源：Windows「不安全附件」清单 + 常见脚本宿主 + 跨平台可执行。
const DENY_EXTENSIONS: &[&str] = &[
    // ── Windows PE / 安装包 / 系统模块 ──
    "exe", "msi", "msp", "mst", "scr", "com", "pif", "cpl", "dll", "sys", "drv", "ocx",
    // ── Windows 快捷方式 / 配置可被滥用执行 ──
    "lnk", "scf", "inf", "reg", "gadget", "url",
    // ── 脚本宿主（cmd / WSH / PowerShell / HTA）──
    "bat", "cmd", "ps1", "psm1", "psd1", "ps1xml", "vbs", "vbe", "js", "jse", "wsf", "wsh",
    "hta", "msh", "msh1", "msh2", "mshxml",
    // ── 跨平台脚本 / 解释器 ──
    "sh", "bash", "zsh", "ksh", "csh", "py", "pyc", "pyo", "pyw", "pl", "rb", "php",
    // ── 归档可执行 / 应用包 ──
    "jar", "app", "apk", "dmg", "pkg", "deb", "rpm", "appimage", "run", "bin", "elf", "out",
];

/// **可执行下载红线拒绝判定（裁决⑩）**：命中 [`is_executable_denylist`] → `Err(Blocked)`。
///
/// E4 只提供**判定逻辑**（纯函数，返 [`BrowserError::Blocked`]）；**实际 enforcement 接线**（在
/// `BrowserTool::execute` 把它接到 yolo/companion 也不旁路的独立 fail-closed 门上）由 **F1** 做。
///
/// 红线语义（DESIGN §16「会话红线（yolo/companion 无 UI 也 fail-closed 拒绝）」含「可执行下载」）：
/// 一旦命中，**无论 session_mode（yolo/companion 也罢）都拒**——这正是「不靠被旁路的 approval pipeline
/// 审批闸、靠独立门」的体现。故本函数**不吃 session_mode**：命中即拒，无放行参数。
///
/// `Ok(())` = 文件名安全（非可执行），可放行下载。
///
/// TODO(E4->F1-enforce-download-redline): F1 在 facade `do_download`/下载触发的动作分支调用本判定，
/// 命中即 hard-deny 返回（不经 approval pipeline）。E4 此处仅提供判定 + 单测。
pub fn reject_executable_download(filename: &str) -> Result<(), BrowserError> {
    if is_executable_denylist(filename) {
        return Err(BrowserError::Blocked {
            reason: format!(
                "executable/script download blocked (red-line, denied even under yolo/companion): {filename}"
            ),
        });
    }
    Ok(())
}

/// **Magic-bytes content sniffing** — detect executable/script content by inspecting the first
/// bytes of a payload. Only the **first ~16 bytes** are needed (all signatures fit within that).
///
/// Detected signatures:
/// - **ELF** (Linux/BSD executable): `7F 45 4C 46` (`.ELF`)
/// - **Mach-O** (macOS executable, 4 variants):
///   - 32-bit big-endian: `FE ED FA CE`
///   - 32-bit little-endian: `CE FA ED FE`
///   - 64-bit big-endian: `FE ED FA CF`
///   - 64-bit little-endian: `CF FA ED FE`
/// - **Mach-O Universal/Fat binary**: `CA FE BA BE`
/// - **PE/DOS** (Windows executable): `4D 5A` (`MZ`)
/// - **Shell script (shebang)**: `23 21` (`#!`)
///
/// Returns `true` if the bytes match any known executable/script magic.
pub fn sniff_is_executable(bytes: &[u8]) -> bool {
    // Need at least 2 bytes for the shortest signature (#! / MZ).
    if bytes.len() < 2 {
        return false;
    }

    // PE/DOS — "MZ"
    if bytes[0] == 0x4D && bytes[1] == 0x5A {
        return true;
    }
    // Shell shebang — "#!"
    if bytes[0] == 0x23 && bytes[1] == 0x21 {
        return true;
    }

    if bytes.len() >= 4 {
        let magic4 = [bytes[0], bytes[1], bytes[2], bytes[3]];
        match magic4 {
            // ELF
            [0x7F, 0x45, 0x4C, 0x46] => return true,
            // Mach-O 32-bit big-endian
            [0xFE, 0xED, 0xFA, 0xCE] => return true,
            // Mach-O 32-bit little-endian
            [0xCE, 0xFA, 0xED, 0xFE] => return true,
            // Mach-O 64-bit big-endian
            [0xFE, 0xED, 0xFA, 0xCF] => return true,
            // Mach-O 64-bit little-endian
            [0xCF, 0xFA, 0xED, 0xFE] => return true,
            // Mach-O Universal/Fat binary
            [0xCA, 0xFE, 0xBA, 0xBE] => return true,
            _ => {}
        }
    }

    false
}

/// Check whether a `data:` URL contains executable content by **decoding and sniffing** its
/// payload's magic bytes.
///
/// - If `url` does not start with `data:` → returns `false` (this check is data:-specific;
///   network downloads are covered by the filename denylist + landed-file paths).
/// - Parses the data: URL structure: `data:[<mediatype>][;base64],<data>`.
/// - Decodes the payload: base64 if `;base64` is present, otherwise percent-decodes.
/// - Sniffs only the **first 16 decoded bytes** (all magic signatures fit within that) to avoid
///   fully decoding potentially huge payloads.
/// - Returns `false` on any parse/decode error — this is an **additive** block; a malformed
///   `data:` URL isn't a provable executable, so we don't false-block (the filename denylist +
///   other layers still apply).
pub fn data_url_is_executable(url: &str) -> bool {
    // Only applies to data: URLs.
    let data_content = match url.strip_prefix("data:") {
        Some(content) => content,
        None => return false,
    };

    // Split on the first comma — everything before is the media type + parameters,
    // everything after is the payload.
    let Some((header, payload)) = data_content.split_once(',') else {
        return false; // Malformed: no comma → can't parse → not provably executable.
    };

    let is_base64 = header
        .split(';')
        .any(|part| part.trim().eq_ignore_ascii_case("base64"));

    // Decode just enough bytes for sniffing (first ~16 decoded bytes).
    // For base64: 16 decoded bytes = ceil(16*4/3) = 22 base64 chars (plus possible padding).
    // We take the first 24 base64 chars to be safe, decode them.
    let decoded_prefix: Vec<u8> = if is_base64 {
        // Take only the first 24 chars of the payload (produces ≥16 decoded bytes).
        let prefix = if payload.len() > 24 {
            &payload[..24]
        } else {
            payload
        };
        // base64 decode — use the forgiving decoder (STANDARD handles padding).
        match base64::engine::general_purpose::STANDARD.decode(prefix) {
            Ok(bytes) => bytes,
            // Try without padding (some data: URLs omit trailing '=').
            Err(_) => {
                // Pad to a multiple of 4 and retry.
                let pad_len = (4 - prefix.len() % 4) % 4;
                let padded = format!("{}{}", prefix, &"===="[..pad_len]);
                match base64::engine::general_purpose::STANDARD.decode(&padded) {
                    Ok(bytes) => bytes,
                    Err(_) => return false, // Can't decode → not provably executable.
                }
            }
        }
    } else {
        // Percent-decode the first 16 bytes worth.
        // Take a generous prefix of the raw text (percent-encoded bytes are 3 chars each,
        // so 48 chars can produce at least 16 decoded bytes).
        let prefix = if payload.len() > 48 {
            &payload[..48]
        } else {
            payload
        };
        percent_decode_bytes(prefix)
    };

    if decoded_prefix.is_empty() {
        return false;
    }

    sniff_is_executable(&decoded_prefix)
}

/// Simple percent-decoding that returns raw bytes. Only decodes `%XX` sequences;
/// other bytes pass through as-is. Returns empty Vec on any malformed sequence.
fn percent_decode_bytes(input: &str) -> Vec<u8> {
    let mut result = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                // Malformed — return what we have so far (best-effort).
                break;
            }
            let hi = hex_val(bytes[i + 1]);
            let lo = hex_val(bytes[i + 2]);
            match (hi, lo) {
                (Some(h), Some(l)) => {
                    result.push((h << 4) | l);
                    i += 3;
                }
                _ => {
                    // Malformed hex — pass through the '%' as-is.
                    result.push(bytes[i]);
                    i += 1;
                }
            }
        } else {
            result.push(bytes[i]);
            i += 1;
        }
    }
    result
}

/// Convert an ASCII hex character to its numeric value (0–15).
fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// **Win MOTW**：给下载文件写 `Zone.Identifier` ADS（标记来自 Internet，触发 SmartScreen /
/// Office 受保护视图）。
///
/// NTFS ADS 写法：对路径 `C:\dir\file.pdf` 打开 `C:\dir\file.pdf:Zone.Identifier` 这个备用数据流
/// （Windows 文件系统 API 直接支持 `path:streamname` 语法），写入 [`MOTW_ZONE_INTERNET`]。
///
/// **仅 NTFS 有效**：FAT32/exFAT 无 ADS——写会失败（`io::Error`），但本函数 best-effort 调用方
/// 应吞错（MOTW 是纵深防御附加层，缺失不致命）。
#[cfg(windows)]
pub fn write_motw(path: &Path) -> std::io::Result<()> {
    // ADS 语法：在文件路径后接 `:Zone.Identifier`。用 display() 得到平台原生路径串再拼流名。
    let ads_path = format!("{}:Zone.Identifier", path.display());
    std::fs::write(ads_path, MOTW_ZONE_INTERNET)
}

/// **macOS MOTW 等价**：给下载文件设 `com.apple.quarantine` 扩展属性（Gatekeeper 在打开
/// 可执行/归档类时校验）。
///
/// 经 CDP（`setDownloadBehavior allowAndName`）控制的下载，chrome **自身**也会经 `LSFileQuarantine`
/// 落 quarantine（agent=`Chrome`，形态更全：含源 URL + 事件 UUID 链 `LSQuarantineEvent`）——但**并非
/// 所有源都落**（实测 `data:`/`blob:`/部分 `file:` 源时缺失），且其写入与本回调（`downloadProgress
/// completed`）**异步竞争**、先后不定。故此处**兜底补设**，与 Windows 分支显式写 `Zone.Identifier`
/// 对称（纵深防御，不赌浏览器行为）。
///
/// **关键：用 `XATTR_CREATE`（仅当不存在才设），不覆盖 chrome 原生 quarantine**——chrome 已设则我们
/// 的 barebones 值（无源 URL/UUID）反而是降级，故让 chrome 的更全形态优先；chrome 未设时我们填空。
/// 已存在（chrome 或前次已落）→ `EEXIST`，视作**成功**（安全目标=文件已被 quarantine 已达成）。
/// 值格式照搬系统真实下载形态 `<flags>;<hex epoch secs>;<agent>;<event-uuid>`：flags `0081` = 来自
/// web、用户尚未放行（实测系统 Edge/Safari/Chrome 下载即此值），agent 用 `NomiFun`，event-uuid 留空。
///
/// best-effort：失败返 `io::Error` 由调用方吞日志（MOTW 是纵深防御附加层，缺失不致命；
/// 可执行下载的硬防线是 [`reject_executable_download`] 的 denylist，不靠 quarantine）。
#[cfg(target_os = "macos")]
pub fn write_motw(path: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // <flags>;<hex epoch>;<agent>;<event-uuid>（uuid 留空，对齐系统真实下载形态）。
    let value = format!("0081;{secs:x};NomiFun;");

    let c_path = CString::new(path.as_os_str().as_bytes())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let c_name = CString::new("com.apple.quarantine").expect("static name has no interior NUL");

    // SAFETY: setxattr 只按 (path, name, value, size) 读我们提供的、本帧栈上存活的 C 串/字节
    // 缓冲；position=0（非资源 fork 偏移）、options=XATTR_CREATE（仅当 quarantine 不存在才设，
    // 不覆盖 chrome 原生更全的 quarantine——见上 doc）。返回 0 成功，-1 设 errno。
    let ret = unsafe {
        libc::setxattr(
            c_path.as_ptr(),
            c_name.as_ptr(),
            value.as_ptr() as *const libc::c_void,
            value.len(),
            0,
            libc::XATTR_CREATE,
        )
    };
    if ret != 0 {
        let err = std::io::Error::last_os_error();
        // EEXIST = chrome（或前次）已落 quarantine → 文件已被标记,安全目标已达成,视作成功（不降级覆盖）。
        if err.raw_os_error() == Some(libc::EEXIST) {
            return Ok(());
        }
        return Err(err);
    }
    Ok(())
}

/// **Linux/其它 Unix**：无内核级 MOTW/quarantine 等价机制（freedesktop `metadata::trust`
/// 提案未普及），保持空实现——下载隔离靠落 per-pet workspace + 可执行 denylist + 不自动打开。
///
/// TODO(verify-linux): 登记在 `docs/superpowers/specs/browser-use/PLATFORM-VERIFICATION.md`。
#[cfg(all(not(windows), not(target_os = "macos")))]
pub fn write_motw(_path: &Path) -> std::io::Result<()> {
    // linux/其它：无 ADS、无 quarantine 等价，no-op。
    Ok(())
}

/// Root directory holding one exclusive staging subdirectory per exact Host.
///
/// Chromium's `allowAndName` landing directory must never be shared across
/// Hosts: a shared directory plus an mtime scanner is a cross-Host data
/// destruction source, and completed outputs must never live next to
/// in-flight artifacts. Each Host derives its own child from its trusted
/// root-process identity ([`host_staging_dir_name`]).
pub(crate) fn download_staging_root(base: &Path) -> PathBuf {
    base.join("download-staging")
}

/// Exclusive staging directory name for one exact Host, derived from the
/// trusted root Chromium process identity (pid + platform start key). The
/// name is only ever produced from values captured while the launched child
/// handle was owned, so it cannot be forged by page-controlled data.
pub(crate) fn host_staging_dir_name(pid: u32, platform_start_key: u64) -> String {
    format!("host-{pid}-{platform_start_key}")
}

/// Parses a directory name produced by [`host_staging_dir_name`]. Anything
/// that does not match exactly is not ours to touch.
pub(crate) fn parse_host_staging_dir_name(name: &str) -> Option<(u32, u64)> {
    let rest = name.strip_prefix("host-")?;
    let (pid, start_key) = rest.split_once('-')?;
    if pid.is_empty()
        || start_key.is_empty()
        || !pid.bytes().all(|byte| byte.is_ascii_digit())
        || !start_key.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    Some((pid.parse().ok()?, start_key.parse().ok()?))
}

/// Upper bound of orphan staging directories examined per Host launch. The
/// sweep is a crash-recovery safety net, not a general garbage collector;
/// repeated launches make progress across the whole root.
const MAX_ORPHAN_STAGING_SCAN_DIRS: usize = 64;

/// Startup safety net for per-Host staging directories whose owning process
/// died without running its durable cleanup (hard kill, power loss).
///
/// Deletion requires exact-ownership proof per directory: the recorded
/// (pid, platform start key) pair parsed from the directory name must be
/// positively proven dead — either no process runs under that PID, or the
/// live process's start key differs (PID recycled). A probe error is not
/// proof and leaves the directory alone, as does any live identity match
/// (which can be a sibling application instance's healthy Host).
pub(crate) fn sweep_orphan_host_staging_dirs(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.take(MAX_ORPHAN_STAGING_SCAN_DIRS) {
        let Ok(entry) = entry else {
            continue;
        };
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some((pid, recorded_start_key)) = parse_host_staging_dir_name(name) else {
            continue;
        };
        let proven_dead = match nomi_process_runtime::probe_process_identity(pid) {
            Ok(None) => true,
            Ok(Some(live)) => live.platform_start_key != recorded_start_key,
            Err(_) => false,
        };
        if !proven_dead {
            continue;
        }
        if let Err(error) = std::fs::remove_dir_all(entry.path()) {
            tracing::debug!(
                dir = %entry.path().display(),
                %error,
                "orphan host staging directory removal failed; a later launch retries"
            );
        }
    }
}

/// Prefix of the same-volume temp used by [`publish_task_output`].
///
/// The temp lives in the *destination* directory (a rename target must share
/// the volume), which is also where `act_download`'s landing detector and
/// callers poll for the finished artifact. It therefore carries a
/// `.crdownload` suffix as well as this prefix: every existing scanner
/// already skips `.crdownload`/`.tmp`, so an in-flight publication can never
/// be mistaken for a completed download.
const TASK_OUTPUT_TEMP_PREFIX: &str = ".nomifun-publish-";

/// Payload for a two-phase task output publication.
pub(crate) enum TaskOutputPayload<'a> {    /// A staged download artifact to move (or cross-volume copy) into place.
    StagedFile(&'a Path),
    /// In-memory bytes (e.g. a rendered PDF) to write.
    Bytes(&'a [u8]),
}

/// Failure detail of [`publish_task_output`].
#[derive(Debug)]
pub(crate) struct TaskOutputPublishError {
    pub(crate) message: String,
    /// `true` when the prepared task charge was permanently committed even
    /// though publication failed (the residual artifact could not be
    /// deleted). `false` means no lasting charge: dropping the reservation
    /// releases its active accounting and no artifact remains.
    pub(crate) charged: bool,
    /// A leftover file the caller owns deletion-retry debt for.
    pub(crate) residual: Option<PathBuf>,
}

impl TaskOutputPublishError {
    fn uncharged(message: String) -> Self {
        Self {
            message,
            charged: false,
            residual: None,
        }
    }
}

/// Two-phase, compensating publication of one task download/PDF output.
///
/// Order of operations (all synchronous; no await between publication and
/// the permanent charge):
/// 1. `prepare_complete(actual_bytes)` reserves the final task-lifetime
///    byte/file charge — publication can never produce an unbilled artifact.
/// 2. The payload is materialized in a unique same-volume temp file
///    (`create_new`, never clobbering) inside the destination directory.
/// 3. The temp is atomically published to its final name without
///    overwriting an existing file.
/// 4. `finalize_complete()` makes the charge permanent.
///
/// Compensation: any failure before step 4 deletes the temp and leaves the
/// charge unprepared-for-commit (dropping the reservation releases the
/// active accounting). If that deletion itself fails (Windows lock/AV), the
/// charge is committed anyway — an artifact may exist on disk, so the task
/// must keep paying for it; silence about an unbilled product is never an
/// option — and the temp path is returned as retained cleanup debt.
pub(crate) fn publish_task_output(
    reservation: &dyn crate::host::TaskDownloadReservation,
    actual_bytes: u64,
    payload: TaskOutputPayload<'_>,
    destination_dir: &Path,
    preferred_name: &str,
    unique_hint: &str,
) -> Result<PathBuf, TaskOutputPublishError> {
    if let Err(error) = reservation.prepare_complete(actual_bytes) {
        return Err(TaskOutputPublishError::uncharged(format!(
            "task output charge preparation denied: {error}"
        )));
    }
    if let Err(error) = std::fs::create_dir_all(destination_dir) {
        return Err(TaskOutputPublishError::uncharged(format!(
            "task output directory is unavailable: {error}"
        )));
    }
    let temp_path = destination_dir.join(format!("{TASK_OUTPUT_TEMP_PREFIX}{unique_hint}.crdownload"));
    // A stale temp can only be our own crashed prior attempt: the name is
    // derived from a Host-unique GUID/nonce, and this directory belongs to
    // the task workspace.
    let _ = std::fs::remove_file(&temp_path);
    if let Err(error) = materialize_temp_output(&payload, &temp_path) {
        let message = format!(
            "task output staging into {} failed: {error}",
            temp_path.display()
        );
        return Err(rollback_unpublished_temp(reservation, temp_path, message));
    }
    let published = publish_temp_no_clobber(
        &temp_path,
        destination_dir,
        preferred_name,
        unique_hint,
    );
    match published {
        Ok(destination) => {
            // Publication succeeded; the charge becomes permanent with no
            // intervening await point.
            reservation.finalize_complete();
            if !matches!(payload, TaskOutputPayload::StagedFile(_)) {
                // Bytes were written directly into the temp which was then
                // linked/renamed; nothing else to reconcile.
            }
            cleanup_published_temp(&temp_path);
            Ok(destination)
        }
        Err(error) => {
            let message = format!(
                "task output publication into {} failed: {error}",
                destination_dir.display()
            );
            Err(rollback_unpublished_temp(reservation, temp_path, message))
        }
    }
}

/// Writes/moves the payload into the exclusive temp path.
fn materialize_temp_output(
    payload: &TaskOutputPayload<'_>,
    temp_path: &Path,
) -> std::io::Result<()> {
    match payload {
        TaskOutputPayload::Bytes(bytes) => {
            use std::io::Write as _;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(temp_path)?;
            file.write_all(bytes)?;
            file.sync_all()
        }
        TaskOutputPayload::StagedFile(source) => {
            match std::fs::rename(source, temp_path) {
                Ok(()) => Ok(()),
                Err(_) => {
                    // Workspaces can live on another volume. Stream into an
                    // exclusively created temp instead of `fs::copy`, which
                    // would truncate an unexpectedly existing path.
                    use std::io::Write as _;
                    let mut reader = std::fs::File::open(source)?;
                    let mut writer = std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(temp_path)?;
                    std::io::copy(&mut reader, &mut writer)?;
                    writer.flush()?;
                    writer.sync_all()?;
                    drop(writer);
                    if let Err(error) = std::fs::remove_file(source) {
                        // The published copy is authoritative; the staging
                        // leftover stays on the caller's staging cleanup
                        // path, which retries and is finally reconciled at
                        // exact host stop.
                        tracing::debug!(
                            file = %source.display(),
                            %error,
                            "cross-volume staged source removal deferred to staging cleanup"
                        );
                    }
                    Ok(())
                }
            }
        }
    }
}

/// Atomically publishes the temp to a final name without clobbering. A hard
/// link fails with `AlreadyExists` when the destination is taken, giving an
/// atomic no-clobber publish on every mainstream filesystem; filesystems
/// without hard links fall back to an existence-guarded rename.
fn publish_temp_no_clobber(
    temp_path: &Path,
    destination_dir: &Path,
    preferred_name: &str,
    unique_hint: &str,
) -> std::io::Result<PathBuf> {
    let mut last_error = None;
    for candidate in [
        preferred_name.to_string(),
        format!("{unique_hint}-{preferred_name}"),
    ] {
        let destination = destination_dir.join(&candidate);
        match std::fs::hard_link(temp_path, &destination) {
            Ok(()) => return Ok(destination),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                last_error = Some(error);
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::Unsupported | std::io::ErrorKind::InvalidInput
                ) =>
            {
                // No hard-link support: fall back to a rename guarded by an
                // existence check. The TOCTOU window is bounded to this
                // task-owned workspace directory.
                if destination.exists() {
                    last_error = Some(std::io::Error::new(
                        std::io::ErrorKind::AlreadyExists,
                        "destination already exists",
                    ));
                    continue;
                }
                return std::fs::rename(temp_path, &destination).map(|()| destination);
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_error.unwrap_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "no non-clobbering destination name was available",
        )
    }))
}

/// Removes the temp after a successful hard-link publish. Failure is benign:
/// the extra directory entry references the same already-charged inode.
fn cleanup_published_temp(temp_path: &Path) {
    if let Err(error) = std::fs::remove_file(temp_path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::debug!(
            file = %temp_path.display(),
            %error,
            "published task output temp link removal deferred"
        );
    }
}

/// Compensates a failed publication after the charge was prepared. Removing
/// the temp lets the caller drop the reservation and release the active
/// charge; an undeletable temp is finalized (kept billed) instead of leaving
/// an unbilled artifact behind.
fn rollback_unpublished_temp(
    reservation: &dyn crate::host::TaskDownloadReservation,
    temp_path: PathBuf,
    message: String,
) -> TaskOutputPublishError {
    match std::fs::remove_file(&temp_path) {
        Ok(()) => TaskOutputPublishError::uncharged(message),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            TaskOutputPublishError::uncharged(message)
        }
        Err(error) => {
            reservation.finalize_complete();
            TaskOutputPublishError {
                message: format!(
                    "{message}; residual artifact {} could not be deleted ({error}) and its task charge was committed",
                    temp_path.display()
                ),
                charged: true,
                residual: Some(temp_path),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── [纯逻辑] 可执行 denylist ───────────────────────────────────────────────

    #[test]
    fn denylist_hits_common_executables_and_scripts() {
        for name in [
            "setup.exe",
            "installer.msi",
            "payload.bat",
            "payload.cmd",
            "script.ps1",
            "module.psm1",
            "screensaver.scr",
            "legacy.com",
            "macro.vbs",
            "loader.js",
            "applet.jar",
            "lib.dll",
            "tool.sh",
            "Some.App", // .app（mac 应用包）大小写后命中
        ] {
            assert!(
                is_executable_denylist(name),
                "expected denylist HIT for {name:?}"
            );
        }
    }

    #[test]
    fn denylist_misses_benign_documents() {
        for name in [
            "report.txt",
            "invoice.pdf",
            "photo.png",
            "data.csv",
            "archive.zip", // zip 本身非可执行（不自动运行；解压才是上层的事）
            "music.mp3",
            "page.html",
            "config.json",
            "sheet.xlsx",
        ] {
            assert!(
                !is_executable_denylist(name),
                "expected denylist MISS for {name:?}"
            );
        }
    }

    #[test]
    fn denylist_handles_double_extension_attack() {
        // 裁决⑩点名：foo.txt.exe（伪装成文本的可执行）——尾扩展名 .exe → 命中。
        assert!(is_executable_denylist("foo.txt.exe"));
        assert!(is_executable_denylist("invoice.pdf.scr"));
        assert!(is_executable_denylist("photo.png.bat"));
        // 反向：foo.exe.txt 的尾扩展名是 .txt（OS 会当文本）→ 不命中（与真实执行语义一致）。
        assert!(!is_executable_denylist("foo.exe.txt"));
        assert!(!is_executable_denylist("malware.scr.pdf"));
    }

    #[test]
    fn denylist_is_case_insensitive() {
        assert!(is_executable_denylist("SETUP.EXE"));
        assert!(is_executable_denylist("Script.Ps1"));
        assert!(is_executable_denylist("PAYLOAD.BaT"));
        assert!(is_executable_denylist("file.MSI"));
    }

    #[test]
    fn denylist_misses_no_extension() {
        // 无扩展名：OS 不自动当可执行启动（且 allowAndName 用 GUID 命名）→ 不命中。
        assert!(!is_executable_denylist("README"));
        assert!(!is_executable_denylist("noext"));
        assert!(!is_executable_denylist("Makefile"));
        // 尾点无扩展名（"foo."）→ 不命中。
        assert!(!is_executable_denylist("foo."));
    }

    #[test]
    fn denylist_strips_directory_prefix_in_suggested_filename() {
        // suggestedFilename 偶含路径前缀；只取最后一段文件名再判扩展名。
        assert!(is_executable_denylist("subdir/setup.exe"));
        assert!(is_executable_denylist("a\\b\\payload.bat"));
        assert!(!is_executable_denylist("setup.exe/report.txt")); // 最后一段是 report.txt
    }

    #[test]
    fn denylist_trims_whitespace() {
        assert!(is_executable_denylist("  setup.exe  "));
        assert!(!is_executable_denylist("  report.txt  "));
    }

    // ── [纯逻辑] 红线拒绝判定 ───────────────────────────────────────────────────

    #[test]
    fn reject_executable_download_blocks_executables() {
        let e = reject_executable_download("setup.exe").unwrap_err();
        assert!(
            matches!(e, BrowserError::Blocked { .. }),
            "executable download must be Blocked, got {e:?}"
        );
        // 拒绝文案含红线语义关键词（yolo/companion 也拒）+ 文件名（便于审计）。
        let msg = format!("{e}");
        assert!(msg.contains("setup.exe"), "blocked reason should name the file: {msg}");
        assert!(
            msg.contains("red-line") || msg.contains("yolo"),
            "blocked reason should signal red-line semantics: {msg}"
        );
    }

    #[test]
    fn reject_executable_download_allows_benign() {
        assert!(reject_executable_download("invoice.pdf").is_ok());
        assert!(reject_executable_download("data.csv").is_ok());
        assert!(reject_executable_download("foo.txt.exe").is_err()); // 多扩展名仍拒
    }

    // ── [纯逻辑] MOTW 内容格式 ──────────────────────────────────────────────────

    #[test]
    fn motw_content_is_internet_zone_with_crlf() {
        // ZoneId=3 = Internet 区（触发 SmartScreen / Office 受保护视图）。
        assert!(MOTW_ZONE_INTERNET.contains("[ZoneTransfer]"));
        assert!(MOTW_ZONE_INTERNET.contains("ZoneId=3"));
        // Windows ADS 约定 CRLF 行尾。
        assert!(MOTW_ZONE_INTERNET.contains("\r\n"));
        // 不含其它 ZoneId（避免误标为本地/可信区）。
        assert!(!MOTW_ZONE_INTERNET.contains("ZoneId=0"));
        assert!(!MOTW_ZONE_INTERNET.contains("ZoneId=1"));
        assert!(!MOTW_ZONE_INTERNET.contains("ZoneId=2"));
    }

    // ── [纯逻辑] 隔离下载目录路径 ───────────────────────────────────────────────

    #[test]
    fn download_dir_is_downloads_subdir_of_workspace() {
        let ws = Path::new("/some/companion/workspace");
        let dl = download_dir(ws);
        assert!(dl.ends_with("downloads"));
        assert!(dl.starts_with("/some/companion/workspace"));
        // 绝不是用户 Downloads / Home：只在传入 workspace 下拼子目录。
        assert_eq!(dl, ws.join("downloads"));
    }

    // ── [纯逻辑] data:URL 可执行内容嗅探（SD-3）──────────────────────────────────

    #[test]
    fn sniff_is_executable_detects_elf_magic() {
        // ELF: 7F 45 4C 46 + some padding
        let elf_bytes = [0x7F, 0x45, 0x4C, 0x46, 0x02, 0x01, 0x01, 0x00];
        assert!(sniff_is_executable(&elf_bytes));
    }

    #[test]
    fn sniff_is_executable_detects_pe_mz_magic() {
        // PE/DOS: "MZ" = 4D 5A
        let pe_bytes = [0x4D, 0x5A, 0x90, 0x00, 0x03, 0x00, 0x00, 0x00];
        assert!(sniff_is_executable(&pe_bytes));
    }

    #[test]
    fn sniff_is_executable_detects_macho_variants() {
        // Mach-O 64-bit little-endian (most common on modern macOS)
        assert!(sniff_is_executable(&[0xCF, 0xFA, 0xED, 0xFE, 0x07, 0x00, 0x00, 0x01]));
        // Mach-O 32-bit big-endian
        assert!(sniff_is_executable(&[0xFE, 0xED, 0xFA, 0xCE, 0x00, 0x00, 0x00, 0x02]));
        // Mach-O 64-bit big-endian
        assert!(sniff_is_executable(&[0xFE, 0xED, 0xFA, 0xCF, 0x00, 0x00, 0x00, 0x02]));
        // Mach-O 32-bit little-endian
        assert!(sniff_is_executable(&[0xCE, 0xFA, 0xED, 0xFE, 0x07, 0x00, 0x00, 0x01]));
        // Universal/Fat binary
        assert!(sniff_is_executable(&[0xCA, 0xFE, 0xBA, 0xBE, 0x00, 0x00, 0x00, 0x02]));
    }

    #[test]
    fn sniff_is_executable_detects_shebang() {
        // Shell script shebang: "#!"
        assert!(sniff_is_executable(b"#!/bin/bash\n"));
        assert!(sniff_is_executable(b"#!/usr/bin/env python3\n"));
        assert!(sniff_is_executable(b"#!"));
    }

    #[test]
    fn sniff_is_executable_returns_false_for_benign_content() {
        // Plain text
        assert!(!sniff_is_executable(b"hello world"));
        // PDF
        assert!(!sniff_is_executable(b"%PDF-1.4"));
        // PNG
        assert!(!sniff_is_executable(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]));
        // JPEG
        assert!(!sniff_is_executable(&[0xFF, 0xD8, 0xFF, 0xE0]));
        // Empty
        assert!(!sniff_is_executable(&[]));
        // Single byte
        assert!(!sniff_is_executable(&[0x7F]));
    }

    #[test]
    fn data_url_executable_without_extension_is_caught() {
        use base64::Engine as _;

        // ── ELF binary as base64 data: URL (no filename extension in download) ──
        let elf_magic: &[u8] = &[0x7F, 0x45, 0x4C, 0x46, 0x02, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        let elf_b64 = base64::engine::general_purpose::STANDARD.encode(elf_magic);
        let elf_url = format!("data:application/octet-stream;base64,{elf_b64}");
        assert!(
            data_url_is_executable(&elf_url),
            "ELF binary in data: URL must be detected as executable"
        );

        // ── PE/MZ binary as base64 data: URL ──
        let pe_magic: &[u8] = &[0x4D, 0x5A, 0x90, 0x00, 0x03, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0x00, 0x00];
        let pe_b64 = base64::engine::general_purpose::STANDARD.encode(pe_magic);
        let pe_url = format!("data:application/octet-stream;base64,{pe_b64}");
        assert!(
            data_url_is_executable(&pe_url),
            "PE/MZ binary in data: URL must be detected as executable"
        );

        // ── Shebang script as base64 data: URL ──
        let script = b"#!/bin/bash\nrm -rf /\n";
        let script_b64 = base64::engine::general_purpose::STANDARD.encode(script);
        let script_url = format!("data:text/plain;base64,{script_b64}");
        assert!(
            data_url_is_executable(&script_url),
            "shebang script in data: URL must be detected as executable"
        );

        // ── Benign text content → must NOT be flagged ──
        let benign = b"hello world, this is safe text content";
        let benign_b64 = base64::engine::general_purpose::STANDARD.encode(benign);
        let benign_url = format!("data:text/plain;base64,{benign_b64}");
        assert!(
            !data_url_is_executable(&benign_url),
            "benign text in data: URL must NOT be flagged as executable"
        );

        // ── Non-data: URL → always false ──
        assert!(!data_url_is_executable("https://example.com/malware.exe"));
        assert!(!data_url_is_executable("file:///tmp/something"));
        assert!(!data_url_is_executable(""));
    }

    #[test]
    fn data_url_executable_percent_encoded() {
        // ELF magic as percent-encoded (no ;base64 flag)
        // 7F 45 4C 46 02 01 01 00 → %7F%45%4C%46%02%01%01%00
        let elf_pct = "data:application/octet-stream,%7F%45%4C%46%02%01%01%00%00%00%00%00%00%00%00%00";
        assert!(
            data_url_is_executable(elf_pct),
            "percent-encoded ELF in data: URL must be detected"
        );

        // Benign percent-encoded text
        let benign_pct = "data:text/plain,hello%20world";
        assert!(
            !data_url_is_executable(benign_pct),
            "benign percent-encoded text must NOT be flagged"
        );
    }

    #[test]
    fn data_url_malformed_returns_false() {
        // No comma → malformed
        assert!(!data_url_is_executable("data:application/octet-stream;base64"));
        // Invalid base64 → can't decode → false
        assert!(!data_url_is_executable("data:;base64,!!!!not-valid-base64!!!!"));
        // Empty payload
        assert!(!data_url_is_executable("data:text/plain,"));
        assert!(!data_url_is_executable("data:;base64,"));
    }

    // ── [纯逻辑+真 FS] MOTW 写入（仅 windows 真写 ADS；其它平台空实现也跑通）──────

    #[cfg(windows)]
    #[test]
    fn write_motw_creates_zone_identifier_ads_on_ntfs() {
        // 在临时目录建一个真文件，写 MOTW，再读回那个 ADS 流校验内容。
        // 注：临时目录一般在 NTFS（C: 卷）；FAT32/exFAT 无 ADS 会失败——本机 Windows 临时目录是 NTFS。
        let dir = std::env::temp_dir().join("nomifun-motw-test");
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join(format!("dl-{}.pdf", std::process::id()));
        std::fs::write(&f, b"%PDF-1.4 fake").unwrap();

        write_motw(&f).expect("write_motw should succeed on NTFS temp dir");

        // 读回 Zone.Identifier ADS 流，断言含 ZoneId=3。
        let ads = format!("{}:Zone.Identifier", f.display());
        let got = std::fs::read_to_string(&ads).expect("Zone.Identifier ADS should exist");
        assert!(got.contains("[ZoneTransfer]"), "ADS content: {got:?}");
        assert!(got.contains("ZoneId=3"), "ADS content: {got:?}");

        // 清理（删主文件即连带删其 ADS）。
        let _ = std::fs::remove_file(&f);
    }

    // ── macOS：write_motw 真写 com.apple.quarantine（Gatekeeper 的 MOTW 等价）──────────
    #[cfg(target_os = "macos")]
    #[test]
    fn write_motw_sets_quarantine_xattr_on_macos() {
        use std::os::unix::ffi::OsStrExt;

        let dir = std::env::temp_dir().join("nomifun-motw-test-macos");
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join(format!("dl-{}.pdf", std::process::id()));
        std::fs::write(&f, b"%PDF-1.4 fake").unwrap();

        write_motw(&f).expect("write_motw should set com.apple.quarantine on a real file");

        // 读回 com.apple.quarantine xattr，断言形态 `0081;<hex>;NomiFun;`。
        let c_path = std::ffi::CString::new(f.as_os_str().as_bytes()).unwrap();
        let c_name = std::ffi::CString::new("com.apple.quarantine").unwrap();
        // 先取长度，再读值。
        let len = unsafe {
            libc::getxattr(c_path.as_ptr(), c_name.as_ptr(), std::ptr::null_mut(), 0, 0, 0)
        };
        assert!(len > 0, "com.apple.quarantine should exist (getxattr len={len})");
        let mut buf = vec![0u8; len as usize];
        let got = unsafe {
            libc::getxattr(
                c_path.as_ptr(),
                c_name.as_ptr(),
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
                0,
                0,
            )
        };
        assert_eq!(got, len, "getxattr value read should match length");
        let value = String::from_utf8_lossy(&buf).to_string();
        assert!(value.starts_with("0081;"), "quarantine flags should mark web download: {value:?}");
        assert!(value.contains("NomiFun"), "quarantine agent should be NomiFun: {value:?}");

        let _ = std::fs::remove_file(&f);
    }

    // ── Linux/其它 Unix：write_motw 是 no-op（无 quarantine 等价），任意路径返 Ok ─────────
    #[cfg(all(not(windows), not(target_os = "macos")))]
    #[test]
    fn write_motw_is_noop_on_linux() {
        // linux：空实现，对任意路径返回 Ok（不真写）。
        let p = Path::new("/tmp/whatever-nonexistent.pdf");
        assert!(write_motw(p).is_ok());
    }

    // ── per-Host staging 命名/解析与孤儿清扫 ─────────────────────────────────

    #[test]
    fn host_staging_dir_name_round_trips_and_rejects_foreign_names() {
        let name = host_staging_dir_name(1234, 987_654_321);
        assert_eq!(name, "host-1234-987654321");
        assert_eq!(parse_host_staging_dir_name(&name), Some((1234, 987_654_321)));

        for foreign in [
            "host-",
            "host-12",
            "host-abc-123",
            "host-12-abc",
            "host--123",
            "host-12-",
            "downloads",
            "host-12-34-56x",
            "HOST-12-34",
        ] {
            assert_eq!(
                parse_host_staging_dir_name(foreign),
                None,
                "{foreign:?} must never be treated as an owned staging dir"
            );
        }
        // Extra dash segments parse as part of the start key only when fully
        // numeric; "host-12-34-56" splits at the first dash after the pid, so
        // the remainder "34-56" is non-numeric and rejected.
        assert_eq!(parse_host_staging_dir_name("host-12-34-56"), None);
    }

    #[test]
    fn orphan_staging_sweep_requires_exact_dead_process_proof() {
        let temp = tempfile::tempdir().expect("create orphan sweep root");
        let root = temp.path();
        let own_pid = std::process::id();
        let own_identity = nomi_process_runtime::probe_process_identity(own_pid)
            .expect("probe own process")
            .expect("own process is alive");

        // Live identity match (this very test process) → never touched.
        let live_dir = root.join(host_staging_dir_name(
            own_pid,
            own_identity.platform_start_key,
        ));
        // Same PID, different start key → the recorded instance is proven
        // dead (PID recycle rule) → removed.
        let recycled_dir = root.join(host_staging_dir_name(
            own_pid,
            own_identity.platform_start_key.wrapping_add(1),
        ));
        // A non-matching name is not ours to touch.
        let foreign_dir = root.join("not-a-host-dir");
        for dir in [&live_dir, &recycled_dir, &foreign_dir] {
            std::fs::create_dir_all(dir).expect("create sweep fixture dir");
            std::fs::write(dir.join("leftover.part"), b"x").expect("seed leftover");
        }

        sweep_orphan_host_staging_dirs(root);

        assert!(live_dir.exists(), "a live exact identity is never swept");
        assert!(
            !recycled_dir.exists(),
            "a proven-dead identity's staging dir is removed"
        );
        assert!(foreign_dir.exists(), "unrecognized names are left alone");
    }

    // ── 两阶段发布助手 ───────────────────────────────────────────────────────

    struct PublishProbeReservation {
        deny_prepare: bool,
        prepared: std::sync::Mutex<Option<u64>>,
        finalized: std::sync::atomic::AtomicBool,
    }

    impl PublishProbeReservation {
        fn new(deny_prepare: bool) -> Self {
            Self {
                deny_prepare,
                prepared: std::sync::Mutex::new(None),
                finalized: std::sync::atomic::AtomicBool::new(false),
            }
        }

        fn prepared(&self) -> Option<u64> {
            *self.prepared.lock().unwrap()
        }

        fn finalized(&self) -> bool {
            self.finalized.load(std::sync::atomic::Ordering::Acquire)
        }
    }

    impl crate::host::TaskDownloadReservation for PublishProbeReservation {
        fn update_progress(
            &self,
            _received_bytes: u64,
            _total_bytes: Option<u64>,
        ) -> Result<(), BrowserError> {
            Ok(())
        }

        fn prepare_complete(&self, actual_bytes: u64) -> Result<(), BrowserError> {
            if self.deny_prepare {
                return Err(BrowserError::Blocked {
                    reason: "injected prepare denial".into(),
                });
            }
            self.prepared.lock().unwrap().replace(actual_bytes);
            Ok(())
        }

        fn finalize_complete(&self) {
            self.finalized
                .store(true, std::sync::atomic::Ordering::Release);
        }
    }

    #[test]
    fn publish_task_output_bytes_prepares_publishes_then_finalizes() {
        let temp = tempfile::tempdir().expect("create publish root");
        let dir = temp.path().join("outputs");
        let reservation = PublishProbeReservation::new(false);

        let published = publish_task_output(
            &reservation,
            5,
            TaskOutputPayload::Bytes(b"hello"),
            &dir,
            "page.pdf",
            "nonce-1",
        )
        .expect("publication succeeds");

        assert_eq!(published, dir.join("page.pdf"));
        assert_eq!(std::fs::read(&published).unwrap(), b"hello");
        assert_eq!(reservation.prepared(), Some(5));
        assert!(reservation.finalized());
        assert!(
            !dir.join(format!("{TASK_OUTPUT_TEMP_PREFIX}nonce-1.crdownload")).exists(),
            "the temp is removed after publication"
        );
    }

    #[test]
    fn publish_task_output_prepare_denial_produces_no_artifact() {
        let temp = tempfile::tempdir().expect("create publish root");
        let dir = temp.path().join("outputs");
        let reservation = PublishProbeReservation::new(true);

        let failure = publish_task_output(
            &reservation,
            5,
            TaskOutputPayload::Bytes(b"hello"),
            &dir,
            "page.pdf",
            "nonce-2",
        )
        .expect_err("denied preparation fails the publication");

        assert!(!failure.charged);
        assert!(failure.residual.is_none());
        assert!(!reservation.finalized());
        assert!(!dir.exists() || std::fs::read_dir(&dir).unwrap().next().is_none());
    }

    #[test]
    fn publish_task_output_never_clobbers_and_rolls_back_on_full_collision() {
        let temp = tempfile::tempdir().expect("create publish root");
        let dir = temp.path().join("outputs");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("page.pdf"), b"old").unwrap();

        // First fallback name is free → published there, original intact.
        let reservation = PublishProbeReservation::new(false);
        let published = publish_task_output(
            &reservation,
            3,
            TaskOutputPayload::Bytes(b"new"),
            &dir,
            "page.pdf",
            "nonce-3",
        )
        .expect("fallback name publishes");
        assert_eq!(published, dir.join("nonce-3-page.pdf"));
        assert_eq!(std::fs::read(dir.join("page.pdf")).unwrap(), b"old");

        // Both candidate names taken → rolled back, nothing overwritten,
        // charge not committed, temp removed.
        let reservation = PublishProbeReservation::new(false);
        let failure = publish_task_output(
            &reservation,
            3,
            TaskOutputPayload::Bytes(b"newer"),
            &dir,
            "page.pdf",
            "nonce-3",
        )
        .expect_err("a full collision must fail instead of overwriting");
        assert!(!failure.charged);
        assert!(!reservation.finalized());
        assert_eq!(std::fs::read(dir.join("page.pdf")).unwrap(), b"old");
        assert_eq!(std::fs::read(dir.join("nonce-3-page.pdf")).unwrap(), b"new");
        assert!(
            !dir.join(format!("{TASK_OUTPUT_TEMP_PREFIX}nonce-3.crdownload")).exists()
        );
    }

    #[test]
    fn publish_task_output_undeletable_residual_keeps_the_charge() {
        let temp = tempfile::tempdir().expect("create publish root");
        let dir = temp.path().join("outputs");
        std::fs::create_dir_all(&dir).unwrap();
        // A directory at the temp path defeats both the stale-temp removal
        // and create_new, and the rollback deletion of a "file" fails too —
        // the injected equivalent of a Windows lock/AV hold.
        let blocked_temp = dir.join(format!("{TASK_OUTPUT_TEMP_PREFIX}nonce-4.crdownload"));
        std::fs::create_dir(&blocked_temp).unwrap();

        let reservation = PublishProbeReservation::new(false);
        let failure = publish_task_output(
            &reservation,
            5,
            TaskOutputPayload::Bytes(b"hello"),
            &dir,
            "page.pdf",
            "nonce-4",
        )
        .expect_err("an unmaterializable temp fails the publication");

        assert!(
            failure.charged,
            "an undeletable residual must keep its task charge committed"
        );
        assert_eq!(failure.residual.as_deref(), Some(blocked_temp.as_path()));
        assert!(
            reservation.finalized(),
            "fail-closed: the charge is finalized when the artifact cannot be removed"
        );
    }

    #[test]
    fn publish_task_output_moves_staged_file_and_consumes_source() {
        let temp = tempfile::tempdir().expect("create publish root");
        let staging = temp.path().join("staging");
        let dir = temp.path().join("outputs");
        std::fs::create_dir_all(&staging).unwrap();
        let source = staging.join("guid-move");
        std::fs::write(&source, b"staged-bytes").unwrap();

        let reservation = PublishProbeReservation::new(false);
        let published = publish_task_output(
            &reservation,
            12,
            TaskOutputPayload::StagedFile(&source),
            &dir,
            "moved.bin",
            "guid-move",
        )
        .expect("staged publication succeeds");

        assert_eq!(published, dir.join("moved.bin"));
        assert_eq!(std::fs::read(&published).unwrap(), b"staged-bytes");
        assert!(!source.exists(), "the staged source is consumed");
        assert_eq!(reservation.prepared(), Some(12));
        assert!(reservation.finalized());
    }
}
