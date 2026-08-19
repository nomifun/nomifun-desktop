//! `MiniAppService`: owner-scoped mini-app CRUD plus the one unscoped read the
//! serve route needs, and the two layers the publish flow crosses.
//!
//! All validation lives here as pure functions so the limits are testable
//! without a database and identical on create, update and publish. The service is
//! the only writer of the `miniapps` table, so a document that reached storage
//! satisfies every bound below.
//!
//! **The published snapshot and the working copy.** `miniapps.html` is the
//! snapshot `/serve` hands to an iframe; `{work_dir}/miniapps/{id}/miniapp.html`
//! is the working copy an editing conversation rewrites in place. The working copy
//! is materialized lazily by [`MiniAppService::ensure_workspace`] and crosses back
//! into the snapshot only through [`MiniAppService::publish`]. Every open of
//! either goes through [`MiniAppService::resolve_within_miniapps`], and no
//! handler is allowed to join a path itself.
//!
//! Nothing here creates, opens or deletes a conversation: a thread that edits a
//! mini-app is an ordinary conversation in an ordinary workspace that was merely
//! *told* the absolute path [`MiniAppService::provision_workspace`] returns.
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use nomifun_common::miniapp_workspace::{MINIAPP_SOURCE_FILE, miniapps_root};
use nomifun_common::{ConversationId, MiniAppId, now_ms};
use nomifun_db::{CreateMiniAppParams, IMiniAppRepository, MiniAppRow, UpdateMiniAppParams};

use crate::dto::{
    CreateMiniAppRequest, MiniAppResponse, MiniAppWorkspaceResponse, UpdateMiniAppRequest,
};
use crate::fsio;

/// Longest display name. A name is a card label, not a description.
pub const MINI_APP_NAME_MAX_CHARS: usize = 100;
/// Longest description. Two lines of card subtitle.
pub const MINI_APP_DESCRIPTION_MAX_CHARS: usize = 500;
/// Longest icon. An emoji (which can be several chars once modifiers and
/// zero-width joiners are counted) or a two-or-three letter monogram — never a
/// sentence smuggled into the grid.
pub const MINI_APP_ICON_MAX_CHARS: usize = 16;
/// Largest storable document. Generous for one self-contained page (CDN links
/// keep libraries out of the body), and small enough that a runaway generation
/// cannot fill the database one row at a time.
pub const MINI_APP_HTML_MAX_BYTES: usize = 4 * 1024 * 1024;

/// Errors surfaced by the mini-app service.
#[derive(Debug, thiserror::Error)]
pub enum MiniAppServiceError {
    #[error("mini-app not found")]
    NotFound,
    #[error("invalid request: {0}")]
    BadRequest(String),
    #[error("internal error: {0}")]
    Internal(String),
}

/// Mini-app service. Cheap to clone (`Arc` internals).
#[derive(Clone)]
pub struct MiniAppService {
    /// The resolved work root — the *same* value `ConversationService` uses as
    /// its `workspace_root`, threaded in from `AppServices` rather than resolved
    /// again here. Two independent resolutions would drift the moment a user
    /// relocated their work dir, and the path this service hands out has to be
    /// the path the model then opens.
    pub(crate) work_dir: PathBuf,
    pub(crate) repo: Arc<dyn IMiniAppRepository>,
}

impl MiniAppService {
    pub fn new(work_dir: PathBuf, repo: Arc<dyn IMiniAppRepository>) -> Self {
        Self { work_dir, repo }
    }

