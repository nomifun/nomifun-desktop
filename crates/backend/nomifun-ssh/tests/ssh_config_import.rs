//! `~/.ssh/config` import.
//!
//! Every config in here is a throwaway file inside a `tempfile::tempdir()`, and
//! every `~` is expanded against that tempdir — a test in this file must never
//! read the developer's real `~/.ssh`, which is also why the parser takes both
//! the config path and the home directory as arguments instead of resolving
//! either one itself.
use std::fs;
use std::path::Path;

use nomifun_ssh::ssh_config::{parse_ssh_config, scan_ssh_config, SshConfigHost};

/// Look a candidate up by alias, so assertions do not depend on scan order
/// beyond the places that explicitly test ordering.
fn by_alias<'a>(hosts: &'a [SshConfigHost], alias: &str) -> &'a SshConfigHost {
    hosts
        .iter()
        .find(|h| h.alias == alias)
        .unwrap_or_else(|| panic!("no candidate {alias:?} in {hosts:#?}"))
}

fn aliases(hosts: &[SshConfigHost]) -> Vec<&str> {
    hosts.iter().map(|h| h.alias.as_str()).collect()
}

/// Write `text` as a config inside a fresh tempdir laid out like a home
/// directory, and return `(dir, config_path)`.
fn config_in_home(text: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let ssh_dir = dir.path().join(".ssh");
    fs::create_dir_all(&ssh_dir).expect("mkdir .ssh");
    let path = ssh_dir.join("config");
    fs::write(&path, text).expect("write config");
    (dir, path)
}

#[test]
fn wildcard_hosts_are_templates_not_hosts() {
    // `Host *` / `Host *.internal` / `Host !staging` are pattern blocks that
    // supply defaults; importing them would create hosts nobody can dial.
    let scan = parse_ssh_config(
        "Host *\n  User root\n\
         Host *.internal\n  User ops\n\
         Host prod-web\n  HostName 10.0.3.21\n  User deploy\n\
         Host !staging web?\n  User nobody\n",
        None,
    );
    assert_eq!(aliases(&scan.hosts), vec!["prod-web"]);
}

#[test]
fn entries_behind_a_jump_host_are_reported_not_imported() {
    // v1 has no ProxyJump/ProxyCommand support, so importing these would only
    // produce hosts that fail to connect. They are named in the scan so the user
    // learns why their bastion-only entries are missing.
    let scan = parse_ssh_config(
        "Host inner-a\n  HostName 10.1.0.5\n  ProxyJump bastion\n\
         Host inner-b\n  HostName 10.1.0.6\n  ProxyCommand ssh -W %h:%p bastion\n\
         Host bastion\n  HostName bastion.example.com\n  User jump\n",
        None,
    );
    assert_eq!(aliases(&scan.hosts), vec!["bastion"]);
    assert_eq!(scan.skipped_proxy, vec!["inner-a", "inner-b"]);
}

#[test]
fn proxy_command_none_is_an_override_not_a_jump_host() {
    // `ProxyCommand none` is how a concrete host opts *out* of a wildcard
    // block's proxy. Treating it as "has a jump host" would drop a perfectly
    // dialable host.
    let scan = parse_ssh_config(
        "Host direct\n  HostName 10.2.0.9\n  ProxyCommand none\n  ProxyJump none\n",
        None,
    );
    assert_eq!(aliases(&scan.hosts), vec!["direct"]);
    assert!(scan.skipped_proxy.is_empty(), "{:?}", scan.skipped_proxy);
}

#[test]
fn a_missing_hostname_falls_back_to_the_alias() {
    // Real ssh semantics: with no HostName, the alias *is* the hostname.
    let scan = parse_ssh_config("Host build.example.com\n  User ci\n", None);
    let host = by_alias(&scan.hosts, "build.example.com");
    assert_eq!(host.host, "build.example.com");
    assert_eq!(host.username.as_deref(), Some("ci"));
}

