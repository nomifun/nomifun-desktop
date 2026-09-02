use std::io::Read as _;

use cap_fs_ext::{
    DirExt, FollowSymlinks, OpenOptionsFollowExt,
};
use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions};

use super::*;

const MAX_ANCHORED_KNOWLEDGE_DEPTH: usize = 128;

pub(super) struct AnchoredKnowledgeFs {
    root: Dir,
}

pub(super) struct AnchoredMarkdownFile {
    pub rel_path: String,
    pub content: String,
    pub size: u64,
}

impl AnchoredKnowledgeFs {
    pub(super) fn open(root: &Path) -> Result<Self, AppError> {
        validate_bound_knowledge_root(root)?;

        #[cfg(target_os = "macos")]
        let directory = {
            // macOS exposes ordinary temporary paths through fixed system
            // aliases such as /var -> /private/var. Rewrite only those known
            // aliases lexically; every user-controlled component is still
            // opened with no-follow semantics, so an intermediate symlink is
            // rejected rather than silently canonicalized.
            open_absolute_directory_nofollow(&macos_system_alias_path(root))?
        };

        #[cfg(not(target_os = "macos"))]
        let directory = open_absolute_directory_nofollow(root)?;

        Ok(Self { root: directory })
    }

    pub(super) fn read_markdown(
        &self,
        rel_path: &str,
        max_bytes: u64,
    ) -> Result<AnchoredMarkdownFile, AppError> {
        let components = validate_relative_markdown_path(rel_path)?;
        let (file, canonical_rel_path) =
            open_markdown_components(&self.root, &components)?;
        let metadata = file.metadata().map_err(|_| {
            AppError::Conflict(
                "bound knowledge document metadata is unavailable".into(),
            )
        })?;
        if !metadata.is_file() {
            return Err(AppError::BadRequest(
                "knowledge document must be a regular Markdown file".into(),
            ));
        }
        if metadata.len() > max_bytes {
            return Err(AppError::BadRequest(format!(
                "knowledge document exceeds the {} MiB read limit",
                max_bytes / 1024 / 1024
            )));
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.into_std()
            .take(max_bytes + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| {
                AppError::Internal(
                    "failed to read bound knowledge document".into(),
                )
            })?;
        if bytes.len() as u64 > max_bytes {
            return Err(AppError::BadRequest(format!(
                "knowledge document exceeds the {} MiB read limit",
                max_bytes / 1024 / 1024
            )));
        }
        let content = String::from_utf8(bytes).map_err(|_| {
            AppError::BadRequest(
                "knowledge document must contain valid UTF-8 text".into(),
            )
        })?;
        Ok(AnchoredMarkdownFile {
            rel_path: canonical_rel_path,
            size: content.len() as u64,
            content,
        })
    }

    pub(super) fn search_documents(
        &self,
        kb_id: &KnowledgeBaseId,
        kb_name: &str,
        limits: RetrievalLoadLimits,
    ) -> Result<Vec<RetrievalDocument>, AppError> {
        let mut collector = AnchoredSearchCollector {
            kb_id,
            kb_name,
            limits,
            visited_entries: 0,
            total_bytes: 0,
            documents: Vec::new(),
        };
        collector.collect(&self.root, "", 0)?;
        Ok(collector.documents)
    }
}

#[cfg(target_os = "macos")]
fn macos_system_alias_path(path: &Path) -> PathBuf {
    const ALIASES: &[(&str, &str)] = &[
        ("/var", "/private/var"),
        ("/tmp", "/private/tmp"),
        ("/etc", "/private/etc"),
        ("/home", "/System/Volumes/Data/home"),
    ];

    for (alias, target) in ALIASES {
        let alias_path = Path::new(alias);
        let Some(observed_target) = std::fs::read_link(alias_path)
            .ok()
            .map(|observed| {
                if observed.is_absolute() {
                    observed
                } else {
                    alias_path
                        .parent()
                        .unwrap_or_else(|| Path::new("/"))
                        .join(observed)
                }
            })
        else {
            continue;
        };
        if observed_target != Path::new(target) {
            continue;
        }
        if path == alias_path {
            return PathBuf::from(target);
        }
        if let Ok(suffix) = path.strip_prefix(alias_path) {
            return Path::new(target).join(suffix);
        }
    }
    path.to_owned()
}

fn validate_bound_knowledge_root(root: &Path) -> Result<(), AppError> {
    if !root.is_absolute() {
        return Err(AppError::BadRequest(
            "bound knowledge root must be absolute".into(),
        ));
    }
    let mut normal_components = 0usize;
    let mut saw_root = false;
    for component in root.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => {
                if normal_components != 0 {
                    return Err(AppError::BadRequest(
                        "bound knowledge root is not canonical".into(),
                    ));
                }
                saw_root = true;
            }
            Component::Normal(_) => {
                normal_components = normal_components.saturating_add(1);
            }
            Component::CurDir | Component::ParentDir => {
                return Err(AppError::BadRequest(
                    "bound knowledge root is not canonical".into(),
                ));
            }
        }
    }
    if !saw_root || normal_components == 0 {
        return Err(AppError::BadRequest(
            "a filesystem, drive, or network-share root cannot be used as a knowledge base"
                .into(),
        ));
    }
    Ok(())
}