    /// The owner's apps, most recently updated first.
    pub async fn list(&self, user_id: &str) -> Result<Vec<MiniAppResponse>, MiniAppServiceError> {
        let rows = self
            .repo
            .list(user_id)
            .await
            .map_err(|e| MiniAppServiceError::Internal(e.to_string()))?;
        // One `stat` per app, not one document read: the library grid must be
        // able to badge "有未发布改动" on a card, and a per-row metadata probe is
        // the cheapest truth available (the flag cannot be stored — see
        // `MiniAppResponse::has_unpublished_changes`).
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(self.project(row).await);
        }
        Ok(out)
    }

    /// One owned app.
    pub async fn get(
        &self,
        user_id: &str,
        id: &MiniAppId,
    ) -> Result<MiniAppResponse, MiniAppServiceError> {
        let row = self.require_owned(user_id, id).await?;
        Ok(self.project(row).await)
    }

    /// Solidify a new app for `user_id`.
    pub async fn create(
        &self,
        user_id: &str,
        req: CreateMiniAppRequest,
    ) -> Result<MiniAppResponse, MiniAppServiceError> {
        let name = validate_name(&req.name)?;
        let description = validate_description(req.description.as_deref().unwrap_or_default())?;
        let icon = validate_icon(req.icon.as_deref().unwrap_or_default())?;
        let html = validate_html(&req.html)?;
        let source_conversation_id = req
            .source_conversation_id
            .as_deref()
            .map(validate_source_conversation_id)
            .transpose()?;

        let row = self
            .repo
            .create(
                user_id,
                CreateMiniAppParams {
                    name: &name,
                    description: &description,
                    icon: icon.as_deref(),
                    html,
                    source_conversation_id: source_conversation_id.as_ref().map(|id| id.as_str()),
                },
            )
            .await
            .map_err(|e| MiniAppServiceError::Internal(e.to_string()))?;
        // A brand-new app has no directory yet, so the flag is false by
        // derivation rather than by assumption.
        Ok(self.project(row).await)
    }

    /// Edit an owned app. At least one field must be present.
    pub async fn update(
        &self,
        user_id: &str,
        id: &MiniAppId,
        req: UpdateMiniAppRequest,
    ) -> Result<MiniAppResponse, MiniAppServiceError> {
        if req.is_empty() {
            return Err(MiniAppServiceError::BadRequest(
                "an update must change at least one field".into(),
            ));
        }
        let name = req.name.as_deref().map(validate_name).transpose()?;
        let description = req
            .description
            .as_deref()
            .map(validate_description)
            .transpose()?;
        // `Some(None)` clears the icon; the DTO's empty string is how a client
        // says "remove it", and `validate_icon` already normalizes that to None.
        let icon = req.icon.as_deref().map(validate_icon).transpose()?;
        let html = req.html.as_deref().map(validate_html).transpose()?;

        let mut row = self
            .repo
            .update(
                user_id,
                id,
                UpdateMiniAppParams {
                    name: name.as_deref(),
                    description: description.as_deref(),
                    icon: icon.as_ref().map(|value| value.as_deref()),
                    html,
                    // A metadata edit is not a publish, so a rename must not make
                    // an unpublished working copy look published. Writing a *body*
                    // through here IS one, though: the preview panel's
                    // 「发布为小程序」 → 「替换已有小程序」 replaces the served snapshot by
                    // this route. Leaving the stamp alone for that case would let a
                    // later 「发布」 promote an older working copy straight over the
                    // document the user just published.
                    published_at: html.map(|_| now_ms()),
                },
            )
            .await
            .map_err(map_not_found)?;
        // Same reason, second half: the two layers must not silently diverge. A
        // body written into the snapshot has to reach the working copy too, or the
        // next iteration turn would edit a document the runner no longer serves.
        if let Some(document) = html {
            if let Some(synced) = self.resync_working_copy(user_id, id, document).await {
                row = synced;
            }
        }
        Ok(self.project(row).await)
    }

    /// Delete an owned app and its working directory.
    ///
    /// The row goes first and alone: it is the fact the user asked to change, and
    /// a cleanup failure must not leave a listed app whose source has already been
    /// removed. Removing the directory afterwards is best-effort and idempotent —
    /// `{work_dir}/miniapps/{id}/` is deliberately outside
    /// `MANAGED_DATASET_ROOTS`, so no other sweep will ever reach it.
    ///
    /// Conversations are deliberately untouched. A thread that edited this app is
    /// an ordinary conversation the user may still want to read; deleting the app
    /// only takes the app's own artifact with it.
    pub async fn delete(&self, user_id: &str, id: &MiniAppId) -> Result<(), MiniAppServiceError> {
        let row = self.require_owned(user_id, id).await?;
        let dir = self.workspace_dir(id.as_str());
        self.repo.delete(user_id, id).await.map_err(map_not_found)?;

        match dir {
            Ok(dir) => {
                // The guard already proved containment; this refuses the one path
                // it cannot distinguish — the root itself.
                if dir == miniapps_root(&self.work_dir) {
                    tracing::error!(
                        miniapp_id = %row.miniapp_id,
                        "refusing to remove the mini-app root as an app directory"
                    );
                } else if let Err(e) = tokio::fs::remove_dir_all(&dir).await {
                    if e.kind() != std::io::ErrorKind::NotFound {
                        tracing::warn!(
                            miniapp_id = %row.miniapp_id,
                            path = %dir.display(),
                            error = %e,
                            "deleted a mini-app but could not remove its working directory"
                        );
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    miniapp_id = %row.miniapp_id,
                    error = %e,
                    "deleted a mini-app but its working directory is not resolvable, so it was left in place"
                );
            }
        }
        Ok(())
    }

    /// The stored HTML document, for the auth-exempt serve route.
    ///
    /// Unscoped by necessity: an iframe subresource load presents no credentials,
    /// so there is no owner to compare against. The id is an unguessable bare
    /// UUIDv7 and the repository read returns the body alone, so a caller holding
    /// a link learns the document and nothing about who owns it.
    ///
    /// Reads the *snapshot*, never the working copy: nothing serializes the
    /// editors that touch the working copy (`bash`'s `> miniapp.html` truncates
    /// before it writes, and a future engine brings its own writer), so serving
    /// from disk could hand out half a document. A snapshot we wrote ourselves
    /// cannot be spliced.
    pub async fn serve_html(&self, id: &MiniAppId) -> Result<String, MiniAppServiceError> {
        self.repo
            .find_by_id_any_owner(id)
            .await
            .map_err(|e| MiniAppServiceError::Internal(e.to_string()))?
            .map(|document| document.html)
            .ok_or(MiniAppServiceError::NotFound)
    }

    /// Materialize the app's private workspace and return its directory.
    ///
    /// Existence-driven, idempotent and crash-safe, in the shape
    /// `nomifun-companion`'s `rehome_unowned_skill_dirs` established: the
    /// predicate is "is the working copy on disk", never "has a flag been set",
    /// so re-running this is a no-op, a killed process self-heals on the next
    /// call, and a user who relocates their work dir gets a fresh materialization
    /// instead of a dangling path.
    ///
    /// Called by the workspace, import and publish flows before they need the
    /// directory — never by `serve`, never at boot. A boot sweep would read
    /// every body and write every file on every start, and could not be
    /// fail-closed without letting a full disk brick the app over a cache warm-up.
    ///
    /// When it does write the working copy it also stamps `published_at`: the two
    /// layers are byte-identical at that instant, so the app must not report
    /// unpublished changes for a file the user has never touched.
    pub async fn ensure_workspace(
        &self,
        user_id: &str,
        id: &MiniAppId,
    ) -> Result<PathBuf, MiniAppServiceError> {
        // Owner-scoped first: an id belonging to somebody else must not be able
        // to create a directory, let alone learn a body.
        self.require_owned(user_id, id).await?;
        let dir = self.workspace_dir(id.as_str())?;
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| MiniAppServiceError::Internal(format!("create mini-app workspace: {e}")))?;
        let source = self.source_path(id.as_str())?;
        if fsio::modified_ms_opt(&source)
            .await
            .map_err(|e| MiniAppServiceError::Internal(format!("stat working copy: {e}")))?
            .is_some()
        {
            return Ok(dir);
        }

        let html = self.snapshot_html(id).await?;
        fsio::save_bytes_atomic(&dir, MINIAPP_SOURCE_FILE, html.as_bytes())
            .await
            .map_err(|e| MiniAppServiceError::Internal(format!("write working copy: {e}")))?;
        // Stamp AFTER the write, from the file's own mtime, so the derivation
        // (`mtime > published_at`) cannot come out true for a copy nobody edited.
        // Stamping first would race the write's own clock: the file lands a
        // millisecond later and the app would open claiming unpublished changes.
        let published_at = fsio::modified_ms_opt(&source)
            .await
            .map_err(|e| MiniAppServiceError::Internal(format!("stat working copy: {e}")))?
            .unwrap_or_else(now_ms);
        // A crash between the write and this stamp leaves the badge showing until
        // the next publish — cosmetic, and strictly safer than the reverse order,
        // which would hide real unpublished work.
        self.repo
            .mark_published_at(user_id, id, published_at)
            .await
            .map_err(map_not_found)?;
        Ok(dir)
    }

    /// [`Self::ensure_workspace`], answered as the absolute path of the working
    /// copy — the shape `POST /api/miniapps/{id}/workspace` returns.
    ///
    /// This is the whole server side of 「继续迭代」: the client asks where the source
    /// is, gets an absolute path, and writes it into the first message of an
    /// ORDINARY conversation. No conversation is created here, and none is
    /// remembered — the app owns its artifact, the thread that edits it owns
    /// nothing.
    ///
    /// The path is absolute because its reader is a model running in some other
    /// conversation's workspace, where a relative path names nothing. It is
    /// produced by the same guarded derivation every read and write in this crate
    /// uses, so the client can neither choose it nor widen it.
    pub async fn provision_workspace(
        &self,
        user_id: &str,
        id: &MiniAppId,
    ) -> Result<MiniAppWorkspaceResponse, MiniAppServiceError> {
        // Ownership, the directory and the working copy — all three are this call.
        self.ensure_workspace(user_id, id).await?;
        let source = self.source_path(id.as_str())?;
        // Fails closed rather than lossily: a path the client cannot round-trip is
        // a path the model would be told to open and not find.
        let source_path = source.to_str().map(str::to_owned).ok_or_else(|| {
            MiniAppServiceError::Internal(format!(
                "mini-app source {} is not valid UTF-8, so it cannot be named to a client",
                source.display()
            ))
        })?;
        Ok(MiniAppWorkspaceResponse { source_path })
    }

    /// Publish the working copy: disk → snapshot, in one owner-scoped statement.
    ///
    /// Validated with exactly the rules `create`/`update` apply to a client-sent
    /// body, plus the document shape `import` demands, because after this the
    /// snapshot is what `/serve` streams into an iframe and the two paths must not
    /// be able to store different things.
    ///
    /// The publish instant is the mtime of the bytes that were read, NOT `now_ms()`:
    /// a stamp later than the bytes it describes would mark a write that landed
    /// during the read as already published, hiding the user's newest change behind
    /// a badge that never lights up again. Reading is bracketed by two `stat`s for
    /// the same reason — nothing serializes the editors that touch the working copy
    /// (`bash`'s `> miniapp.html` truncates before it writes), so a read that
    /// straddles a write would promote half a document over a working app.
    pub async fn publish(
        &self,
        user_id: &str,
        id: &MiniAppId,
    ) -> Result<MiniAppResponse, MiniAppServiceError> {
        // Prove ownership before touching the filesystem: otherwise a stranger's
        // id would reveal whether a working copy exists.
        self.require_owned(user_id, id).await?;
        let source = self.source_path(id.as_str())?;
        let nothing_to_publish = || {
            MiniAppServiceError::BadRequest(
                "this mini-app has no working copy yet, so there is nothing to publish; \
                 iterate on it first"
                    .into(),
            )
        };
        let mtime_before = fsio::modified_ms_opt(&source)
            .await
            .map_err(|e| MiniAppServiceError::Internal(format!("stat working copy: {e}")))?
            .ok_or_else(nothing_to_publish)?;
        let bytes = fsio::read_bytes_opt(&source)
            .await
            .map_err(|e| MiniAppServiceError::Internal(format!("read working copy: {e}")))?
            .ok_or_else(nothing_to_publish)?;
        let mtime_after = fsio::modified_ms_opt(&source)
            .await
            .map_err(|e| MiniAppServiceError::Internal(format!("stat working copy: {e}")))?;
        if mtime_after != Some(mtime_before) {
            return Err(MiniAppServiceError::BadRequest(
                "the working copy was being written while it was read, so it may be \
                 half a document; wait for the current turn to finish and publish again"
                    .into(),
            ));
        }
        let html = String::from_utf8(bytes).map_err(|_| {
            MiniAppServiceError::BadRequest(
                "the working copy is not valid UTF-8; the document must be a UTF-8 HTML file".into(),
            )
        })?;
        let html = validate_html(&html)?;
        // The same shape gate every import runs. Without it any non-blank blob —
        // a plan, a notes file, a stack trace an errored turn left behind — would
        // replace a working app, and there is no previous snapshot to fall back to.
        if !crate::validation::looks_like_html_document(html) {
            return Err(MiniAppServiceError::BadRequest(
                "the working copy does not look like an HTML document, so publishing it \
                 would replace the running app with something that cannot render; ask the \
                 session to finish writing it first"
                    .into(),
            ));
        }

        let row = self
            .repo
            .update(
                user_id,
                id,
                UpdateMiniAppParams {
                    name: None,
                    description: None,
                    icon: None,
                    // One statement writes the body, its size and the publish
                    // instant. Two statements could leave a published body with a
                    // stale timestamp, which reads back as "still unpublished".
                    html: Some(html),
                    published_at: Some(mtime_before),
                },
            )
            .await
            .map_err(map_not_found)?;
        Ok(self.project(row).await)
    }

    /// Rewrite the working copy from a body that was written straight into the
    /// snapshot, and re-stamp the publish instant from the file that landed.
    ///
    /// Best-effort and lazily scoped: an app with no working copy keeps having
    /// none (`ensure_workspace` will materialize this very document on the first
    /// 「继续迭代」), and a failure leaves `published_at` at the row's own stamp,
    /// which is *newer* than the stale file — so the badge stays down and the
    /// newer snapshot cannot be overwritten by an older working copy. Returns the
    /// re-stamped row so the caller's response does not report the publish state
    /// from before this ran.
    async fn resync_working_copy(
        &self,
        user_id: &str,
        id: &MiniAppId,
        document: &str,
    ) -> Option<MiniAppRow> {
        let dir = self.workspace_dir(id.as_str()).ok()?;
        let source = self.source_path(id.as_str()).ok()?;
        if !matches!(fsio::modified_ms_opt(&source).await, Ok(Some(_))) {
            return None;
        }
        if let Err(e) = fsio::save_bytes_atomic(&dir, MINIAPP_SOURCE_FILE, document.as_bytes()).await
        {
            tracing::warn!(
                miniapp_id = %id.as_str(),
                error = %e,
                "wrote a mini-app snapshot but could not refresh its working copy"
            );
            return None;
        }
        let mtime = fsio::modified_ms_opt(&source).await.ok().flatten()?;
        self.repo.mark_published_at(user_id, id, mtime).await.ok()
    }

    /// `{work_dir}/miniapps/{miniapp_id}` — the app's private workspace, guarded.
    ///
    /// Public because callers outside this crate (the app's own e2e, a future
    /// maintenance surface) need the app's directory; every caller gets the
    /// guarded answer or an error, never a hand-joined path.
    pub fn workspace_dir(&self, miniapp_id: &str) -> Result<PathBuf, MiniAppServiceError> {
        self.resolve_within_miniapps(Path::new(miniapp_id))
    }

    /// `{work_dir}/miniapps/{miniapp_id}/miniapp.html` — the working copy, guarded.
    fn source_path(&self, miniapp_id: &str) -> Result<PathBuf, MiniAppServiceError> {
        self.resolve_within_miniapps(&Path::new(miniapp_id).join(MINIAPP_SOURCE_FILE))
    }

    /// Resolve a `miniapps`-root-relative path and guarantee it stays inside that
    /// root. **Every** read and write in this crate goes through here.
    ///
    /// Modelled on `nomifun-workshop`'s `resolve_within_workshop`: reject the
    /// obvious (`NUL`, `..`, an absolute or prefixed segment) up front, then
    /// canonicalize both sides and require containment — the pre-check catches
    /// spelling, canonicalization catches a symlink pointing out of the tree,
    /// which the pre-check cannot see.
    ///
    /// Unlike workshop's version this does not require the target to exist: the
    /// first write into a fresh app's directory has nothing to canonicalize yet.
    /// Instead each side is canonicalized as far as it exists and the
    /// yet-to-be-created tail is re-attached, so a work dir that is itself a
    /// symlink (`/var` → `/private/var` on macOS) compares equal instead of
    /// failing containment.
    ///
    /// Every path this crate resolves is built from an id that came from
    /// `Path<MiniAppId>` or a database row, so a bare lowercase UUIDv7 with no
    /// `.`, `/` or NUL — import mints a fresh id rather than reusing a file name.
    /// That makes the spelling checks defense in depth; the canonicalization is
    /// not. A working copy (or an `{id}` directory) replaced by a symlink pointing
    /// out of the tree is the one attack the spelling checks cannot see, and it is
    /// the reason `publish` refuses rather than following it.
    fn resolve_within_miniapps(&self, rel: &Path) -> Result<PathBuf, MiniAppServiceError> {
        let has_bad_component = rel.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        });
        if has_bad_component
            || rel.as_os_str().is_empty()
            || rel.to_string_lossy().contains('\0')
        {
            return Err(MiniAppServiceError::BadRequest(format!(
                "mini-app path {} is not a relative path inside the workspace",
                rel.display()
            )));
        }
        let root = partially_canonicalize(&miniapps_root(&self.work_dir))?;
        let target = partially_canonicalize(&miniapps_root(&self.work_dir).join(rel))?;
        if !target.starts_with(&root) {
            return Err(MiniAppServiceError::BadRequest(format!(
                "mini-app path {} escapes the mini-app workspace",
                rel.display()
            )));
        }
        Ok(target)
    }

    /// The owner-scoped row, or `NotFound` — the gate in front of every disk
    /// operation, so a cross-owner id can neither read nor create anything.
    pub(crate) async fn require_owned(
        &self,
        user_id: &str,
        id: &MiniAppId,
    ) -> Result<MiniAppRow, MiniAppServiceError> {
        self.repo
            .find(user_id, id)
            .await
            .map_err(|e| MiniAppServiceError::Internal(e.to_string()))?
            .ok_or(MiniAppServiceError::NotFound)
    }

    /// The published snapshot's body, for materializing a working copy.
    ///
    /// Uses the unscoped document read on purpose: ownership was already proven
    /// by the `require_owned` call that preceded it, and the metadata projection
    /// deliberately carries no body, so this is the only read that can supply
    /// one. It stays unscoped so `MiniAppDocumentRow` remains the single shape
    /// that can ever carry a document.
    async fn snapshot_html(&self, id: &MiniAppId) -> Result<String, MiniAppServiceError> {
        self.repo
            .find_by_id_any_owner(id)
            .await
            .map_err(|e| MiniAppServiceError::Internal(e.to_string()))?
            .map(|document| document.html)
            .ok_or(MiniAppServiceError::NotFound)
    }

    /// Row → response, measuring the one fact the row cannot know.
    ///
    /// The measurement is best-effort on purpose. The id came from the database,
    /// whose column CHECK proves a bare UUIDv7, so the guard cannot refuse it on
    /// spelling; the only ways this probe fails are a hand-planted symlink
    /// pointing out of the tree or an unreadable directory. Neither may 500 a
    /// library listing over a badge — and the honest badge for a working copy we
    /// will not open is "there is none". `publish` reads the same path without
    /// this tolerance, so the guard still refuses to promote such a file.
    async fn project(&self, row: MiniAppRow) -> MiniAppResponse {
        let mtime = match self.source_path(&row.miniapp_id) {
            Ok(source) => match fsio::modified_ms_opt(&source).await {
                Ok(mtime) => mtime,
                Err(e) => {
                    tracing::warn!(
                        miniapp_id = %row.miniapp_id,
                        error = %e,
                        "cannot stat mini-app working copy; reporting no unpublished changes"
                    );
                    None
                }
            },
            Err(e) => {
                tracing::warn!(
                    miniapp_id = %row.miniapp_id,
                    error = %e,
                    "mini-app working copy is not resolvable inside the workspace; reporting no unpublished changes"
                );
                None
            }
        };
        let unpublished = has_unpublished_changes(mtime, row.published_at);
        MiniAppResponse::new(row, unpublished)
    }
}