#[test]
fn port_defaults_to_22_and_a_missing_user_stays_empty() {
    // A blank user is left blank rather than guessed from the local account:
    // the form asks for it, and a wrong guess fails authentication later, far
    // from the cause.
    let scan = parse_ssh_config("Host bare\n  HostName 10.0.0.7\n", None);
    let host = by_alias(&scan.hosts, "bare");
    assert_eq!(host.port, 22);
    assert_eq!(host.username, None);
    assert_eq!(host.identity_file, None);
}

#[test]
fn an_explicit_port_wins_and_a_bogus_port_falls_back_to_22() {
    let scan = parse_ssh_config(
        "Host high\n  HostName 10.0.0.8\n  Port 2222\n\
         Host bogus\n  HostName 10.0.0.9\n  Port not-a-number\n",
        None,
    );
    assert_eq!(by_alias(&scan.hosts, "high").port, 2222);
    assert_eq!(by_alias(&scan.hosts, "bogus").port, 22);
}

#[test]
fn keywords_are_case_insensitive_and_survive_indentation_comments_and_equals() {
    // ssh_config keywords are case-insensitive, blocks are conventionally
    // indented, `=` is a legal separator, and full-line `#` is a comment.
    let scan = parse_ssh_config(
        "# a leading comment\n\
         \n\
         HOST  Mixed-Case\n\
         \t  hostname=10.0.4.4\n\
         \t  USER   deploy   # trailing note\n\
         \t  PoRt = 2022\n\
         \t  # an indented comment line\n\
         \t  IdentityFile \"/keys/id_rsa\"\n",
        None,
    );
    let host = by_alias(&scan.hosts, "Mixed-Case");
    assert_eq!(host.host, "10.0.4.4");
    assert_eq!(host.username.as_deref(), Some("deploy"));
    assert_eq!(host.port, 2022);
    assert_eq!(host.identity_file.as_deref(), Some("/keys/id_rsa"));
}

#[test]
fn the_first_value_of_a_repeated_keyword_wins() {
    // ssh itself takes the first obtained value for a keyword; a second one is
    // dead text, and silently preferring it would import a host the user's own
    // `ssh` command never talks to.
    let scan = parse_ssh_config(
        "Host dup\n  HostName first.example.com\n  HostName second.example.com\n  Port 2001\n  Port 2002\n",
        None,
    );
    let host = by_alias(&scan.hosts, "dup");
    assert_eq!(host.host, "first.example.com");
    assert_eq!(host.port, 2001);
}

#[test]
fn identity_file_tilde_expands_against_the_supplied_home() {
    let scan = parse_ssh_config(
        "Host prod\n  HostName 10.0.3.21\n  IdentityFile ~/.ssh/id_ed25519\n",
        Some(Path::new("/home/tester")),
    );
    assert_eq!(
        by_alias(&scan.hosts, "prod").identity_file.as_deref(),
        Some("/home/tester/.ssh/id_ed25519")
    );
}

#[test]
fn a_foreign_user_tilde_is_left_alone() {
    // `~other/` needs another account's home, which we cannot know. Left as
    // written so the import reports a path the user recognises instead of a
    // fabricated one; the key simply reads as unavailable.
    let scan = parse_ssh_config(
        "Host prod\n  HostName 10.0.3.21\n  IdentityFile ~someone/.ssh/id_rsa\n",
        Some(Path::new("/home/tester")),
    );
    assert_eq!(
        by_alias(&scan.hosts, "prod").identity_file.as_deref(),
        Some("~someone/.ssh/id_rsa")
    );
}

#[test]
fn include_directives_are_counted_not_followed() {
    // We do not follow `Include`. Counting it is what keeps that honest: the UI
    // says so, instead of showing a silently short list.
    let scan = parse_ssh_config(
        "Include config.d/*\n\
         Include ~/.ssh/work_config\n\
         Host local-only\n  HostName 10.0.0.2\n",
        None,
    );
    assert_eq!(aliases(&scan.hosts), vec!["local-only"]);
    assert_eq!(scan.skipped_includes, 2);
}