fn open_absolute_directory_nofollow(path: &Path) -> Result<Dir, AppError> {
    let mut anchor = PathBuf::new();
    let mut components = Vec::new();
    let mut saw_root = false;
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => {
                if !components.is_empty() {
                    return Err(AppError::Conflict(
                        "bound knowledge root path is unavailable".into(),
                    ));
                }
                anchor.push(component.as_os_str());
                saw_root = true;
            }
            Component::Normal(component) => {
                components.push(component.to_owned());
            }
            Component::CurDir | Component::ParentDir => {
                return Err(AppError::Conflict(
                    "bound knowledge root path is unavailable".into(),
                ));
            }
        }
    }
    if !saw_root {
        return Err(AppError::Conflict(
            "bound knowledge root path is unavailable".into(),
        ));
    }
    let mut directory =
        Dir::open_ambient_dir(&anchor, ambient_authority()).map_err(|_| {
            AppError::Conflict(
                "bound knowledge root is unavailable".into(),
            )
        })?;
    for component in components {
        directory = directory
            .open_dir_nofollow(Path::new(&component))
            .map_err(|_| {
                AppError::Conflict(
                    "bound knowledge root contains an unavailable or unsafe directory"
                        .into(),
                )
            })?;
    }
    Ok(directory)
}

fn validate_relative_markdown_path(
    rel_path: &str,
) -> Result<Vec<String>, AppError> {
    if rel_path.is_empty() || rel_path.contains('\\') {
        return Err(AppError::BadRequest(
            "knowledge path must be a normalized relative path".into(),
        ));
    }
    let path = Path::new(rel_path);
    if path.is_absolute() || !is_md(path) {
        return Err(AppError::BadRequest(
            "knowledge path must name a relative Markdown file".into(),
        ));
    }
    let mut components = Vec::new();
    for component in path.components() {
        let Component::Normal(component) = component else {
            return Err(AppError::BadRequest(
                "knowledge path contains an invalid component".into(),
            ));
        };
        let component = component.to_str().ok_or_else(|| {
            AppError::BadRequest(
                "knowledge path must contain valid Unicode".into(),
            )
        })?;
        validate_portable_path_component(component)?;
        components.push(component.to_owned());
    }
    if components
        .iter()
        .take(components.len().saturating_sub(1))
        .any(|component| is_excluded_tree_dir_name(component))
    {
        return Err(AppError::BadRequest(
            "knowledge path crosses an excluded directory".into(),
        ));
    }
    Ok(components)
}