/// Canonicalize `path` as far as it exists, then re-attach the components that do
/// not yet.
///
/// Needed because a containment check on a path we are about to *create* has
/// nothing to canonicalize, while comparing two uncanonicalized paths would
/// wrongly reject a work dir reached through a symlink. Canonicalizing the
/// deepest existing ancestor is what resolves the symlink; the tail is pure
/// spelling and has already been checked for `..`.
fn partially_canonicalize(path: &Path) -> Result<PathBuf, MiniAppServiceError> {
    match std::fs::canonicalize(path) {
        Ok(canonical) => Ok(canonical),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
                // No ancestor left to resolve: nothing exists, so the literal
                // spelling is the best answer available.
                return Ok(path.to_path_buf());
            };
            Ok(partially_canonicalize(parent)?.join(name))
        }
        Err(e) => Err(MiniAppServiceError::Internal(format!(
            "resolve {}: {e}",
            path.display()
        ))),
    }
}

/// Whether the working copy is newer than what `/serve` is handing out.
///
/// `None` mtime means there is no working copy at all — nothing has been iterated
/// on, so there is nothing unpublished, and a badge here would be pure noise.
///
/// A working copy with NO `published_at` counts as unpublished. Every path that
/// writes the file also stamps the column (`ensure_workspace` from the file's own
/// mtime, `publish` and a body-bearing `update` from theirs), so this state means
/// one of those stamps failed — and `ensure_workspace` says in as many words that
/// it prefers leaving the badge up to hiding real work. It must NOT fall back to
/// `updated_at`: that column moves on a plain rename, which would let renaming an
/// app retire the only affordance for publishing its iterated document.
pub(crate) fn has_unpublished_changes(
    working_copy_mtime_ms: Option<i64>,
    published_at: Option<i64>,
) -> bool {
    match (working_copy_mtime_ms, published_at) {
        (None, _) => false,
        (Some(_), None) => true,
        // Strictly greater: materializing the working copy stamps `published_at`
        // from that very file's mtime, so equal timestamps mean "identical", not
        // "changed".
        (Some(mtime), Some(published)) => mtime > published,
    }
}

