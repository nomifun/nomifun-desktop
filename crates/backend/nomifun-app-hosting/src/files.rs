use std::{fs, io::{self, Read}, net::TcpListener, path::{Component, Path, PathBuf}};
use crate::{AppError, AppSettings, ImportKind};

const MAX_ARCHIVE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_ARCHIVE_FILES: usize = 100_000;

pub(crate) fn relative_path(value: &str) -> Result<PathBuf, AppError> {
    let path = Path::new(value);
    if path.as_os_str().is_empty() || path.components().any(|part| !matches!(part, Component::Normal(_) | Component::CurDir)) {
        return Err(AppError::Invalid(format!("path must stay inside the managed project: {value}")));
    }
    Ok(path.to_owned())
}

pub(crate) fn contained_directory(root: &Path, relative: &str) -> Result<PathBuf, AppError> {
    let requested = root.join(relative_path(relative)?);
    ensure_no_links(root, &requested)?;
    let root = dunce::canonicalize(root)?;
    let path = dunce::canonicalize(requested)?;
    if !path.starts_with(&root) || !path.is_dir() {
        return Err(AppError::Invalid("project directory must stay inside its managed copy".into()));
    }
    Ok(path)
}

pub(crate) fn copy_import(source: &Path, target: &Path) -> Result<(), AppError> {
    let source = dunce::canonicalize(source)?;
    if target.starts_with(&source) {
        return Err(AppError::Invalid("import source cannot contain the hosting directory".into()));
    }
    if source.is_dir() { copy_directory(&source, &source, target)?; }
    else if source.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("zip")) { extract_zip(&source, target)?; }
    else { return Err(AppError::Invalid("import a project folder or ZIP archive".into())); }
    Ok(())
}

fn copy_directory(source_root: &Path, source: &Path, target: &Path) -> Result<(), AppError> {
    if fs::symlink_metadata(target).is_ok_and(|metadata| is_link_like(&metadata)) {
        return Err(AppError::Invalid("overwrite refuses directory links in the managed copy".into()));
    }
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let original = entry.path();
        let resolved = dunce::canonicalize(&original)?;
        if !resolved.starts_with(source_root) {
            return Err(AppError::Invalid(format!("import contains a link outside its source folder: {}", original.display())));
        }
        // Materialize internal file links, but reject directory links: they can
        // cycle and turn an ordinary copy into an unbounded traversal.
        let metadata = fs::symlink_metadata(&original)?;
        let destination = target.join(entry.file_name());
        if is_link_like(&metadata) && resolved.is_dir() {
            return Err(AppError::Invalid(format!("import contains a directory link: {}", original.display())));
        }
        if resolved.is_dir() { copy_directory(source_root, &resolved, &destination)?; }
        else {
            if fs::symlink_metadata(&destination).is_ok_and(|metadata| is_link_like(&metadata)) {
                return Err(AppError::Invalid("overwrite refuses links in the managed copy".into()));
            }
            fs::copy(&resolved, &destination)?;
        }
    }
    Ok(())
}

fn extract_zip(source: &Path, target: &Path) -> Result<(), AppError> {
    let mut archive = zip::ZipArchive::new(fs::File::open(source)?).map_err(|error| AppError::Invalid(error.to_string()))?;
    if archive.len() > MAX_ARCHIVE_FILES { return Err(AppError::Invalid("ZIP contains too many entries".into())); }
    let mut total = 0u64;
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(|error| AppError::Invalid(error.to_string()))?;
        let relative = file.enclosed_name().ok_or_else(|| AppError::Invalid("ZIP entry escapes its managed folder".into()))?;
        if file.unix_mode().is_some_and(|mode| mode & 0o170000 == 0o120000) {
            return Err(AppError::Invalid("ZIP links are not supported; import ordinary files".into()));
        }
        total = total.checked_add(file.size()).ok_or_else(|| AppError::Invalid("ZIP is too large".into()))?;
        if total > MAX_ARCHIVE_BYTES { return Err(AppError::Invalid("ZIP expands beyond 2 GiB".into())); }
        let destination = target.join(relative);
        if file.is_dir() { ensure_no_links(target, &destination)?; fs::create_dir_all(&destination)?; }
        else {
            if let Some(parent) = destination.parent() { ensure_no_links(target, parent)?; fs::create_dir_all(parent)?; }
            if fs::symlink_metadata(&destination).is_ok_and(|metadata| is_link_like(&metadata)) { return Err(AppError::Invalid("overwrite refuses links in the managed copy".into())); }
            let mut output = fs::File::create(&destination)?;
            let written = io::copy(&mut file.by_ref().take(MAX_ARCHIVE_BYTES + 1), &mut output)?;
            if written > MAX_ARCHIVE_BYTES { return Err(AppError::Invalid("ZIP entry exceeds the size limit".into())); }
            #[cfg(unix)]
            if let Some(mode) = file.unix_mode() {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&destination, fs::Permissions::from_mode(mode & 0o777))?;
            }
        }
    }
    Ok(())
}

