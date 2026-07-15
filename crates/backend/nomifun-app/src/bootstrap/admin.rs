//! First-run admin credential bootstrap for headless authenticated hosts.
//!
//! The standalone web host (`nomifun-web`) runs the backend in authenticated
//! (non-`--local`) mode: every request needs a valid session. A brand-new data
//! dir seeds only the canonical installation owner with an EMPTY password hash, so
//! `has_users()` is false and there is no in-band HTTP endpoint to create the
//! first admin — the `/api/webui/*` and `/api/auth/internal/*` setup routes are
//! gated to local mode (`ensure_local_mode`).
//!
//! Two paths resolve this:
//!   * **Interactive first-run setup (the default):** leave the install
//!     uninitialised so the first browser visitor's chosen username + password
//!     become the admin via `POST /api/auth/setup` (see `auth::routes`). This
//!     function is then a no-op.
//!   * **Pre-seed (opt-in):** when the operator supplies `NOMIFUN_ADMIN_PASSWORD`
//!     (optionally `NOMIFUN_ADMIN_USERNAME`), this function provisions the admin
//!     at boot — closing the brief first-run window where anyone reaching the
//!     port could claim the account before the legitimate operator does.
//!
//! It is a no-op in local mode (the desktop shell needs no login) and idempotent
//! across restarts (skipped once a real user exists). It touches only username +
//! password_hash, so it never disturbs the JWT secret resolved by
//! [`crate::AppServices::from_config`].

use anyhow::{Context, Result};

use crate::AppServices;

/// Operator-supplied desired admin identity for first-run pre-seeding.
#[derive(Debug, Default, Clone)]
pub struct AdminBootstrap {
    /// Desired admin username. Defaults to `"admin"` when `None`.
    pub username: Option<String>,
    /// Desired admin password. When `None`, no pre-seed happens — the admin is
    /// created interactively via the WebUI first-run setup instead.
    pub password: Option<String>,
}

/// Pre-seed the first admin user when a password is supplied out-of-band.
///
/// Returns `Ok(true)` when the install still needs **interactive first-run
/// setup** (authenticated mode, no admin yet, nothing pre-seeded) — the caller
/// can warn if that state is exposed on a non-loopback address. Returns
/// `Ok(false)` when auth is disabled (local mode), an admin already exists, or
/// one was just pre-seeded. See the module docs.
pub async fn ensure_admin_credentials(services: &AppServices, opts: AdminBootstrap) -> Result<bool> {
    // NoAuth (desktop local / `--insecure-no-auth`) skips authentication
    // entirely — no admin needed. Under TrustLocalToken the desktop provisions
    // a password lazily when remote access is first enabled, not at boot.
    if !services.auth_policy.requires_admin_provisioning() {
        return Ok(false);
    }

    // Idempotent: once a real (non-empty-password) user exists, leave it alone.
    let has_users = services
        .user_repo
        .has_users()
        .await
        .map_err(|e| anyhow::anyhow!("admin bootstrap: failed to query users: {e}"))?;
    if has_users {
        if opts.password.is_some() {
            tracing::info!(
                "admin bootstrap: an admin already exists; ignoring NOMIFUN_ADMIN_PASSWORD. \
                 Rotate it from the in-app change-password flow instead."
            );
        }
        return Ok(false);
    }

    // No explicit password → leave the install uninitialised on purpose. The
    // first WebUI visitor sets the admin interactively (`POST /api/auth/setup`).
    let Some(password) = opts.password else {
        tracing::info!(
            "admin bootstrap: no NOMIFUN_ADMIN_PASSWORD set — the first WebUI visitor \
             will create the admin via first-run setup."
        );
        return Ok(true);
    };

    let username = opts.username.unwrap_or_else(|| "admin".to_owned());
    nomifun_auth::validate_username(&username)
        .map_err(|e| anyhow::anyhow!("admin bootstrap: invalid admin username '{username}': {e}"))?;
    nomifun_auth::validate_password(&password)
        .map_err(|e| anyhow::anyhow!("admin bootstrap: NOMIFUN_ADMIN_PASSWORD rejected: {e}"))?;

    // bcrypt is CPU-bound — hash off the async runtime.
    let pw_for_hash = password.clone();
    let hash = tokio::task::spawn_blocking(move || nomifun_auth::hash_password(&pw_for_hash))
        .await
        .context("admin bootstrap: password hash task panicked")?
        .map_err(|e| anyhow::anyhow!("admin bootstrap: hashing failed: {e}"))?;

    let provisioned = services
        .user_repo
        .set_system_user_credentials_if_uninitialized(&username, &hash)
        .await
        .map_err(|e| anyhow::anyhow!("admin bootstrap: failed to persist admin credentials: {e}"))?;

    if provisioned {
        tracing::info!(%username, "admin bootstrap: pre-seeded admin from NOMIFUN_ADMIN_PASSWORD");
    } else {
        tracing::info!("admin bootstrap: admin already initialized; skipped pre-seed");
    }
    Ok(false)
}
