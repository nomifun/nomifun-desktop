#![cfg(windows)]
use std::{collections::BTreeSet, fs, path::Path};
use nomi_native_files::{NativeFileError, NativeReadAcquisition, NativeReadRequest, acquire, acquire_source_path,
    acquire_source_with_hooks, acquire_instruction_scope, acquire_instruction_scope_with_access};

#[test]
fn an_unavailable_filesystem_root_preserves_native_error_and_is_never_source_absence() {
    // An unmounted DOS root exercises the same exhausted ancestor walk as a
    // removed UNC share without network configuration or a remote timeout.
    // No drive is mounted, mapped, created or otherwise modified by this test.
    let (root, root_error) = (b'E'..=b'Z').rev().find_map(|letter| {
        let root = std::path::PathBuf::from(format!("{}:\\", letter as char));
        match fs::symlink_metadata(&root) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some((root, error)),
            _ => None,
        }
    }).expect("native fixture requires one genuinely unavailable filesystem root");
    let path = root.join("nomifun-readonly-missing-root").join("AGENTS.md");
    let requests = [
        NativeReadRequest::Source { path: path.to_str().unwrap().into(), max_bytes: 1024 },
        NativeReadRequest::InstructionScope { path: path.to_str().unwrap().into(), recursive: true, path_semantics: Default::default() },
        NativeReadRequest::InstructionScope { path: path.to_str().unwrap().into(), recursive: true, path_semantics: nomi_native_files::NativePathSemantics::Entry },
    ];
    for request in requests {
        let error = acquire(request).unwrap_err().observation();
        assert_eq!(error.code, "RESOURCE_NOT_FOUND", "{error:?}");
        assert_eq!(error.kind.as_deref(), Some("NotFound"));
        assert_eq!(error.raw_os_error, root_error.raw_os_error());
        assert!(error.raw_os_error.is_some(), "the actual native root error must survive projection");
    }
}

#[test]
fn windows_sources_preserve_alias_and_stream_identity_and_exclude_devices() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("source.txt"), b"main").unwrap();
    fs::write(root.path().join("source.txt:private"), b"stream data").unwrap();
    for (name, bytes, expected) in [("source.txt.", b"main".as_slice(), "source.txt"),
        ("source.txt::$DATA", b"main".as_slice(), "source.txt"),
        ("source.txt:private", b"stream data".as_slice(), "source.txt:private")] {
        let acquired = acquire_source_path(&root.path().join(name), 1024).unwrap().unwrap();
        assert_eq!(acquired.bytes, bytes);
        assert!(acquired.canonical_path.ends_with(expected), "{}", acquired.canonical_path);
    }
    for name in ["NUL", "CON", r"\\.\NUL", r"\\?\GLOBALROOT\Device\Null"] {
        let path = if Path::new(name).is_absolute() { name.into() } else { root.path().join(name) };
        assert!(acquire_source_path(&path, 1024).is_err(), "{name}");
    }
}