pub(crate) fn ensure_no_links(root: &Path, path: &Path) -> Result<(), AppError> {
    let relative = path.strip_prefix(root).map_err(|_| AppError::Invalid("path is outside its managed copy".into()))?;
    let mut current = root.to_owned();
    reject_link(&current)?;
    for part in relative.components() {
        if !matches!(part, Component::Normal(_) | Component::CurDir) {
            return Err(AppError::Invalid("managed path must not escape its owner".into()));
        }
        current.push(part);
        reject_link(&current)?;
    }
    Ok(())
}

fn reject_link(path: &Path) -> Result<(), AppError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if is_link_like(&metadata) => Err(AppError::Invalid("managed path contains a link or reparse point".into())),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AppError::Io(error)),
    }
}

fn is_link_like(metadata: &fs::Metadata) -> bool {
    if metadata.is_symlink() { return true; }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // FILE_ATTRIBUTE_REPARSE_POINT includes junctions and mount points,
        // beyond FileType's symbolic-link classification.
        return metadata.file_attributes() & 0x400 != 0;
    }
    #[cfg(not(windows))]
    false
}

pub(crate) fn detect_settings(root: &Path, kind: ImportKind) -> Result<AppSettings, AppError> {
    // A conventional ZIP with one enclosing directory is usable immediately.
    let mut project = root.to_owned();
    if !root.join("package.json").is_file() && !root.join("index.html").is_file() {
        let entries = fs::read_dir(root)?.collect::<Result<Vec<_>, _>>()?;
        if entries.len() == 1 && entries[0].path().is_dir() {
            project = entries[0].path();
        }
    }
    let mut settings = detect_project_settings(&project, kind, None)?;
    settings.project_directory = project.strip_prefix(root).expect("detected project is contained")
        .to_str().ok_or_else(|| AppError::Invalid("project directory must be UTF-8".into()))?
        .to_owned();
    if settings.project_directory.is_empty() { settings.project_directory = ".".into(); }
    prepare_static_preset(root, &mut settings)?;
    Ok(settings)
}

