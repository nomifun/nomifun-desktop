//! Read-only import of hosts from an OpenSSH client config (`~/.ssh/config`).
//!
//! This module *reads* ssh configuration, and — only when the user confirms an
//! import — one private key the config names. It never writes, moves, backs up
//! or re-permissions anything under `~/.ssh`, and it touches no other file
//! there: not `known_hosts`, not `authorized_keys`.
//!
//! Neither the config path nor `~` is resolved in here: both arrive as
//! arguments, and only [`scan_default_ssh_config`] consults the environment. A
//! test therefore points the whole parser at a tempdir and cannot read the
//! developer's real `~/.ssh` even by accident.
//!
//! ## Scope
//!
//! Deliberately the subset a host-book row needs — `Host`, `HostName`, `User`,
//! `Port`, `IdentityFile` — plus the two keywords that *disqualify* an entry
//! (`ProxyJump`/`ProxyCommand`; jump hosts are explicitly out of scope for v1,
//! so importing such an entry would only produce a host that cannot connect).
//! Everything else in a real config is ignored.
//!
//! Three deliberate departures from what `ssh(1)` itself would compute, each of
//! which can only make the import *offer less*, never connect somewhere the
//! user's own `ssh` would not:
//!
//! - **`Include` is not followed.** Following it would mean glob expansion,
//!   recursion and cycle limits, and a containment rule to stay inside `~/.ssh`
//!   — real machinery for a minority of configs. Instead the directives are
//!   counted, and the count is reported to the user, so an unexpectedly short
//!   candidate list always comes with the reason attached.
//! - **Pattern blocks are not inherited.** `Host *` (and any top-of-file global)
//!   is a template, not a machine; we skip it entirely rather than fold its
//!   `User`/`Port` into concrete hosts. An import can therefore arrive with a
//!   blank username that `ssh` would have filled in — visibly blank, in a form
//!   field, rather than silently wrong.
//! - **`Match` bodies are skipped.** Their conditions cannot be evaluated
//!   without running the user's `exec` predicates.
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Serialize;
use zeroize::Zeroizing;

/// The port ssh uses when a config says nothing.
pub const DEFAULT_SSH_PORT: i64 = 22;

/// Refuse to slurp an "identity file" larger than this. A key is a couple of
/// kilobytes; anything else is a misconfiguration (`IdentityFile /dev/zero`) and
/// this is a read we perform on the user's behalf, not one they can watch.
const MAX_IDENTITY_FILE_BYTES: u64 = 256 * 1024;

/// One host in the config that could be added to the book.
///
/// Non-secret by construction: the only credential-adjacent field is the
/// identity file's *path*. This type is what the candidate-list route serializes,
/// and no key material can reach it because none is ever read into it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshConfigHost {
    /// The `Host` alias, used as the host book's display name.
    pub alias: String,
    /// `HostName` if given, else the alias itself (real ssh semantics).
    pub host: String,
    pub port: i64,
    /// `User` if given. Left `None` rather than guessed from the local account:
    /// a wrong guess fails authentication far from its cause.
    pub username: Option<String>,
    /// `IdentityFile` with a leading `~/` expanded. A path, never contents.
    pub identity_file: Option<String>,
}

/// Everything one pass over a config file found.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshConfigScan {
    /// The file that was read, for the UI to name. `None` only when this
    /// account has no resolvable home directory.
    pub config_path: Option<String>,
    pub hosts: Vec<SshConfigHost>,
    /// Aliases left out because they go through a jump host. Named, not merely
    /// counted: a user whose whole config is bastion-fronted otherwise sees an
    /// empty list with no explanation.
    pub skipped_proxy: Vec<String>,
    /// How many `Include` directives were seen and not followed (see the module
    /// docs). Surfaced so a short list is never a silent one.
    pub skipped_includes: usize,
}

/// `<home>/.ssh/config` for this account.
pub fn default_ssh_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".ssh").join("config"))
}

/// Scan this account's `~/.ssh/config`.
///
/// The only function in this module that consults the environment; all of the
/// work is [`scan_ssh_config`], which tests drive against a tempdir.
pub fn scan_default_ssh_config() -> std::io::Result<SshConfigScan> {
    let Some(path) = default_ssh_config_path() else {
        return Ok(SshConfigScan::default());
    };
    scan_ssh_config(&path, dirs::home_dir().as_deref())
}