#[test]
fn mounted_volume_guid_and_globalroot_paths_are_real_filesystem_sources() {
    use std::os::windows::ffi::{OsStrExt as _, OsStringExt as _};
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetVolumeNameForVolumeMountPointW(root: *const u16, volume: *mut u16, length: u32) -> i32;
        fn QueryDosDeviceW(name: *const u16, target: *mut u16, length: u32) -> u32;
    }
    let fixture = tempfile::tempdir().unwrap(); let root = std::path::absolute(fixture.path()).unwrap();
    let drive = root.ancestors().last().unwrap();
    let drive_name: Vec<u16> = drive.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut volume_name = [0u16; 128];
    // SAFETY: both UTF-16 buffers remain live and the output size is exact.
    assert_ne!(unsafe { GetVolumeNameForVolumeMountPointW(drive_name.as_ptr(), volume_name.as_mut_ptr(), volume_name.len() as u32) }, 0);
    let length = volume_name.iter().position(|unit| *unit == 0).unwrap();
    let mut source = std::path::PathBuf::from(std::ffi::OsString::from_wide(&volume_name[..length]));
    source.push(root.strip_prefix(drive).unwrap()); source.push("value.txt");
    fs::write(root.join("value.txt"), b"volume source").unwrap();
    let acquired = acquire_source_path(&source, 1024).unwrap().unwrap();
    assert_eq!(acquired.bytes, b"volume source");
    assert_eq!(Path::new(&acquired.canonical_path), fs::canonicalize(root.join("value.txt")).unwrap());
    let letter = match drive.components().next().unwrap() {
        std::path::Component::Prefix(prefix) => match prefix.kind() { std::path::Prefix::Disk(letter) | std::path::Prefix::VerbatimDisk(letter) => letter, _ => panic!("local disk fixture") },
        _ => panic!("drive prefix"),
    };
    let device_name: Vec<u16> = format!("{}:", letter as char).encode_utf16().chain(Some(0)).collect();
    let mut device = [0_u16; 512];
    assert_ne!(unsafe { QueryDosDeviceW(device_name.as_ptr(), device.as_mut_ptr(), device.len() as u32) }, 0);
    let length = device.iter().position(|unit| *unit == 0).unwrap();
    let mut alias = std::path::PathBuf::from(format!(r"\\?\GLOBALROOT{}\", std::ffi::OsString::from_wide(&device[..length]).to_string_lossy()));
    alias.push(root.strip_prefix(drive).unwrap()); alias.push("value.txt");
    let globalroot = acquire_source_path(&alias, 1024).unwrap().unwrap();
    assert_eq!(globalroot.bytes, b"volume source");
    assert_eq!(globalroot.canonical_path, acquired.canonical_path);
    let dos_alias = std::path::PathBuf::from(format!(r"\\.\{}", root.join("value.txt").display()));
    let dos = acquire_source_path(&dos_alias, 1024).unwrap().unwrap();
    assert_eq!(dos.bytes, b"volume source"); assert_eq!(dos.canonical_path, acquired.canonical_path);
}

#[test]
fn source_cannot_read_a_junction_introduced_after_resolution() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("inside");
    let outside = fixture.path().join("outside");
    fs::create_dir(&root).unwrap(); fs::create_dir(&outside).unwrap();
    fs::write(root.join("value"), b"inside").unwrap();
    fs::write(outside.join("value"), b"outside").unwrap();
    let result = acquire_source_with_hooks(&root.join("value"), 1024, || {
        fs::rename(&root, fixture.path().join("retained")).unwrap();
        junction::create(&outside, &root).unwrap();
    }, || panic!("the redirected source must be rejected before bytes are read"));
    junction::delete(&root).unwrap(); fs::rename(fixture.path().join("retained"), &root).unwrap();
    assert!(matches!(result, Err(NativeFileError::Changed(_))), "{result:?}");
    assert_eq!(fs::read(root.join("value")).unwrap(), b"inside");
    assert_eq!(fs::read(outside.join("value")).unwrap(), b"outside");
}

#[test]
fn native_scope_keeps_hidden_rules_and_never_walks_descendant_junctions() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(fixture.path()).unwrap();
    fs::create_dir_all(root.join("inside/.hidden/deep")).unwrap(); fs::create_dir(root.join("outside")).unwrap();
    fs::write(root.join("inside/AGENTS.md"), b"root rules").unwrap();
    fs::write(root.join("inside/.hidden/deep/agents.override.md"), b"deep rules").unwrap();
    fs::write(root.join("outside/AGENTS.md"), b"outside rules").unwrap();
    junction::create(root.join("outside"), root.join("inside/linked")).unwrap();
    let scope = acquire_instruction_scope(&root.join("inside"), true).unwrap();
    assert_eq!(scope.kind, "directory"); assert!(!scope.complete);
    assert_eq!(scope.incomplete_reasons, BTreeSet::from(["symlink_entry".into()]));
    assert_eq!(scope.directories, BTreeSet::from([root.join("inside").to_str().unwrap().into(), root.join("inside/.hidden/deep").to_str().unwrap().into()]));
    let NativeReadAcquisition::InstructionScope { scope } = acquire(NativeReadRequest::InstructionScope {
        path: root.join("inside/linked").to_str().unwrap().into(), recursive: false, path_semantics: Default::default() }).unwrap() else { panic!("scope result"); };
    assert_eq!(Path::new(&scope.canonical_path), root.join("outside"));
}

