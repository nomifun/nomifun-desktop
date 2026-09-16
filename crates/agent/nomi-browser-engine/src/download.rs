//! Pure filename/content checks shared by native user and Agent downloads.
//! Output ownership and publication belong to Browser Platform v2.

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


#[cfg(test)]
mod tests {
    use super::*;
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
}