/// A generated application already owns its files, unlike an imported copy.
/// Resolve defaults only on an explicit configuration or run request, never
/// on reads or ordinary file writes. Detection itself does not run a process.
/// Existing foreground commands always remain authoritative.
/// With neither foreground command configured, conventional project defaults
/// can fill missing preparation steps. This is not a general missing-field
/// merge: configured projects retain explicit null install/build commands.
pub(crate) fn complete_execution_settings(root: &Path, kind: ImportKind, current: &AppSettings) -> Result<AppSettings, AppError> {
    let configured = |value: &Option<String>| value.as_deref().is_some_and(|value| !value.trim().is_empty());
    if configured(&current.preview_command) || configured(&current.start_command) { return Ok(current.clone()); }
    let project = contained_directory(root, &current.project_directory)?;
    let explicit_output = current.output_directory.as_deref().filter(|value| !value.trim().is_empty());
    let selected = if explicit_output.is_some() || current.project_directory != "." { project }
        else { find_single_project(root, &project)? };
    has_project_marker(root, &selected)?;
    let mut detected = if let Some(output) = explicit_output {
        let output_path = selected.join(relative_path(output)?);
        ensure_no_links(root, &output_path.join("index.html"))?;
        if !output_path.join("index.html").is_file() {
            return Err(AppError::Invalid("configured static output has no index.html; configure a foreground command or correct the output directory".into()));
        }
        AppSettings { start_command: Some("node \".nomifun-static-server.cjs\"".into()), output_directory: Some(output.into()), ..Default::default() }
    } else {
        detect_project_settings(&selected, kind, current.environment.get("PORT").map(String::as_str))?
    };
    if !configured(&detected.preview_command) && !configured(&detected.start_command) {
        return Err(AppError::Invalid("no runnable project was detected; configure a foreground preview or production command".into()));
    }
    detected.project_directory = selected.strip_prefix(root).expect("detected project is contained")
        .to_str().ok_or_else(|| AppError::Invalid("project directory must be UTF-8".into()))?.to_owned();
    if detected.project_directory.is_empty() { detected.project_directory = ".".into(); }
    // Preserve explicit preparation commands and ordinary runtime settings.
    let mut settings = current.clone();
    settings.project_directory = detected.project_directory;
    settings.preview_command = detected.preview_command;
    settings.start_command = detected.start_command;
    if settings.install_command.is_none() { settings.install_command = detected.install_command; }
    if settings.build_command.is_none() { settings.build_command = detected.build_command; }
    if settings.output_directory.is_none() { settings.output_directory = detected.output_directory; }
    let static_preset = settings.start_command.as_deref().is_some_and(|command| command.contains(".nomifun-static-server.cjs"));
    if !static_preset && settings.web_url.as_deref().is_none_or(|value| value.trim().is_empty()) { settings.web_url = detected.web_url; }
    for (key, value) in detected.environment { settings.environment.entry(key).or_insert(value); }
    prepare_static_preset(root, &mut settings)?;
    Ok(settings)
}

fn has_project_marker(root: &Path, path: &Path) -> Result<bool, AppError> {
    let mut found = false;
    for marker in ["package.json", "index.html", "server.js", "server.mjs", "server.cjs", "index.js", "dist/index.html", "build/index.html"] {
        let file = path.join(marker);
        ensure_no_links(root, &file)?;
        found |= file.is_file();
    }
    Ok(found)
}

fn find_single_project(root: &Path, start: &Path) -> Result<PathBuf, AppError> {
    let mut pending = vec![(start.to_owned(), 0usize)];
    let mut candidates = Vec::new();
    let mut visited = 0usize;
    while let Some((directory, depth)) = pending.pop() {
        visited += 1;
        if visited > 256 { return Err(AppError::Invalid("project discovery is too broad; select the project directory in application settings".into())); }
        ensure_no_links(root, &directory)?;
        if has_project_marker(root, &directory)? {
            candidates.push(directory);
            if candidates.len() > 1 { return Err(AppError::Invalid("multiple runnable projects were found; select one project directory in application settings".into())); }
            // Nested assets and build output belong to this project.
            continue;
        }
        if depth >= 4 { continue; }
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let name = entry.file_name();
            if name.to_str().is_some_and(|name| name.starts_with('.') || matches!(name, "node_modules" | "target" | "vendor")) { continue; }
            let path = entry.path();
            ensure_no_links(root, &path)?;
            if entry.file_type()?.is_dir() { pending.push((path, depth + 1)); }
        }
    }
    candidates.pop().ok_or_else(|| AppError::Invalid("no runnable project was found; create index.html or configure a foreground command".into()))
}