#[test]
fn entry_scope_addresses_junction_itself_and_scans_only_the_deleted_tree() {
    use nomi_native_files::{NativePathSemantics, acquire_instruction_scope_with_semantics};
    let fixture = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(fixture.path()).unwrap();
    let inside = root.join("inside"); let outside = root.join("outside");
    fs::create_dir_all(inside.join("deep")).unwrap(); fs::create_dir(&outside).unwrap();
    fs::write(inside.join("deep/AGENTS.md"), b"inside").unwrap();
    fs::write(outside.join("AGENTS.md"), b"outside must not be discovered").unwrap();
    let link = inside.join("linked"); junction::create(&outside, &link).unwrap();
    let entry = acquire_instruction_scope_with_semantics(&link, true, NativePathSemantics::Entry).unwrap();
    assert_eq!(entry.path_semantics, NativePathSemantics::Entry);
    assert_eq!(entry.kind, "link"); assert_eq!(Path::new(&entry.canonical_path), link);
    assert_eq!(entry.directories, BTreeSet::from([inside.to_str().unwrap().into()]));
    assert!(entry.complete); assert_eq!(entry.entries_scanned, 0);
    let tree = acquire_instruction_scope_with_semantics(&inside, true, NativePathSemantics::Entry).unwrap();
    assert!(tree.complete); assert!(tree.incomplete_reasons.is_empty());
    assert_eq!(tree.directories, BTreeSet::from([inside.to_str().unwrap().into(), inside.join("deep").to_str().unwrap().into()]));
    let followed = acquire_instruction_scope(&link, false).unwrap();
    assert_eq!(followed.path_semantics, NativePathSemantics::Follow);
    assert_eq!(Path::new(&followed.canonical_path), outside);
    assert_eq!(followed.kind, "directory");
    fs::remove_file(outside.join("AGENTS.md")).unwrap(); fs::remove_dir(&outside).unwrap();
    let dangling = acquire_instruction_scope_with_semantics(&link, true, NativePathSemantics::Entry).unwrap();
    assert_eq!(dangling.kind, "link"); assert!(dangling.complete);
    assert!(acquire_instruction_scope(&link, false).is_err(), "a dangling content link is never missing_ok absence");
    let missing = acquire_instruction_scope_with_semantics(&inside.join("future/deep/file"), true, NativePathSemantics::Entry).unwrap();
    assert_eq!(missing.kind, "missing"); assert!(missing.complete);
    junction::delete(&link).unwrap();
}

#[test]
fn entry_scope_keeps_instruction_named_links_for_the_existing_source_reader() {
    use nomi_native_files::{NativePathSemantics, acquire_instruction_scope_with_semantics};
    let fixture = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(fixture.path()).unwrap();
    let inside = root.join("inside"); let nested = inside.join("nested"); let outside = root.join("outside");
    fs::create_dir_all(&nested).unwrap(); fs::create_dir(&outside).unwrap();
    fs::write(outside.join("AGENTS.md"), b"external rules are not enumerated").unwrap();
    let instruction = nested.join("AGENTS.md"); let ordinary_link = inside.join("other-link");
    junction::create(&outside, &instruction).unwrap(); junction::create(&outside, &ordinary_link).unwrap();
    let scope = acquire_instruction_scope_with_semantics(&inside, true, NativePathSemantics::Entry).unwrap();
    let source = acquire_source_path(&instruction, 1024);
    let followed = acquire_instruction_scope(&inside, true).unwrap();
    junction::delete(&instruction).unwrap(); junction::delete(&ordinary_link).unwrap();
    assert!(scope.complete); assert!(scope.incomplete_reasons.is_empty());
    assert_eq!(scope.directories, BTreeSet::from([inside.to_str().unwrap().into(), nested.to_str().unwrap().into()]));
    assert!(!scope.directories.contains(outside.to_str().unwrap()));
    let failure = source.expect_err("a directory instruction link must reach the native source error, never absence").observation();
    assert_ne!(failure.code, "RESOURCE_NOT_FOUND");
    eprintln!("ENTRY_INSTRUCTION_LINK_SOURCE_ERROR code={} kind={:?} raw_os_error={:?}", failure.code, failure.kind, failure.raw_os_error);
    assert!(!followed.complete); assert!(followed.incomplete_reasons.contains("symlink_entry"));
    assert_eq!(fs::read(outside.join("AGENTS.md")).unwrap(), b"external rules are not enumerated");
}