/// Read and parse one config file, expanding `~` against `home`.
pub fn scan_ssh_config(config_path: &Path, home: Option<&Path>) -> std::io::Result<SshConfigScan> {
    let mut scan = match std::fs::read_to_string(config_path) {
        Ok(text) => parse_ssh_config(&text, home),
        // No config file truthfully means "nothing to import", so it is not an
        // error. Any *other* io error is surfaced: answering "0 hosts" for a
        // config we failed to read would be a lie the user cannot detect.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => SshConfigScan::default(),
        Err(e) => return Err(e),
    };
    scan.config_path = Some(config_path.display().to_string());
    Ok(scan)
}

/// Parse config text. Pure — no filesystem access, no environment.
pub fn parse_ssh_config(text: &str, home: Option<&Path>) -> SshConfigScan {
    let mut scan = SshConfigScan::default();
    let mut seen_aliases: HashSet<String> = HashSet::new();
    let mut seen_endpoints: HashSet<(String, i64, Option<String>)> = HashSet::new();
    let mut current: Option<Block> = None;
    // Inside a `Match` body every directive is conditional, so none of it may be
    // attributed to anything.
    let mut in_match = false;

    for line in text.lines() {
        let Some((keyword, value)) = split_directive(line) else {
            continue;
        };
        match keyword.to_ascii_lowercase().as_str() {
            "host" => {
                flush(current.take(), &mut scan, home, &mut seen_aliases, &mut seen_endpoints);
                in_match = false;
                current = Some(Block::new(&value));
            }
            "match" => {
                flush(current.take(), &mut scan, home, &mut seen_aliases, &mut seen_endpoints);
                in_match = true;
            }
            "include" => scan.skipped_includes += 1,
            other => {
                if in_match {
                    continue;
                }
                if let Some(block) = current.as_mut() {
                    block.apply(other, &value);
                }
            }
        }
    }
    flush(current, &mut scan, home, &mut seen_aliases, &mut seen_endpoints);
    scan
}

/// Read the private key an `IdentityFile` points at, or `None` when there is no
/// usable private key there.
///
/// `None` covers every honest failure the same way — absent, unreadable, too
/// large, not UTF-8, or a *public* key (an `IdentityFile` may legally name one,
/// with the private half living in an agent). The caller reports the host as
/// still needing a credential rather than storing something that cannot
/// authenticate.
pub fn read_identity_file(path: &Path) -> Option<Zeroizing<String>> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_IDENTITY_FILE_BYTES {
        return None;
    }
    let body = Zeroizing::new(std::fs::read_to_string(path).ok()?);
    if !body.contains("PRIVATE KEY") {
        return None;
    }
    Some(body)
}

// ── internals ───────────────────────────────────────────────────────────

/// One `Host` block's accumulated state.
struct Block {
    /// The block's concrete (non-pattern) aliases, in file order.
    aliases: Vec<String>,
    host: Option<String>,
    username: Option<String>,
    port: Option<i64>,
    identity_file: Option<String>,
    /// A `ProxyJump`/`ProxyCommand` other than `none` was seen.
    proxied: bool,
}

impl Block {
    fn new(host_line: &str) -> Self {
        Block {
            // A pattern token (`*`, `?`, `!`) is a template for other hosts, not
            // a host. A line may mix the two (`Host web1 !staging`), so filter
            // per token rather than per line.
            aliases: split_args(host_line)
                .into_iter()
                .filter(|token| !token.contains(['*', '?', '!']))
                .collect(),
            host: None,
            username: None,
            port: None,
            identity_file: None,
            proxied: false,
        }
    }