fn detect_project_settings(project: &Path, kind: ImportKind, port_hint: Option<&str>) -> Result<AppSettings, AppError> {
    let mut settings = AppSettings::default();
    let package_path = project.join("package.json");
    if package_path.is_file() {
        let package: serde_json::Value = serde_json::from_slice(&fs::read(package_path)?)?;
        let scripts = package.get("scripts");
        let script = |key: &str| scripts.and_then(|s| s.get(key)).and_then(|s| s.as_str()).filter(|s| !s.trim().is_empty());
        let manager = if project.join("bun.lock").exists() || project.join("bun.lockb").exists() { "bun" }
            else if project.join("pnpm-lock.yaml").exists() { "pnpm" }
            else if project.join("yarn.lock").exists() { "yarn" } else { "npm" };
        if kind == ImportKind::Source {
            settings.install_command = Some(format!("{manager} install"));
            settings.build_command = script("build").map(|_| format!("{manager} run build"));
            settings.preview_command = script("dev").map(|_| format!("{manager} run dev"));
        }
        settings.start_command = script("start").map(|_| format!("{manager} run start"));
        let web_script = script("dev").or_else(|| script("start"));
        if let Some(command) = web_script {
            if command.contains("vite") || command.contains("next ") || command.contains("vue-cli-service") {
                let port = selected_port(port_hint)?;
                settings.environment.insert("PORT".into(), port.to_string());
                settings.web_url = Some(format!("http://127.0.0.1:{port}"));
                if command.contains("vite") {
                    if let Some(preview) = &mut settings.preview_command {
                        let separator = if manager == "npm" { " --" } else { "" };
                        preview.push_str(&format!("{separator} --host 127.0.0.1 --port {port} --strictPort"));
                    }
                }
            }
        }
    }
    if settings.start_command.is_none() {
        for filename in ["server.js", "server.mjs", "server.cjs", "index.js"] {
            if project.join(filename).is_file() { settings.start_command = Some(format!("node {filename}")); break; }
        }
    }
    let static_root = if project.join("dist/index.html").is_file() { Some("dist") }
        else if project.join("build/index.html").is_file() { Some("build") }
        else if project.join("index.html").is_file() { Some(".") }
        else if settings.build_command.is_some() && fs::read_to_string(project.join("package.json")).is_ok_and(|p| p.contains("vite")) { Some("dist") }
        else { None };
    if settings.start_command.is_none() {
        if let Some(directory) = static_root {
            // Preview and ordinary static use share the one App web address.
            let port = selected_port(settings.environment.get("PORT").map(String::as_str).or(port_hint))?;
            settings.output_directory = Some(directory.into());
            settings.start_command = Some("node \".nomifun-static-server.cjs\"".into());
            settings.web_url = Some(format!("http://127.0.0.1:{port}"));
            settings.environment.insert("PORT".into(), port.to_string());
        }
    }
    Ok(settings)
}

/// Explicit ordinary HTTP preset, also usable with non-conventional output
/// directories. Saving an output directory alone does not select this preset;
/// an explicit run can resolve it when a real index.html is present.
pub(crate) fn prepare_static_preset(root: &Path, settings: &mut AppSettings) -> Result<(), AppError> {
    if !settings.start_command.as_deref().is_some_and(|command| matches!(command.trim(), "node .nomifun-static-server.cjs" | "node \".nomifun-static-server.cjs\"")) {
        return Ok(());
    }
    let project = contained_directory(root, &settings.project_directory)?;
    let output = settings.output_directory.get_or_insert_with(|| ".".into());
    relative_path(output)?;
    let server = project.join(".nomifun-static-server.cjs");
    ensure_no_links(root, &server)?;
    let port = match settings.environment.get("PORT") {
        Some(value) if !value.trim().is_empty() => value.parse::<u16>().ok().filter(|port| *port != 0)
            .ok_or_else(|| AppError::Invalid("static HTTP preset PORT must be between 1 and 65535".into()))?,
        _ => free_port()?,
    };
    settings.environment.insert("PORT".into(), port.to_string());
    if settings.web_url.as_deref().is_none_or(|value| value.trim().is_empty()) {
        settings.web_url = Some(format!("http://127.0.0.1:{port}"));
    }
    fs::write(server, STATIC_SERVER)?;
    Ok(())
}

fn free_port() -> io::Result<u16> { Ok(TcpListener::bind(("127.0.0.1", 0))?.local_addr()?.port()) }