/// Trim, then require a non-empty name within the length cap.
pub(crate) fn validate_name(raw: &str) -> Result<String, MiniAppServiceError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(MiniAppServiceError::BadRequest("name is required".into()));
    }
    if name.chars().count() > MINI_APP_NAME_MAX_CHARS {
        return Err(MiniAppServiceError::BadRequest(format!(
            "name must be at most {MINI_APP_NAME_MAX_CHARS} characters"
        )));
    }
    Ok(name.to_string())
}

/// Trim; an absent description is the empty string, not NULL (the column has a
/// `''` default, so "no description" has one spelling).
pub(crate) fn validate_description(raw: &str) -> Result<String, MiniAppServiceError> {
    let description = raw.trim();
    if description.chars().count() > MINI_APP_DESCRIPTION_MAX_CHARS {
        return Err(MiniAppServiceError::BadRequest(format!(
            "description must be at most {MINI_APP_DESCRIPTION_MAX_CHARS} characters"
        )));
    }
    Ok(description.to_string())
}

/// Trim; an empty icon is `None` so the grid falls back to its default glyph
/// instead of rendering a blank box.
pub(crate) fn validate_icon(raw: &str) -> Result<Option<String>, MiniAppServiceError> {
    let icon = raw.trim();
    if icon.is_empty() {
        return Ok(None);
    }
    if icon.chars().count() > MINI_APP_ICON_MAX_CHARS {
        return Err(MiniAppServiceError::BadRequest(format!(
            "icon must be at most {MINI_APP_ICON_MAX_CHARS} characters"
        )));
    }
    Ok(Some(icon.to_string()))
}

/// Require a non-blank document within the size cap, and store it verbatim.
///
/// The body is NOT trimmed: leading whitespace inside a `<pre>` is the author's,
/// and the byte length reported back to the client must be the length of what was
/// stored.
pub(crate) fn validate_html(raw: &str) -> Result<&str, MiniAppServiceError> {
    if raw.trim().is_empty() {
        return Err(MiniAppServiceError::BadRequest("html is required".into()));
    }
    if raw.len() > MINI_APP_HTML_MAX_BYTES {
        return Err(MiniAppServiceError::BadRequest(format!(
            "html must be at most {MINI_APP_HTML_MAX_BYTES} bytes"
        )));
    }
    Ok(raw)
}

/// A supplied provenance id must be a canonical bare UUIDv7 — the column's CHECK
/// enforces the same thing, and failing here names the field instead of surfacing
/// a constraint violation as an internal error.
pub(crate) fn validate_source_conversation_id(
    raw: &str,
) -> Result<ConversationId, MiniAppServiceError> {
    ConversationId::parse(raw).map_err(|e| {
        MiniAppServiceError::BadRequest(format!("source_conversation_id is invalid: {e}"))
    })
}