#[test]
fn a_match_block_does_not_leak_into_the_preceding_host() {
    // `Match` conditions cannot be evaluated here, so its body is skipped. If it
    // were merged into the previous block instead, `prod` would be imported with
    // a hostname that only applies when the condition holds.
    let scan = parse_ssh_config(
        "Host prod\n  HostName 10.0.3.21\n\
         Match host nonsense exec \"true\"\n  HostName 127.0.0.9\n  Port 9999\n\
         Host after\n  HostName 10.0.3.22\n",
        None,
    );
    assert_eq!(aliases(&scan.hosts), vec!["prod", "after"]);
    let prod = by_alias(&scan.hosts, "prod");
    assert_eq!(prod.host, "10.0.3.21");
    assert_eq!(prod.port, 22);
}

#[test]
fn one_host_line_with_several_plain_aliases_yields_one_candidate_each() {
    let scan = parse_ssh_config("Host web1 web2\n  User deploy\n", None);
    assert_eq!(aliases(&scan.hosts), vec!["web1", "web2"]);
    assert_eq!(by_alias(&scan.hosts, "web1").host, "web1");
    assert_eq!(by_alias(&scan.hosts, "web2").host, "web2");
}

#[test]
fn aliases_of_one_login_collapse_to_the_first() {
    // Two aliases resolving to the same user@host:port are one machine. Emitting
    // both would import two identical rows into the host book.
    let scan = parse_ssh_config(
        "Host prod prod-short\n  HostName 10.0.3.21\n  User deploy\n\
         Host prod-again\n  HostName 10.0.3.21\n  User deploy\n\
         Host prod-other-user\n  HostName 10.0.3.21\n  User root\n",
        None,
    );
    assert_eq!(aliases(&scan.hosts), vec!["prod", "prod-other-user"]);
}

#[test]
fn scanning_an_absent_config_is_nothing_to_import_not_a_failure() {
    let dir = tempfile::tempdir().expect("tempdir");
    let missing = dir.path().join(".ssh").join("config");
    let scan = scan_ssh_config(&missing, Some(dir.path())).expect("absent config is not an error");
    assert!(scan.hosts.is_empty());
    assert_eq!(scan.config_path.as_deref(), Some(missing.to_str().unwrap()));
}

#[test]
fn the_candidate_list_never_carries_private_key_material() {
    // The GET that feeds the import screen must be non-secret by construction:
    // it may name the identity file, never its contents.
    let (dir, config) = config_in_home(
        "Host prod\n  HostName 10.0.3.21\n  User deploy\n  IdentityFile ~/.ssh/id_ed25519\n",
    );
    let key = dir.path().join(".ssh").join("id_ed25519");
    fs::write(
        &key,
        "-----BEGIN OPENSSH PRIVATE KEY-----\nTOTALLY-FAKE-KEY-BODY\n-----END OPENSSH PRIVATE KEY-----\n",
    )
    .expect("write fake key");

    let scan = scan_ssh_config(&config, Some(dir.path())).expect("scan");
    let json = serde_json::to_string(&scan).expect("serialize");
    assert!(
        !json.contains("TOTALLY-FAKE-KEY-BODY"),
        "key body leaked into the candidate list: {json}"
    );
    assert!(
        !json.contains("PRIVATE KEY"),
        "key material leaked into the candidate list: {json}"
    );
    // The path itself is not a secret, and the user needs to see which key a
    // host would use.
    assert!(json.contains("id_ed25519"), "the key path should be shown: {json}");
}

#[test]
fn the_candidate_list_serializes_camel_case() {
    let scan = parse_ssh_config(
        "Host prod\n  HostName 10.0.3.21\n  User deploy\n  IdentityFile /keys/id\n\
         Host jumped\n  ProxyJump bastion\n\
         Include other\n",
        None,
    );
    let value = serde_json::to_value(&scan).expect("serialize");
    let mut keys: Vec<&str> = value
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    let mut expected = vec!["configPath", "hosts", "skippedProxy", "skippedIncludes"];
    expected.sort_unstable();
    assert_eq!(keys, expected);

    let mut host_keys: Vec<&str> = value["hosts"][0]
        .as_object()
        .expect("host object")
        .keys()
        .map(String::as_str)
        .collect();
    host_keys.sort_unstable();
    let mut expected_host = vec!["alias", "host", "port", "username", "identityFile"];
    expected_host.sort_unstable();
    assert_eq!(host_keys, expected_host);
}