fn selected_port(hint: Option<&str>) -> Result<u16, AppError> {
    match hint.filter(|value| !value.trim().is_empty()) {
        Some(value) => value.parse::<u16>().ok().filter(|port| *port != 0)
            .ok_or_else(|| AppError::Invalid("HTTP preset PORT must be between 1 and 65535".into())),
        None => Ok(free_port()?),
    }
}

/// The standard Python static server has a fixed command-line port contract.
/// Read only a single literal invocation, never shell expressions or arbitrary
/// backend source. An explicit user URL remains authoritative in the owner.
pub(crate) fn standard_static_web_url(root: &Path, settings: &AppSettings, command: &str) -> Option<String> {
    let arguments = literal_arguments(command)?;
    let executable = Path::new(arguments.first()?).file_name()?.to_str()?.to_ascii_lowercase();
    let executable = executable.strip_suffix(".exe").unwrap_or(&executable);
    let python = executable == "python" || executable == "py" || executable.strip_prefix("python").is_some_and(|suffix|
        suffix.split('.').all(|component| !component.is_empty() && component.chars().all(|character| character.is_ascii_digit())));
    if !python { return None; }
    let mut index = 1;
    if executable == "py" && arguments.get(index).is_some_and(|argument|
        argument.starts_with("-3") && argument[2..].chars().all(|character| character.is_ascii_digit() || character == '.')) {
        index += 1;
    }
    if arguments.get(index).map(String::as_str) != Some("-m") || arguments.get(index + 1).map(String::as_str) != Some("http.server") { return None; }
    index += 2;
    let mut port = 8000u16;
    let mut explicit_port = false;
    let mut bind = "127.0.0.1".to_owned();
    let mut directory = None;
    while let Some(argument) = arguments.get(index) {
        match argument.as_str() {
            "--bind" | "-b" => { index += 1; bind = arguments.get(index)?.clone(); }
            "--directory" | "-d" => { index += 1; directory = Some(arguments.get(index)?.as_str()); }
            _ if argument.starts_with("--bind=") => bind = argument.strip_prefix("--bind=")?.to_owned(),
            _ if argument.starts_with("--directory=") => directory = argument.strip_prefix("--directory="),
            _ if !explicit_port && argument.chars().all(|character| character.is_ascii_digit()) => {
                port = argument.parse().ok().filter(|port| *port != 0)?;
                explicit_port = true;
            }
            _ => return None,
        }
        index += 1;
    }
    let project = contained_directory(root, &settings.project_directory).ok()?;
    let served = if let Some(directory) = directory {
        let directory = project.join(relative_path(directory).ok()?);
        ensure_no_links(root, &directory).ok()?;
        let directory = dunce::canonicalize(directory).ok()?;
        if !directory.starts_with(root) || !directory.is_dir() { return None; }
        directory
    } else { project };
    let host = match bind.as_str() {
        "0.0.0.0" | "127.0.0.1" => "127.0.0.1".to_owned(),
        "::" | "::1" => "[::1]".to_owned(),
        "localhost" => "localhost".to_owned(),
        value => match value.parse::<std::net::IpAddr>().ok()? {
            std::net::IpAddr::V4(address) if address.is_loopback() => address.to_string(),
            std::net::IpAddr::V6(address) if address.is_loopback() => format!("[{address}]"),
            _ => return None,
        },
    };
    let suffix = single_static_index_path(root, &served).ok()?.unwrap_or_default();
    Some(format!("http://{host}:{port}/{suffix}"))
}