#[test]
fn changed_scope_junction_does_not_supply_external_entries() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("inside"); let outside = fixture.path().join("outside");
    fs::create_dir(&root).unwrap(); fs::create_dir(&outside).unwrap(); fs::write(outside.join("AGENTS.md"), b"external").unwrap();
    let mut changed = false;
    let result = acquire_instruction_scope_with_access(&root, true, &nomi_native_files::windows::NativeWindowsAccess,
        || {}, || {}, |_| {
            fs::rename(&root, fixture.path().join("retained")).unwrap(); junction::create(&outside, &root).unwrap(); changed = true;
        }, |_| {});
    if changed { junction::delete(&root).unwrap(); fs::rename(fixture.path().join("retained"), &root).unwrap(); }
    assert!(result.is_err(), "{result:?}"); assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
}

fn icacls(path: &Path, arguments: &[&str]) {
    let result = std::process::Command::new("icacls.exe").arg(path).args(arguments).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
}

struct RestoreDacl { file: fs::File, descriptor: *mut std::ffi::c_void, acl: *mut windows_sys::Win32::Security::ACL }
impl RestoreDacl {
    fn capture(path: &Path) -> Self {
        use std::os::windows::{fs::OpenOptionsExt as _, io::AsRawHandle as _};
        use windows_sys::Win32::{Security::{DACL_SECURITY_INFORMATION, Authorization::{GetSecurityInfo, SE_FILE_OBJECT}},
            Storage::FileSystem::{READ_CONTROL, WRITE_DAC, FILE_FLAG_BACKUP_SEMANTICS}};
        let file = fs::OpenOptions::new().access_mode(READ_CONTROL | WRITE_DAC).share_mode(7).custom_flags(FILE_FLAG_BACKUP_SEMANTICS).open(path).unwrap();
        let mut result = Self { file, descriptor: std::ptr::null_mut(), acl: std::ptr::null_mut() };
        // SAFETY: output pointers receive an owned security descriptor and its
        // borrowed DACL, retained until restoration through the same handle.
        let status = unsafe { GetSecurityInfo(result.file.as_raw_handle(), SE_FILE_OBJECT, DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(), std::ptr::null_mut(), &mut result.acl, std::ptr::null_mut(), &mut result.descriptor) };
        assert_eq!(status, 0); result
    }
}
impl Drop for RestoreDacl {
    fn drop(&mut self) {
        use std::os::windows::io::AsRawHandle as _;
        use windows_sys::Win32::{Foundation::LocalFree, Security::{DACL_SECURITY_INFORMATION, Authorization::{SetSecurityInfo, SE_FILE_OBJECT}}};
        // SAFETY: the retained file has WRITE_DAC even while pathname metadata
        // access is denied; the original ACL remains in descriptor's storage.
        let status = unsafe { SetSecurityInfo(self.file.as_raw_handle(), SE_FILE_OBJECT, DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(), std::ptr::null_mut(), self.acl, std::ptr::null_mut()) };
        unsafe { LocalFree(self.descriptor) }; assert_eq!(status, 0);
    }
}

#[test]
fn denied_source_and_scope_preserve_windows_error_five() {
    let root = tempfile::tempdir().unwrap(); let target = root.path().join("AGENTS.md"); fs::write(&target, b"rules").unwrap();
    let restore = RestoreDacl::capture(&target); let restore_parent = RestoreDacl::capture(root.path());
    icacls(&target, &["/deny", "*S-1-1-0:(RD,RA)"]);
    icacls(root.path(), &["/deny", "*S-1-1-0:(RD)"]);
    let independent_metadata = nomi_native_files::windows::open_metadata(None, &fs::canonicalize(&target).unwrap_or_else(|_| {
        nomi_native_files::windows::join_component(&fs::canonicalize(root.path()).unwrap(), "AGENTS.md".as_ref()).unwrap()
    }));
    let source = acquire_source_path(&target, 1024); let scope = acquire_instruction_scope(&target, false);
    drop(restore_parent);
    drop(restore);
    assert_eq!(independent_metadata.unwrap_err().raw_os_error(), Some(5));
    for error in [source.unwrap_err(), scope.unwrap_err()] {
        assert_eq!(error.observation().code, "SYSTEM_ACCESS_DENIED");
        assert_eq!(error.observation().raw_os_error, Some(5));
    }
    assert_eq!(fs::read(&target).unwrap(), b"rules");
}