    /// Record a directive. `keyword` is already lowercased.
    ///
    /// First value wins for every keyword, matching ssh's own
    /// first-obtained-value rule: a later duplicate is dead text in the user's
    /// config, and preferring it would import a host their `ssh` never dials.
    fn apply(&mut self, keyword: &str, value: &str) {
        match keyword {
            "hostname" if self.host.is_none() => self.host = Some(value.to_string()),
            "user" if self.username.is_none() => self.username = Some(value.to_string()),
            // An unparseable port is left at the default rather than rejecting
            // the whole host.
            "port" if self.port.is_none() => self.port = value.parse::<i64>().ok(),
            "identityfile" if self.identity_file.is_none() => {
                self.identity_file = Some(value.to_string());
            }
            // `ProxyCommand none` / `ProxyJump none` is how a concrete host opts
            // *out* of a pattern block's proxy — the opposite of having one.
            "proxycommand" | "proxyjump" if !value.eq_ignore_ascii_case("none") => {
                self.proxied = true;
            }
            _ => {}
        }
    }
}

fn flush(
    block: Option<Block>,
    scan: &mut SshConfigScan,
    home: Option<&Path>,
    seen_aliases: &mut HashSet<String>,
    seen_endpoints: &mut HashSet<(String, i64, Option<String>)>,
) {
    let Some(block) = block else { return };
    if block.aliases.is_empty() {
        return;
    }
    if block.proxied {
        scan.skipped_proxy.extend(block.aliases);
        return;
    }
    let identity_file = block
        .identity_file
        .as_deref()
        .map(|path| expand_tilde(path, home));
    for alias in block.aliases {
        let host = block.host.clone().unwrap_or_else(|| alias.clone());
        let port = block.port.unwrap_or(DEFAULT_SSH_PORT);
        let username = block.username.clone();
        if !seen_aliases.insert(alias.clone()) {
            continue;
        }
        // Two aliases resolving to the same `user@host:port` are one machine;
        // emitting both would offer to create two identical rows.
        if !seen_endpoints.insert((host.clone(), port, username.clone())) {
            continue;
        }
        scan.hosts.push(SshConfigHost {
            alias,
            host,
            port,
            username,
            identity_file: identity_file.clone(),
        });
    }
}

/// Split a config line into `(keyword, value)`, or `None` for a comment, a blank
/// line, or a keyword with no argument.
///
/// Keyword and value are separated by whitespace, an `=`, or both; a value may be
/// double-quoted. A line whose first non-blank character is `#` is a comment.
fn split_directive(line: &str) -> Option<(&str, String)> {
    let line = strip_trailing_comment(line.trim()).trim_end();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let separator = line.find(|c: char| c.is_whitespace() || c == '=')?;
    let (keyword, rest) = line.split_at(separator);
    let value = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '=');
    if value.is_empty() {
        return None;
    }
    Some((keyword, unquote(value).to_string()))
}

/// Drop a `#` comment that follows whitespace.
///
/// `ssh(1)` itself would keep it as part of the value, but `Port 2222 # staging`
/// is a common enough habit that importing what the user meant beats importing a
/// port that fails to parse — and none of the five values we read (host, user,
/// port, key path) plausibly contains a space followed by `#`.
fn strip_trailing_comment(line: &str) -> &str {
    let mut after_space = false;
    for (index, ch) in line.char_indices() {
        if ch == '#' && after_space {
            return &line[..index];
        }
        after_space = ch.is_whitespace();
    }
    line
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(value)
}

fn split_args(value: &str) -> Vec<String> {
    value
        .split_whitespace()
        .map(|token| unquote(token).to_string())
        .filter(|token| !token.is_empty())
        .collect()
}

/// Expand a leading `~/` against this account's home.
///
/// `~other/…` is left verbatim: it needs another account's home directory, which
/// we have no business guessing. Percent tokens (`%d`, `%r`, …) are likewise
/// left alone. Either way the user sees the path they actually wrote, and the key
/// simply reads as unavailable at import time.
///
/// Joined with `/` rather than `Path::join`, which on Windows would splice a
/// `\` into an otherwise `/`-separated config value and yield
/// `/home/user\.ssh/id_ed25519`. This value is an OpenSSH config path — the
/// separator OpenSSH writes and reads is `/`, and Windows accepts it too.
fn expand_tilde(path: &str, home: Option<&Path>) -> String {
    let Some(home) = home else {
        return path.to_string();
    };
    if path == "~" {
        return home.display().to_string();
    }
    match path.strip_prefix("~/") {
        Some(rest) => {
            let home = home.display().to_string();
            format!("{}/{rest}", home.trim_end_matches(['/', '\\']))
        }
        None => path.to_string(),
    }
}