/// A literal Python static command already establishes its served root. When
/// the root has no index and exactly one real nested HTML entry, point preview
/// there without rewriting the command, its cwd, or a live process identity.
fn single_static_index_path(root: &Path, served: &Path) -> Result<Option<String>, AppError> {
    ensure_no_links(root, &served.join("index.html"))?;
    if served.join("index.html").is_file() { return Ok(Some(String::new())); }
    let mut pending = vec![(served.to_owned(), 0usize)];
    let mut candidate = None;
    let mut visited = 0usize;
    while let Some((directory, depth)) = pending.pop() {
        visited += 1;
        if visited > 256 { return Ok(None); }
        ensure_no_links(root, &directory)?;
        ensure_no_links(root, &directory.join("index.html"))?;
        if directory.join("index.html").is_file() {
            if candidate.is_some() { return Ok(None); }
            candidate = Some(directory);
            continue;
        }
        if depth >= 4 { continue; }
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let name = entry.file_name();
            if name.to_str().is_some_and(|name| name.starts_with('.') || matches!(name, "node_modules" | "target" | "vendor")) { continue; }
            let path = entry.path();
            ensure_no_links(root, &path)?;
            if entry.file_type()?.is_dir() { pending.push((path, depth + 1)); }
        }
    }
    let Some(candidate) = candidate else { return Ok(None); };
    let relative = candidate.strip_prefix(served).expect("static index was discovered beneath the served root");
    let mut suffix = String::new();
    for component in relative.components() {
        let Some(value) = component.as_os_str().to_str() else { return Ok(None); };
        for byte in value.bytes() {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') { suffix.push(char::from(byte)); }
            else { use std::fmt::Write; write!(&mut suffix, "%{byte:02X}").expect("String writes cannot fail"); }
        }
        suffix.push('/');
    }
    Ok(Some(suffix))
}

fn literal_arguments(command: &str) -> Option<Vec<String>> {
    // cmd and POSIX shells interpret these even inside some quoting forms.
    if command.contains(['&', '|', ';', '<', '>', '$', '`', '%', '^', '\n', '\r', '\0']) { return None; }
    #[cfg(windows)]
    if command.contains('\'') { return None; }
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    for character in command.chars() {
        match (quote, character) {
            (Some(expected), value) if value == expected => quote = None,
            (Some(_), value) => current.push(value),
            (None, '"' | '\'') => quote = Some(character),
            (None, value) if value.is_whitespace() => {
                if !current.is_empty() { arguments.push(std::mem::take(&mut current)); }
            }
            (None, value) => current.push(value),
        }
    }
    if quote.is_some() { return None; }
    if !current.is_empty() { arguments.push(current); }
    Some(arguments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detected_vite_preview_and_static_use_share_one_real_port_contract() {
        let temporary = tempfile::tempdir().unwrap();
        fs::write(temporary.path().join("package.json"), r#"{"scripts":{"dev":"vite","build":"vite build"}}"#).unwrap();
        let settings = detect_settings(temporary.path(), ImportKind::Source).unwrap();
        let port = &settings.environment["PORT"];
        assert_eq!(settings.web_url.as_deref(), Some(format!("http://127.0.0.1:{port}").as_str()));
        assert!(settings.preview_command.as_ref().unwrap().contains(&format!("--port {port} --strictPort")));
        assert_eq!(settings.start_command.as_deref(), Some("node \".nomifun-static-server.cjs\""));
        assert_eq!(settings.output_directory.as_deref(), Some("dist"));
    }

    #[test]
    fn only_literal_standard_static_servers_have_inferable_urls() {
        let temporary = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(temporary.path()).unwrap();
        fs::create_dir(root.join("web files")).unwrap();
        let settings = AppSettings::default();
        for (command, url) in [
            ("python -m http.server 8080", "http://127.0.0.1:8080/"),
            ("python3 -m http.server", "http://127.0.0.1:8000/"),
            ("py -3.13 -m http.server 8123 --bind 0.0.0.0", "http://127.0.0.1:8123/"),
            ("python -m http.server 8123 --bind=::", "http://[::1]:8123/"),
            ("python -m http.server --directory \"web files\" 8123", "http://127.0.0.1:8123/"),
        ] {
            assert_eq!(standard_static_web_url(&root, &settings, command).as_deref(), Some(url), "{command}");
        }
        for command in [
            "node server.cjs", "echo python -m http.server 8080", "python -m http.server 8080 & echo ready",
            "python -m http.server 8080 > output.log", "python -m http.server %PORT%", "python -m http.server $PORT",
            "python -m http.server 0", "python -m http.server 8080 --directory ../outside", "python -m http.server --bind=example.com",
            "python -m http.server 8080 --unknown-option", "python -m http.server 8080 --directory \"web files",
        ] {
            assert!(standard_static_web_url(&root, &settings, command).is_none(), "must not infer {command}");
        }
    }

    #[test]
    fn standard_python_static_preview_uses_only_an_unambiguous_real_nested_index() {
        let temporary = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(temporary.path()).unwrap();
        fs::create_dir(root.join("贪吃蛇 game")).unwrap();
        fs::write(root.join("贪吃蛇 game/index.html"), "GAME").unwrap();
        fs::write(root.join("README.md"), "user source").unwrap();
        fs::create_dir(root.join(".git")).unwrap();
        let settings = AppSettings::default();
        assert_eq!(standard_static_web_url(&root, &settings, "python -m http.server 8080").as_deref(),
            Some("http://127.0.0.1:8080/%E8%B4%AA%E5%90%83%E8%9B%87%20game/"));
        assert_eq!(standard_static_web_url(&root, &settings, "python -m http.server 8080 --directory \"贪吃蛇 game\"").as_deref(),
            Some("http://127.0.0.1:8080/"));
        fs::create_dir(root.join("another-game")).unwrap();
        fs::write(root.join("another-game/index.html"), "OTHER").unwrap();
        assert_eq!(standard_static_web_url(&root, &settings, "python -m http.server 8080").as_deref(),
            Some("http://127.0.0.1:8080/"), "multiple HTML projects must not invent a selected page");
        fs::write(root.join("index.html"), "ROOT").unwrap();
        assert_eq!(standard_static_web_url(&root, &settings, "python -m http.server 8080").as_deref(), Some("http://127.0.0.1:8080/"));
    }

    #[cfg(windows)]
    #[test]
    fn standard_python_static_preview_does_not_infer_a_page_through_a_junction() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("owned");
        let external = temporary.path().join("external");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&external).unwrap();
        fs::write(external.join("index.html"), "EXTERNAL_SOURCE").unwrap();
        let root = dunce::canonicalize(root).unwrap();
        let link = root.join("nested-app");
        junction::create(&external, &link).unwrap();
        let inferred = standard_static_web_url(&root, &AppSettings::default(), "python -m http.server 8080");
        junction::delete(&link).unwrap();
        assert!(inferred.is_none());
        assert_eq!(fs::read_to_string(external.join("index.html")).unwrap(), "EXTERNAL_SOURCE");
    }
}