pub(crate) fn map_not_found(e: nomifun_db::DbError) -> MiniAppServiceError {
    match e {
        nomifun_db::DbError::NotFound(_) => MiniAppServiceError::NotFound,
        other => MiniAppServiceError::Internal(other.to_string()),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use nomifun_common::miniapp_workspace::miniapp_workspace_dir;

    fn message(result: Result<impl std::fmt::Debug, MiniAppServiceError>) -> String {
        match result {
            Err(MiniAppServiceError::BadRequest(m)) => m,
            other => panic!("expected a bad-request rejection, got {other:?}"),
        }
    }

    #[test]
    fn name_is_trimmed_required_and_capped() {
        assert_eq!(validate_name("  Timer  ").unwrap(), "Timer");
        assert!(message(validate_name("   ")).contains("required"));
        // The cap counts characters, not bytes: 100 CJK names are common and a
        // byte cap would reject a third of them.
        let hundred = "小".repeat(MINI_APP_NAME_MAX_CHARS);
        assert_eq!(validate_name(&hundred).unwrap().chars().count(), 100);
        assert!(message(validate_name(&"小".repeat(MINI_APP_NAME_MAX_CHARS + 1))).contains("100"));
    }

    #[test]
    fn description_is_optional_trimmed_and_capped() {
        assert_eq!(validate_description("").unwrap(), "");
        assert_eq!(validate_description(" hi ").unwrap(), "hi");
        assert!(validate_description(&"a".repeat(MINI_APP_DESCRIPTION_MAX_CHARS)).is_ok());
        assert!(
            message(validate_description(&"a".repeat(MINI_APP_DESCRIPTION_MAX_CHARS + 1)))
                .contains("500")
        );
    }

    #[test]
    fn blank_icon_becomes_none_and_long_icon_is_refused() {
        assert_eq!(validate_icon("  ").unwrap(), None);
        assert_eq!(validate_icon(" ⏱ ").unwrap().as_deref(), Some("⏱"));
        assert!(message(validate_icon(&"x".repeat(MINI_APP_ICON_MAX_CHARS + 1))).contains("16"));
    }

    #[test]
    fn html_is_required_stored_verbatim_and_size_capped() {
        assert!(message(validate_html("   \n ")).contains("required"));
        // Verbatim: whitespace inside the document is the author's, and the byte
        // length the client is told must be the length of what was stored.
        assert_eq!(validate_html("\n<h1>hi</h1>\n").unwrap(), "\n<h1>hi</h1>\n");
        let at_cap = "a".repeat(MINI_APP_HTML_MAX_BYTES);
        assert!(validate_html(&at_cap).is_ok());
        let over_cap = "a".repeat(MINI_APP_HTML_MAX_BYTES + 1);
        assert!(message(validate_html(&over_cap)).contains("bytes"));
    }

    #[test]
    fn source_conversation_id_must_be_a_canonical_uuidv7() {
        let good = "0190f5fe-7c00-7a00-8000-000000000002";
        assert_eq!(
            validate_source_conversation_id(good).unwrap().as_str(),
            good
        );
        for bad in [
            "",
            "conv-1",
            // v4, not v7 — the column's CHECK would reject it as a constraint
            // violation, which reaches the client as a 500 rather than a field error.
            "9f1b5c62-2f3a-4b19-9c1e-2f4d6a8b0c11",
            // Uppercase is not canonical here even though it parses as a UUID.
            "0190F5FE-7C00-7A00-8000-000000000002",
        ] {
            assert!(
                validate_source_conversation_id(bad).is_err(),
                "{bad} must be refused"
            );
        }
    }

    // ---------------------------------------------------------------------
    // The publish-state derivation. Pure, so every corner is cheap to pin.
    // ---------------------------------------------------------------------

    #[test]
    fn no_working_copy_means_nothing_is_unpublished() {
        // Every app that has never been iterated on is in this state, so a badge
        // here would fire on the whole library.
        assert!(!has_unpublished_changes(None, None));
        assert!(!has_unpublished_changes(None, Some(500)));
    }

    #[test]
    fn a_working_copy_newer_than_the_publish_instant_is_unpublished() {
        assert!(has_unpublished_changes(Some(1_001), Some(1_000)));
        assert!(!has_unpublished_changes(Some(999), Some(1_000)));
        // Equal is NOT newer: materializing the working copy stamps `published_at`
        // from that file's own mtime, and the two are byte-identical then.
        assert!(!has_unpublished_changes(Some(1_000), Some(1_000)));
    }

    #[test]
    fn a_working_copy_with_no_publish_instant_is_unpublished() {
        // Reachable only when a stamp failed after the file landed. Showing the
        // badge is the safe answer, and it must not depend on `updated_at` — a
        // rename moves that column and would retire the publish affordance.
        assert!(has_unpublished_changes(Some(1), None));
        assert!(has_unpublished_changes(Some(i64::MAX), None));
    }

    // ---------------------------------------------------------------------
    // Workspace lifecycle + publish, against an in-memory repository. No
    // database and no router: what is under test is the two-layer storage rule,
    // not SQL.
    // ---------------------------------------------------------------------

    pub(crate) const OWNER: &str = "0190f5fe-7c00-7a00-8000-0000000000ff";
    pub(crate) const STRANGER: &str = "0190f5fe-7c00-7a00-8000-0000000000ee";

    /// The smallest `IMiniAppRepository` that still keeps the invariants the
    /// service leans on: owner scoping, `html_size` maintained by the writer,
    /// `COALESCE` update semantics, and `mark_published_at` touching nothing else.
    #[derive(Default)]
    pub(crate) struct FakeRepo {
        rows: std::sync::Mutex<Vec<(MiniAppRow, String)>>,
    }

    impl FakeRepo {
        fn find_index(&self, id: &str) -> Option<usize> {
            self.rows
                .lock()
                .unwrap()
                .iter()
                .position(|(row, _)| row.miniapp_id == id)
        }
    }

    #[async_trait::async_trait]
    impl IMiniAppRepository for FakeRepo {
        async fn list(&self, user_id: &str) -> Result<Vec<MiniAppRow>, nomifun_db::DbError> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .iter()
                .filter(|(row, _)| row.user_id == user_id)
                .map(|(row, _)| row.clone())
                .collect())
        }

        async fn find(
            &self,
            user_id: &str,
            id: &MiniAppId,
        ) -> Result<Option<MiniAppRow>, nomifun_db::DbError> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .iter()
                .find(|(row, _)| row.miniapp_id == id.as_str() && row.user_id == user_id)
                .map(|(row, _)| row.clone()))
        }

        async fn find_by_id_any_owner(
            &self,
            id: &MiniAppId,
        ) -> Result<Option<nomifun_db::MiniAppDocumentRow>, nomifun_db::DbError> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .iter()
                .find(|(row, _)| row.miniapp_id == id.as_str())
                .map(|(_, html)| nomifun_db::MiniAppDocumentRow { html: html.clone() }))
        }

        async fn create(
            &self,
            user_id: &str,
            params: CreateMiniAppParams<'_>,
        ) -> Result<MiniAppRow, nomifun_db::DbError> {
            let now = now_ms();
            let row = MiniAppRow {
                id: self.rows.lock().unwrap().len() as i64 + 1,
                miniapp_id: MiniAppId::new().as_str().to_string(),
                user_id: user_id.to_string(),
                name: params.name.to_string(),
                description: params.description.to_string(),
                icon: params.icon.map(str::to_string),
                source_conversation_id: params.source_conversation_id.map(str::to_string),
                html_size: params.html.len() as i64,
                published_at: None,
                created_at: now,
                updated_at: now,
            };
            self.rows
                .lock()
                .unwrap()
                .push((row.clone(), params.html.to_string()));
            Ok(row)
        }

        async fn update(
            &self,
            user_id: &str,
            id: &MiniAppId,
            params: UpdateMiniAppParams<'_>,
        ) -> Result<MiniAppRow, nomifun_db::DbError> {
            let index = self
                .find_index(id.as_str())
                .ok_or_else(|| nomifun_db::DbError::NotFound("miniapp".into()))?;
            let mut rows = self.rows.lock().unwrap();
            let (row, html) = &mut rows[index];
            if row.user_id != user_id {
                return Err(nomifun_db::DbError::NotFound("miniapp".into()));
            }
            if let Some(name) = params.name {
                row.name = name.to_string();
            }
            if let Some(description) = params.description {
                row.description = description.to_string();
            }
            if let Some(icon) = params.icon {
                row.icon = icon.map(str::to_string);
            }
            if let Some(body) = params.html {
                *html = body.to_string();
                row.html_size = body.len() as i64;
            }
            if let Some(published_at) = params.published_at {
                row.published_at = Some(published_at);
            }
            row.updated_at = now_ms();
            Ok(row.clone())
        }

        async fn delete(&self, user_id: &str, id: &MiniAppId) -> Result<(), nomifun_db::DbError> {
            let index = self
                .find_index(id.as_str())
                .ok_or_else(|| nomifun_db::DbError::NotFound("miniapp".into()))?;
            let mut rows = self.rows.lock().unwrap();
            if rows[index].0.user_id != user_id {
                return Err(nomifun_db::DbError::NotFound("miniapp".into()));
            }
            rows.remove(index);
            Ok(())
        }

        async fn mark_published_at(
            &self,
            user_id: &str,
            id: &MiniAppId,
            published_at: i64,
        ) -> Result<MiniAppRow, nomifun_db::DbError> {
            let index = self
                .find_index(id.as_str())
                .ok_or_else(|| nomifun_db::DbError::NotFound("miniapp".into()))?;
            let mut rows = self.rows.lock().unwrap();
            let (row, _) = &mut rows[index];
            if row.user_id != user_id {
                return Err(nomifun_db::DbError::NotFound("miniapp".into()));
            }
            row.published_at = Some(published_at);
            Ok(row.clone())
        }
    }

    pub(crate) struct Fixture {
        pub(crate) _root: tempfile::TempDir,
        pub(crate) service: MiniAppService,
        /// The same repository the service holds, so a test can plant the states
        /// only a partial failure produces (a working copy with no publish stamp).
        pub(crate) repo: Arc<FakeRepo>,
    }

    impl Fixture {
        pub(crate) fn new() -> Self {
            let root = tempfile::tempdir().expect("temp work dir");
            let repo = Arc::new(FakeRepo::default());
            let service = MiniAppService::new(root.path().to_path_buf(), repo.clone());
            Fixture {
                _root: root,
                service,
                repo,
            }
        }

        pub(crate) async fn solidify(&self, html: &str) -> MiniAppId {
            let created = self
                .service
                .create(
                    OWNER,
                    CreateMiniAppRequest {
                        name: "Timer".into(),
                        description: None,
                        icon: None,
                        html: html.to_string(),
                        source_conversation_id: None,
                    },
                )
                .await
                .expect("create");
            assert!(
                !created.has_unpublished_changes,
                "a brand-new app has no working copy, so nothing can be unpublished"
            );
            MiniAppId::parse(created.miniapp_id).expect("bare UUIDv7")
        }
    }

    /// The mtime the derivation compares has millisecond resolution, so a test
    /// that edits a file it just wrote must let the clock move first — otherwise
    /// it asserts on a coin flip.
    async fn advance_past_mtime_resolution() {
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    }

    #[tokio::test]
    async fn the_workspace_path_is_the_shared_formula() {
        let fixture = Fixture::new();
        let id = fixture.solidify("<p>hi</p>").await;
        // The guard must not invent a second layout: the shared formula in
        // `nomifun-common` is what every other reader of the tree (the app's e2e,
        // a relocation sweep) derives the path with.
        // `workspace_dir` resolves through the filesystem, which on Windows
        // returns a `\\?\` verbatim path, while the formula builds a plain one
        // for a directory that need not exist yet — so `canonicalize` cannot
        // normalize both sides. Compare with the prefix removed instead.
        let plain = |path: std::path::PathBuf| {
            let text = path.to_string_lossy().into_owned();
            std::path::PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned())
        };
        assert_eq!(
            plain(fixture.service.workspace_dir(id.as_str()).expect("resolve")),
            plain(miniapp_workspace_dir(fixture._root.path(), id.as_str()))
        );
    }

    #[tokio::test]
    async fn ensure_workspace_materializes_the_snapshot_and_claims_no_unpublished_changes() {
        let fixture = Fixture::new();
        let html = "<!doctype html><title>v1</title>";
        let id = fixture.solidify(html).await;

        let dir = fixture
            .service
            .ensure_workspace(OWNER, &id)
            .await
            .expect("ensure");
        let source = dir.join(MINIAPP_SOURCE_FILE);
        assert_eq!(std::fs::read_to_string(&source).unwrap(), html);

        // Byte-identical at this instant: the app must not open telling the user
        // they have changes to publish.
        let app = fixture.service.get(OWNER, &id).await.expect("get");
        assert!(
            !app.has_unpublished_changes,
            "a freshly materialized working copy is the published document"
        );
        assert!(
            app.published_at.is_some(),
            "materializing stamps the publish instant, or the flag reads back true"
        );
    }

    #[tokio::test]
    async fn ensure_workspace_is_idempotent_and_never_clobbers_the_working_copy() {
        let fixture = Fixture::new();
        let id = fixture.solidify("<!doctype html><title>v1</title>").await;
        let dir = fixture
            .service
            .ensure_workspace(OWNER, &id)
            .await
            .expect("first ensure");

        // Simulate an iteration turn, then re-enter the flow the way reopening the
        // detail page does.
        advance_past_mtime_resolution().await;
        let edited = "<!doctype html><title>edited by the session</title>";
        std::fs::write(dir.join(MINIAPP_SOURCE_FILE), edited).unwrap();

        let again = fixture
            .service
            .ensure_workspace(OWNER, &id)
            .await
            .expect("second ensure");
        assert_eq!(again, dir);
        assert_eq!(
            std::fs::read_to_string(dir.join(MINIAPP_SOURCE_FILE)).unwrap(),
            edited,
            "re-materializing would silently throw away the user's iteration"
        );
        assert!(
            fixture
                .service
                .get(OWNER, &id)
                .await
                .expect("get")
                .has_unpublished_changes,
            "an edited working copy must be reported as unpublished"
        );
    }

    #[tokio::test]
    async fn a_stranger_can_neither_materialize_nor_publish() {
        let fixture = Fixture::new();
        let id = fixture.solidify("<p>mine</p>").await;
        assert!(matches!(
            fixture.service.ensure_workspace(STRANGER, &id).await,
            Err(MiniAppServiceError::NotFound)
        ));
        assert!(matches!(
            fixture.service.publish(STRANGER, &id).await,
            Err(MiniAppServiceError::NotFound)
        ));
        // And nothing was created on the stranger's behalf.
        let dir = fixture.service.workspace_dir(id.as_str()).expect("resolve");
        assert!(!dir.exists(), "a non-owner must not be able to mkdir");
    }

    #[tokio::test]
    async fn publishing_without_a_working_copy_is_refused() {
        let fixture = Fixture::new();
        let id = fixture.solidify("<p>hi</p>").await;
        let text = message(fixture.service.publish(OWNER, &id).await);
        assert!(
            text.contains("nothing to publish"),
            "the message must say what to do about it, got {text}"
        );
    }

    #[tokio::test]
    async fn publish_validates_the_working_copy_exactly_like_create_does() {
        let fixture = Fixture::new();
        let id = fixture.solidify("<p>hi</p>").await;
        let dir = fixture
            .service
            .ensure_workspace(OWNER, &id)
            .await
            .expect("ensure");
        let source = dir.join(MINIAPP_SOURCE_FILE);

        // Blank: the same rejection `validate_html` gives a client-sent body, so a
        // model that truncated the file cannot publish an empty app.
        std::fs::write(&source, "   \n ").unwrap();
        assert!(message(fixture.service.publish(OWNER, &id).await).contains("required"));

        // Over the cap: same limit, same wording.
        std::fs::write(&source, "a".repeat(MINI_APP_HTML_MAX_BYTES + 1)).unwrap();
        let text = message(fixture.service.publish(OWNER, &id).await);
        assert!(text.contains("bytes"), "got {text}");

        // Not UTF-8: the column is TEXT and the serve route declares charset=utf-8,
        // so this has to be a named field error rather than a lossy conversion.
        std::fs::write(&source, [0x3c, 0x70, 0xff, 0x3e]).unwrap();
        let text = message(fixture.service.publish(OWNER, &id).await);
        assert!(text.contains("UTF-8"), "got {text}");

        // None of the failures touched the snapshot.
        assert_eq!(
            fixture.service.serve_html(&id).await.expect("serve"),
            "<p>hi</p>"
        );
    }

    #[tokio::test]
    async fn publish_promotes_the_working_copy_and_clears_the_flag() {
        let fixture = Fixture::new();
        let id = fixture.solidify("<!doctype html><title>v1</title>").await;
        let dir = fixture
            .service
            .ensure_workspace(OWNER, &id)
            .await
            .expect("ensure");
        advance_past_mtime_resolution().await;
        let next = "<!doctype html><title>v2</title>";
        std::fs::write(dir.join(MINIAPP_SOURCE_FILE), next).unwrap();
        assert!(
            fixture
                .service
                .get(OWNER, &id)
                .await
                .expect("get")
                .has_unpublished_changes
        );

        let published = fixture.service.publish(OWNER, &id).await.expect("publish");
        assert_eq!(published.html_size, next.len() as i64);
        assert!(
            !published.has_unpublished_changes,
            "the response must already reflect the publish it just performed"
        );
        assert_eq!(fixture.service.serve_html(&id).await.expect("serve"), next);
        // And a fresh read agrees — the flag is derived, not cached.
        assert!(
            !fixture
                .service
                .get(OWNER, &id)
                .await
                .expect("get")
                .has_unpublished_changes
        );
    }

    /// Publishing must not claim a later instant than the bytes it published.
    ///
    /// Stamping `now_ms()` would mark a write that landed during the publish as
    /// already published: its mtime would be *older* than the stamp, the badge
    /// would stay down, and the user's newest change would be neither served nor
    /// offered for publishing until some later edit happened to pass the stamp.
    #[tokio::test]
    async fn publish_stamps_the_mtime_of_the_bytes_it_read_not_the_wall_clock() {
        let fixture = Fixture::new();
        let id = fixture.solidify("<!doctype html><title>v1</title>").await;
        let dir = fixture
            .service
            .ensure_workspace(OWNER, &id)
            .await
            .expect("ensure");
        let source = dir.join(MINIAPP_SOURCE_FILE);
        advance_past_mtime_resolution().await;
        std::fs::write(&source, "<!doctype html><title>v2</title>").unwrap();

        let published = fixture.service.publish(OWNER, &id).await.expect("publish");
        let stamped = published.published_at.expect("publish stamps an instant");
        let mtime = fsio::modified_ms_opt(&source)
            .await
            .expect("stat")
            .expect("the working copy exists");
        assert_eq!(
            stamped, mtime,
            "the publish instant must be the mtime of the published bytes"
        );

        // The property that stamp buys: a write landing after the read is still
        // reported as unpublished rather than swallowed.
        advance_past_mtime_resolution().await;
        std::fs::write(&source, "<!doctype html><title>v3</title>").unwrap();
        assert!(
            fixture
                .service
                .get(OWNER, &id)
                .await
                .expect("get")
                .has_unpublished_changes,
            "a change made after a publish must never be hidden"
        );
    }

    /// The working copy is written by an agent, so "non-blank UTF-8 under the cap"
    /// does not prove it is a page. Without this gate an errored turn's notes file
    /// would replace a working app, and there is no previous snapshot to restore.
    #[tokio::test]
    async fn publish_refuses_a_body_that_is_not_a_document() {
        let fixture = Fixture::new();
        let id = fixture.solidify("<!doctype html><title>works</title>").await;
        let dir = fixture
            .service
            .ensure_workspace(OWNER, &id)
            .await
            .expect("ensure");
        let source = dir.join(MINIAPP_SOURCE_FILE);
        advance_past_mtime_resolution().await;

        for not_a_document in [
            "TODO: ask the user which timezone they meant",
            "{\"plan\":[\"read the file\",\"rewrite it\"]}",
            "Error: the model ran out of context",
        ] {
            std::fs::write(&source, not_a_document).unwrap();
            let text = message(fixture.service.publish(OWNER, &id).await);
            assert!(
                text.contains("does not look like an HTML document"),
                "{not_a_document:?} must not be publishable, got {text}"
            );
        }
        // The app the user has been running is untouched by every refusal.
        assert_eq!(
            fixture.service.serve_html(&id).await.expect("serve"),
            "<!doctype html><title>works</title>"
        );
    }

    /// The preview panel's 「发布为小程序」 → 「替换已有小程序」 writes a body through
    /// `update`. That IS a publish, so it must stamp — otherwise the runner keeps
    /// offering 「发布」 and one click promotes an older working copy over the
    /// document the user just published.
    #[tokio::test]
    async fn writing_a_body_through_update_publishes_and_cannot_be_reverted_by_a_later_publish() {
        let fixture = Fixture::new();
        let id = fixture.solidify("<p>v1</p>").await;
        let dir = fixture
            .service
            .ensure_workspace(OWNER, &id)
            .await
            .expect("ensure");
        advance_past_mtime_resolution().await;
        // An iteration turn: the working copy is now ahead of the snapshot.
        std::fs::write(dir.join(MINIAPP_SOURCE_FILE), "<p>working copy</p>").unwrap();
        assert!(
            fixture
                .service
                .get(OWNER, &id)
                .await
                .expect("get")
                .has_unpublished_changes
        );

        advance_past_mtime_resolution().await;
        let solidified = fixture
            .service
            .update(
                OWNER,
                &id,
                UpdateMiniAppRequest {
                    html: Some("<p>solidified</p>".into()),
                    ..Default::default()
                },
            )
            .await
            .expect("solidify update");

        assert_eq!(
            fixture.service.serve_html(&id).await.expect("serve"),
            "<p>solidified</p>"
        );
        assert!(
            !solidified.has_unpublished_changes,
            "the snapshot is the newest document, so nothing is pending"
        );
        // The two layers agree, so the next iteration turn starts from what the
        // runner serves rather than editing an abandoned document.
        assert_eq!(
            std::fs::read_to_string(dir.join(MINIAPP_SOURCE_FILE)).unwrap(),
            "<p>solidified</p>"
        );
        // And a fresh read agrees: the publish affordance is gone, so the older
        // working copy cannot be promoted over what was just written.
        assert!(
            !fixture
                .service
                .get(OWNER, &id)
                .await
                .expect("get")
                .has_unpublished_changes
        );
    }

    /// A rename must never retire the publish affordance. Reachable when
    /// `ensure_workspace`'s stamp failed after the file landed, which leaves
    /// `published_at` NULL with a real working copy on disk.
    #[tokio::test]
    async fn renaming_an_app_cannot_hide_an_unpublished_working_copy() {
        let fixture = Fixture::new();
        let id = fixture.solidify("<p>v1</p>").await;
        let dir = fixture
            .service
            .ensure_workspace(OWNER, &id)
            .await
            .expect("ensure");
        advance_past_mtime_resolution().await;
        std::fs::write(dir.join(MINIAPP_SOURCE_FILE), "<p>v2</p>").unwrap();
        // Simulate the failed stamp the ensure path documents as "cosmetic".
        {
            let mut rows = fixture.repo.rows.lock().unwrap();
            rows[0].0.published_at = None;
        }
        assert!(
            fixture
                .service
                .get(OWNER, &id)
                .await
                .expect("get")
                .has_unpublished_changes
        );

        advance_past_mtime_resolution().await;
        let renamed = fixture
            .service
            .update(
                OWNER,
                &id,
                UpdateMiniAppRequest {
                    name: Some("Renamed".into()),
                    ..Default::default()
                },
            )
            .await
            .expect("rename");
        assert!(
            renamed.has_unpublished_changes,
            "a rename moves `updated_at`; it must not be read as a publish"
        );
    }

    /// Deleting the app takes its working directory with it. The tree is
    /// deliberately outside `MANAGED_DATASET_ROOTS`, so nothing else ever would.
    #[tokio::test]
    async fn deleting_an_app_removes_its_working_directory() {
        let fixture = Fixture::new();
        let id = fixture.solidify("<p>bye</p>").await;
        let dir = fixture
            .service
            .ensure_workspace(OWNER, &id)
            .await
            .expect("ensure");
        assert!(dir.join(MINIAPP_SOURCE_FILE).exists());

        fixture.service.delete(OWNER, &id).await.expect("delete");
        assert!(
            !dir.exists(),
            "the app's source must not outlive the app the user deleted"
        );
        // The root itself survives — other apps live there.
        assert!(
            nomifun_common::miniapp_workspace::miniapps_root(fixture._root.path()).exists()
        );
    }

    #[tokio::test]
    async fn a_stranger_cannot_delete_an_app_or_its_directory() {
        let fixture = Fixture::new();
        let id = fixture.solidify("<p>mine</p>").await;
        let dir = fixture
            .service
            .ensure_workspace(OWNER, &id)
            .await
            .expect("ensure");
        assert!(matches!(
            fixture.service.delete(STRANGER, &id).await,
            Err(MiniAppServiceError::NotFound)
        ));
        assert!(dir.join(MINIAPP_SOURCE_FILE).exists());
    }

    /// What 「继续迭代」 asks for: an absolute path to the working copy, materialized
    /// if it was not there, and the same answer every time.
    #[tokio::test]
    async fn provisioning_the_workspace_answers_the_absolute_source_path() {
        let fixture = Fixture::new();
        let html = "<!doctype html><title>v1</title>";
        let id = fixture.solidify(html).await;

        let provisioned = fixture
            .service
            .provision_workspace(OWNER, &id)
            .await
            .expect("provision");
        let source = std::path::Path::new(&provisioned.source_path);
        // Absolute, because the reader is a model in some other conversation's
        // workspace, where a relative path names nothing.
        assert!(source.is_absolute(), "{}", provisioned.source_path);
        assert_eq!(source.file_name().unwrap(), MINIAPP_SOURCE_FILE);
        assert_eq!(std::fs::read_to_string(source).expect("working copy"), html);

        // Idempotent, and it never re-materializes over work in progress.
        advance_past_mtime_resolution().await;
        let edited = "<!doctype html><title>edited</title>";
        std::fs::write(source, edited).unwrap();
        let again = fixture
            .service
            .provision_workspace(OWNER, &id)
            .await
            .expect("re-provision");
        assert_eq!(again.source_path, provisioned.source_path);
        assert_eq!(std::fs::read_to_string(source).expect("working copy"), edited);
    }

    #[tokio::test]
    async fn a_stranger_cannot_provision_a_workspace() {
        let fixture = Fixture::new();
        let id = fixture.solidify("<p>mine</p>").await;
        assert!(matches!(
            fixture.service.provision_workspace(STRANGER, &id).await,
            Err(MiniAppServiceError::NotFound)
        ));
        assert!(
            !fixture.service.workspace_dir(id.as_str()).expect("resolve").exists(),
            "a non-owner must not be able to mkdir"
        );
    }

    #[tokio::test]
    async fn a_metadata_edit_does_not_count_as_publishing() {
        let fixture = Fixture::new();
        let id = fixture.solidify("<p>v1</p>").await;
        let dir = fixture
            .service
            .ensure_workspace(OWNER, &id)
            .await
            .expect("ensure");
        advance_past_mtime_resolution().await;
        std::fs::write(dir.join(MINIAPP_SOURCE_FILE), "<p>v2</p>").unwrap();

        let renamed = fixture
            .service
            .update(
                OWNER,
                &id,
                UpdateMiniAppRequest {
                    name: Some("Renamed".into()),
                    ..Default::default()
                },
            )
            .await
            .expect("rename");
        assert!(
            renamed.has_unpublished_changes,
            "renaming an app must not make unpublished work look published"
        );
        assert_eq!(
            fixture.service.serve_html(&id).await.expect("serve"),
            "<p>v1</p>"
        );
    }

    #[test]
    fn the_escape_guard_refuses_anything_that_leaves_the_root() {
        let fixture = Fixture::new();
        for bad in [
            "..",
            "../evil",
            "0190f5fe-7c00-7a00-8000-000000000001/../../evil",
            "",
        ] {
            let text = message(fixture.service.workspace_dir(bad));
            assert!(
                text.contains("escapes") || text.contains("not a relative path"),
                "{bad} must be refused, got {text}"
            );
        }
        // An absolute path is not a relative path inside the root, whatever it
        // names — the client never sends one, and the import flow must not be able
        // to smuggle one in either.
        let absolute = if cfg!(windows) { "C:\\evil" } else { "/etc/passwd" };
        assert!(fixture.service.workspace_dir(absolute).is_err());
    }

    /// The guard's real job: a working copy replaced by a symlink out of the tree
    /// must not be readable or writable through it. Spelling checks cannot see
    /// this — only canonicalization can.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_symlinked_working_copy_is_refused() {
        let fixture = Fixture::new();
        let id = fixture.solidify("<p>hi</p>").await;
        let dir = fixture
            .service
            .ensure_workspace(OWNER, &id)
            .await
            .expect("ensure");
        let outside = fixture._root.path().join("outside.html");
        std::fs::write(&outside, "<p>not yours</p>").unwrap();
        std::fs::remove_file(dir.join(MINIAPP_SOURCE_FILE)).unwrap();
        std::os::unix::fs::symlink(&outside, dir.join(MINIAPP_SOURCE_FILE)).unwrap();

        let text = message(fixture.service.publish(OWNER, &id).await);
        assert!(text.contains("escapes"), "got {text}");
        // The snapshot is untouched, so the runner keeps serving the real app.
        assert_eq!(
            fixture.service.serve_html(&id).await.expect("serve"),
            "<p>hi</p>"
        );
    }
}