fn open_markdown_components(
    root: &Dir,
    components: &[String],
) -> Result<(cap_std::fs::File, String), AppError> {
    let (file_name, parents) = components.split_last().ok_or_else(|| {
        AppError::BadRequest("knowledge path must not be empty".into())
    })?;
    let mut directory = root.try_clone().map_err(|_| {
        AppError::Conflict(
            "bound knowledge root handle is unavailable".into(),
        )
    })?;
    let mut canonical = Vec::with_capacity(components.len());
    for component in parents {
        directory = directory
            .open_dir_nofollow(component)
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    AppError::NotFound(
                        "knowledge document was not found".into(),
                    )
                } else {
                    AppError::BadRequest(
                        "knowledge path crosses an unsafe directory".into(),
                    )
                }
            })?;
        canonical.push(component.clone());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    options.follow(FollowSymlinks::No);
    let file = directory
        .open_with(file_name, &options)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                AppError::NotFound(
                    "knowledge document was not found".into(),
                )
            } else {
                AppError::BadRequest(
                    "knowledge document is unavailable or unsafe".into(),
                )
            }
        })?;
    canonical.push(file_name.clone());
    Ok((file, canonical.join("/")))
}

struct AnchoredSearchCollector<'a> {
    kb_id: &'a KnowledgeBaseId,
    kb_name: &'a str,
    limits: RetrievalLoadLimits,
    visited_entries: usize,
    total_bytes: u64,
    documents: Vec<RetrievalDocument>,
}