pub(crate) const STATIC_SERVER: &str = r#"'use strict';
const http = require('node:http');
const fs = require('node:fs');
const path = require('node:path');
const root = fs.realpathSync(path.resolve(process.env.NOMIFUN_STATIC_ROOT || process.argv[2] || '.'));
const mime = {'.html':'text/html; charset=utf-8','.js':'text/javascript','.css':'text/css','.json':'application/json','.svg':'image/svg+xml','.png':'image/png','.jpg':'image/jpeg','.ico':'image/x-icon','.wasm':'application/wasm'};
const server = http.createServer((req, res) => {
  try {
    const route = decodeURIComponent(new URL(req.url, 'http://localhost').pathname);
    let file = path.resolve(root, '.' + route);
    if (file !== root && !file.startsWith(root + path.sep)) { res.writeHead(403).end(); return; }
    if (fs.statSync(file).isDirectory()) file = path.join(file, 'index.html');
    file = fs.realpathSync(file);
    if (!file.startsWith(root + path.sep)) { res.writeHead(403).end(); return; }
    res.setHeader('Content-Type', mime[path.extname(file)] || 'application/octet-stream');
    if (req.method === 'HEAD') { res.end(); return; }
    fs.createReadStream(file).on('error', () => res.destroy()).pipe(res);
  } catch { res.writeHead(404).end('Not found'); }
});
server.on('error', error => { console.error(error.message); process.exit(1); });
server.listen(Number(process.env.PORT), '127.0.0.1', () => console.log('Serving static files'));
process.on('SIGTERM', () => server.close(() => process.exit(0)));
"#;