impl AnchoredSearchCollector<'_> {
    fn collect(
        &mut self,
        directory: &Dir,
        parent: &str,
        depth: usize,
    ) -> Result<(), AppError> {
        if depth > MAX_ANCHORED_KNOWLEDGE_DEPTH {
            return Err(AppError::BadRequest(format!(
                "knowledge search exceeds the {MAX_ANCHORED_KNOWLEDGE_DEPTH}-level directory depth limit"
            )));
        }
        let entries = directory
            .entries()
            .map_err(|_| {
                AppError::Conflict(
                    "bound knowledge directory cannot be enumerated".into(),
                )
            })?;
        for entry in entries {
            let entry = entry.map_err(|_| {
                AppError::Conflict(
                    "bound knowledge directory enumeration failed".into(),
                )
            })?;
            self.visited_entries = self.visited_entries.saturating_add(1);
            if self.visited_entries > self.limits.max_entries {
                return Err(AppError::BadRequest(format!(
                    "knowledge search exceeds the {} filesystem entry limit",
                    self.limits.max_entries
                )));
            }
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            if validate_portable_path_component(&name).is_err() {
                continue;
            }
            let rel_path = if parent.is_empty() {
                name.clone()
            } else {
                format!("{parent}/{name}")
            };
            if let Ok(child) = directory.open_dir_nofollow(&name) {
                if !is_excluded_tree_dir_name(&name) {
                    self.collect(&child, &rel_path, depth + 1)?;
                }
                continue;
            }
            if !is_md(Path::new(&name)) {
                continue;
            }
            let mut options = OpenOptions::new();
            options.read(true);
            options.follow(FollowSymlinks::No);
            let Ok(file) = directory.open_with(&name, &options) else {
                continue;
            };
            let Ok(metadata) = file.metadata() else {
                continue;
            };
            if !metadata.is_file() {
                continue;
            }
            if self.documents.len() >= self.limits.max_documents {
                return Err(AppError::BadRequest(format!(
                    "knowledge search exceeds the {} document limit",
                    self.limits.max_documents
                )));
            }
            if metadata.len() > self.limits.max_file_bytes {
                return Err(AppError::BadRequest(format!(
                    "knowledge search document exceeds the {} MiB file limit",
                    self.limits.max_file_bytes / 1024 / 1024
                )));
            }
            let mut bytes = Vec::with_capacity(metadata.len() as usize);
            if file
                .into_std()
                .take(self.limits.max_file_bytes + 1)
                .read_to_end(&mut bytes)
                .is_err()
            {
                continue;
            }
            if bytes.len() as u64 > self.limits.max_file_bytes {
                return Err(AppError::BadRequest(format!(
                    "knowledge search document exceeds the {} MiB file limit",
                    self.limits.max_file_bytes / 1024 / 1024
                )));
            }
            let Ok(content) = String::from_utf8(bytes) else {
                continue;
            };
            self.total_bytes = self
                .total_bytes
                .checked_add(content.len() as u64)
                .ok_or_else(|| {
                    AppError::BadRequest(
                        "knowledge search content size overflowed".into(),
                    )
                })?;
            if self.total_bytes > self.limits.max_total_bytes {
                return Err(AppError::BadRequest(format!(
                    "knowledge search exceeds the {} MiB total content limit",
                    self.limits.max_total_bytes / 1024 / 1024
                )));
            }
            self.documents.push(RetrievalDocument {
                kb_id: self.kb_id.clone(),
                kb_name: self.kb_name.to_owned(),
                rel_path,
                heading: first_heading_text(&content),
                content: content.into(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> RetrievalLoadLimits {
        RetrievalLoadLimits {
            max_entries: 64,
            max_documents: 16,
            max_file_bytes: 1024 * 1024,
            max_total_bytes: 4 * 1024 * 1024,
        }
    }

    #[cfg(unix)]
    fn link_directory(source: &Path, target: &Path) {
        std::os::unix::fs::symlink(source, target).unwrap();
    }

    #[cfg(unix)]
    fn link_file(source: &Path, target: &Path) {
        std::os::unix::fs::symlink(source, target).unwrap();
    }

    #[cfg(windows)]
    fn link_directory(source: &Path, target: &Path) {
        junction::create(source, target).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn anchored_knowledge_root_accepts_the_macos_var_system_alias() {
        let directory = tempfile::Builder::new()
            .prefix("nomifun-anchored-knowledge-")
            .tempdir_in("/var/tmp")
            .unwrap();
        let root = directory.path().join("knowledge");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("visible.md"), "# Visible").unwrap();

        let anchored = AnchoredKnowledgeFs::open(&root).unwrap();
        assert_eq!(
            anchored
                .read_markdown("visible.md", 1024)
                .unwrap()
                .content,
            "# Visible"
        );
    }

    #[test]
    fn anchored_knowledge_search_and_read_reject_linked_content() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("knowledge");
        let outside = directory.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(root.join("visible.md"), "# Visible\ninside").unwrap();
        std::fs::write(outside.join("secret.md"), "# Secret\noutside").unwrap();
        link_directory(&outside, &root.join("escape"));

        let anchored = AnchoredKnowledgeFs::open(&root).unwrap();
        let documents = anchored
            .search_documents(
                &KnowledgeBaseId::new(),
                "test",
                limits(),
            )
            .unwrap();
        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].rel_path, "visible.md");
        assert!(
            anchored
                .read_markdown("escape/secret.md", 1024 * 1024)
                .is_err(),
            "an anchored read must not follow a linked directory"
        );
    }

    #[cfg(unix)]
    #[test]
    fn anchored_knowledge_search_and_read_reject_a_linked_markdown_file() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("knowledge");
        let outside = directory.path().join("outside.md");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("visible.md"), "# Visible\ninside").unwrap();
        std::fs::write(&outside, "# Secret\noutside").unwrap();
        link_file(&outside, &root.join("secret.md"));

        let anchored = AnchoredKnowledgeFs::open(&root).unwrap();
        let documents = anchored
            .search_documents(&KnowledgeBaseId::new(), "test", limits())
            .unwrap();
        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].rel_path, "visible.md");
        assert!(
            anchored.read_markdown("secret.md", 1024).is_err(),
            "an anchored read must not follow a linked Markdown file"
        );
    }

    #[test]
    fn anchored_knowledge_root_rejects_a_linked_component() {
        let directory = tempfile::tempdir().unwrap();
        let outside = directory.path().join("outside");
        let linked_root = directory.path().join("linked-root");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.md"), "# Secret").unwrap();
        link_directory(&outside, &linked_root);

        assert!(
            AnchoredKnowledgeFs::open(&linked_root).is_err(),
            "the root capability must not be established through a link or junction"
        );
    }

    #[test]
    fn anchored_knowledge_root_rejects_an_intermediate_linked_component() {
        let directory = tempfile::tempdir().unwrap();
        let outside = directory.path().join("outside");
        let linked_parent = directory.path().join("linked-parent");
        std::fs::create_dir_all(outside.join("knowledge")).unwrap();
        link_directory(&outside, &linked_parent);

        assert!(
            AnchoredKnowledgeFs::open(&linked_parent.join("knowledge")).is_err(),
            "a symlink in the root path must not be canonicalized into an accepted root"
        );
    }

    #[test]
    fn anchored_child_handle_survives_or_blocks_path_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("knowledge");
        let child_path = root.join("nested");
        let moved_path = root.join("nested-original");
        let outside = directory.path().join("outside");
        std::fs::create_dir_all(&child_path).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(child_path.join("note.md"), "# Original").unwrap();
        std::fs::write(outside.join("note.md"), "# Outside").unwrap();

        let anchored = AnchoredKnowledgeFs::open(&root).unwrap();
        let child = anchored.root.open_dir_nofollow("nested").unwrap();
        let replaced = std::fs::rename(&child_path, &moved_path).is_ok();
        if replaced {
            link_directory(&outside, &child_path);
        }

        let mut options = OpenOptions::new();
        options.read(true);
        options.follow(FollowSymlinks::No);
        let mut content = String::new();
        child
            .open_with("note.md", &options)
            .unwrap()
            .into_std()
            .read_to_string(&mut content)
            .unwrap();
        assert_eq!(content, "# Original");

        #[cfg(windows)]
        assert!(
            !replaced,
            "the Windows directory capability must deny path replacement while open"
        );
    }

    #[test]
    fn anchored_root_handle_survives_or_blocks_root_path_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("knowledge");
        let moved_root = directory.path().join("knowledge-original");
        let outside = directory.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(root.join("note.md"), "# Original").unwrap();
        std::fs::write(outside.join("note.md"), "# Outside").unwrap();

        let anchored = AnchoredKnowledgeFs::open(&root).unwrap();
        let replaced = std::fs::rename(&root, &moved_root).is_ok();
        if replaced {
            link_directory(&outside, &root);
        }

        assert_eq!(
            anchored.read_markdown("note.md", 1024).unwrap().content,
            "# Original"
        );

        #[cfg(windows)]
        assert!(
            !replaced,
            "the Windows root directory capability must deny root replacement while open"
        );
    }

    #[test]
    fn anchored_knowledge_search_limits_fail_closed() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("knowledge");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("one.md"), "# One\ncontent").unwrap();
        std::fs::write(root.join("two.md"), "# Two\ncontent").unwrap();
        let anchored = AnchoredKnowledgeFs::open(&root).unwrap();
        let kb_id = KnowledgeBaseId::new();

        for limits in [
            RetrievalLoadLimits {
                max_entries: 0,
                ..limits()
            },
            RetrievalLoadLimits {
                max_documents: 0,
                ..limits()
            },
            RetrievalLoadLimits {
                max_file_bytes: 1,
                ..limits()
            },
            RetrievalLoadLimits {
                max_total_bytes: 1,
                ..limits()
            },
        ] {
            assert!(matches!(
                anchored.search_documents(&kb_id, "test", limits),
                Err(AppError::BadRequest(_))
            ));
        }
    }

    #[test]
    fn anchored_knowledge_search_depth_limit_fails_closed() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("knowledge");
        let mut nested = root.clone();
        for _ in 0..=MAX_ANCHORED_KNOWLEDGE_DEPTH {
            nested.push("d");
        }
        std::fs::create_dir_all(&nested).unwrap();

        let anchored = AnchoredKnowledgeFs::open(&root).unwrap();
        assert!(matches!(
            anchored.search_documents(
                &KnowledgeBaseId::new(),
                "test",
                RetrievalLoadLimits {
                    max_entries: MAX_ANCHORED_KNOWLEDGE_DEPTH + 2,
                    ..limits()
                },
            ),
            Err(AppError::BadRequest(_))
        ));
    }
}
