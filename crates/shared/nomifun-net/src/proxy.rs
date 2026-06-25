use std::collections::HashSet;
use std::sync::OnceLock;

use tracing::{debug, warn};

const PROXY_ENV_KEYS: &[&str] = &[
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct SystemProxyConfig {
    http_proxy: Option<String>,
    https_proxy: Option<String>,
    all_proxy: Option<String>,
    no_proxy: Option<String>,
}

pub fn apply_detected_proxy(mut builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    if process_has_proxy_env() {
        return builder;
    }

    let Some(config) = system_proxy_config() else {
        return builder;
    };

    let no_proxy = effective_no_proxy(config);

    if let Some(proxy_url) = config.all_proxy.as_deref() {
        match reqwest::Proxy::all(proxy_url) {
            Ok(proxy) => {
                debug!("Using detected system ALL_PROXY for outbound HTTP client");
                return builder.proxy(proxy.no_proxy(no_proxy));
            }
            Err(err) => warn!(error = %err, "Ignoring invalid detected system ALL_PROXY"),
        }
    }

    if let Some(proxy_url) = config.http_proxy.as_deref() {
        match reqwest::Proxy::http(proxy_url) {
            Ok(proxy) => {
                debug!("Using detected system HTTP_PROXY for outbound HTTP client");
                builder = builder.proxy(proxy.no_proxy(no_proxy.clone()));
            }
            Err(err) => warn!(error = %err, "Ignoring invalid detected system HTTP_PROXY"),
        }
    }

    if let Some(proxy_url) = config.https_proxy.as_deref() {
        match reqwest::Proxy::https(proxy_url) {
            Ok(proxy) => {
                debug!("Using detected system HTTPS_PROXY for outbound HTTP client");
                builder = builder.proxy(proxy.no_proxy(no_proxy));
            }
            Err(err) => warn!(error = %err, "Ignoring invalid detected system HTTPS_PROXY"),
        }
    }

    builder
}

pub fn child_proxy_env<'a, I>(configured_env_names: I) -> Vec<(String, String)>
where
    I: IntoIterator<Item = &'a str>,
{
    let configured_names: HashSet<String> = configured_env_names
        .into_iter()
        .map(|name| name.to_ascii_uppercase())
        .collect();
    let process_names = process_env_proxy_names();
    if has_proxy_name(&configured_names) || has_proxy_name(&process_names) {
        return Vec::new();
    }

    let Some(config) = system_proxy_config() else {
        return Vec::new();
    };

    proxy_env_from_config(config, &configured_names, &process_names)
}

fn system_proxy_config() -> Option<&'static SystemProxyConfig> {
    static CONFIG: OnceLock<Option<SystemProxyConfig>> = OnceLock::new();
    CONFIG.get_or_init(detect_system_proxy).as_ref()
}

fn process_has_proxy_env() -> bool {
    has_proxy_name(&process_env_proxy_names())
}

fn process_env_proxy_names() -> HashSet<String> {
    std::env::vars()
        .filter(|(_, value)| !value.trim().is_empty())
        .map(|(name, _)| name.to_ascii_uppercase())
        .collect()
}

fn has_proxy_name(names: &HashSet<String>) -> bool {
    PROXY_ENV_KEYS
        .iter()
        .any(|key| names.contains(&key.to_ascii_uppercase()))
}

fn has_env_name(names: &HashSet<String>, key: &str) -> bool {
    names.contains(&key.to_ascii_uppercase())
}

fn proxy_env_from_config(
    config: &SystemProxyConfig,
    configured_names: &HashSet<String>,
    process_names: &HashSet<String>,
) -> Vec<(String, String)> {
    let mut vars = Vec::new();
    push_proxy_pair(
        &mut vars,
        "HTTP_PROXY",
        config.http_proxy.as_deref(),
        configured_names,
        process_names,
    );
    push_proxy_pair(
        &mut vars,
        "HTTPS_PROXY",
        config.https_proxy.as_deref(),
        configured_names,
        process_names,
    );
    push_proxy_pair(
        &mut vars,
        "ALL_PROXY",
        config.all_proxy.as_deref(),
        configured_names,
        process_names,
    );

    if let Some(no_proxy) = config.no_proxy.as_deref() {
        push_if_missing(
            &mut vars,
            "NO_PROXY",
            no_proxy,
            configured_names,
            process_names,
        );
        push_if_missing(
            &mut vars,
            "no_proxy",
            no_proxy,
            configured_names,
            process_names,
        );
    }

    vars
}

fn push_proxy_pair(
    vars: &mut Vec<(String, String)>,
    upper_key: &str,
    value: Option<&str>,
    configured_names: &HashSet<String>,
    process_names: &HashSet<String>,
) {
    let Some(value) = value else {
        return;
    };
    push_if_missing(vars, upper_key, value, configured_names, process_names);
    push_if_missing(
        vars,
        &upper_key.to_ascii_lowercase(),
        value,
        configured_names,
        process_names,
    );
}

fn push_if_missing(
    vars: &mut Vec<(String, String)>,
    key: &str,
    value: &str,
    configured_names: &HashSet<String>,
    process_names: &HashSet<String>,
) {
    if has_env_name(configured_names, key) || has_env_name(process_names, key) {
        return;
    }
    vars.push((key.to_owned(), value.to_owned()));
}

fn effective_no_proxy(config: &SystemProxyConfig) -> Option<reqwest::NoProxy> {
    let mut items = Vec::new();
    if let Ok(value) = std::env::var("NO_PROXY").or_else(|_| std::env::var("no_proxy"))
        && !value.trim().is_empty()
    {
        items.push(value);
    }
    if let Some(value) = config.no_proxy.as_ref()
        && !value.trim().is_empty()
    {
        items.push(value.clone());
    }

    if items.is_empty() {
        return None;
    }
    reqwest::NoProxy::from_string(&items.join(","))
}

fn detect_system_proxy() -> Option<SystemProxyConfig> {
    detect_platform_proxy()
}

#[cfg(target_os = "macos")]
fn detect_platform_proxy() -> Option<SystemProxyConfig> {
    use std::process::Command;

    let output = Command::new("/usr/sbin/scutil")
        .arg("--proxy")
        .output()
        .or_else(|_| Command::new("scutil").arg("--proxy").output())
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_scutil_proxy(&stdout)
}

#[cfg(not(target_os = "macos"))]
fn detect_platform_proxy() -> Option<SystemProxyConfig> {
    None
}

#[cfg(target_os = "macos")]
fn parse_scutil_proxy(text: &str) -> Option<SystemProxyConfig> {
    let mut http_enable = false;
    let mut https_enable = false;
    let mut socks_enable = false;
    let mut http_host = None;
    let mut https_host = None;
    let mut socks_host = None;
    let mut http_port = None;
    let mut https_port = None;
    let mut socks_port = None;
    let mut exceptions = Vec::new();
    let mut in_exceptions = false;

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.starts_with("ExceptionsList") && line.contains("<array>") {
            in_exceptions = true;
            continue;
        }
        if in_exceptions {
            if line.starts_with('}') {
                in_exceptions = false;
                continue;
            }
            if let Some((_, value)) = line.split_once(':') {
                exceptions.extend(parse_exception_values(value));
            }
            continue;
        }

        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        match key {
            "HTTPEnable" => http_enable = value == "1",
            "HTTPSEnable" => https_enable = value == "1",
            "SOCKSEnable" => socks_enable = value == "1",
            "HTTPProxy" => http_host = non_empty(value),
            "HTTPSProxy" => https_host = non_empty(value),
            "SOCKSProxy" => socks_host = non_empty(value),
            "HTTPPort" => http_port = parse_port(value),
            "HTTPSPort" => https_port = parse_port(value),
            "SOCKSPort" => socks_port = parse_port(value),
            _ => {}
        }
    }

    let http_proxy = enabled_proxy_url(http_enable, "http", http_host.as_deref(), http_port);
    let https_proxy = enabled_proxy_url(https_enable, "http", https_host.as_deref(), https_port);
    let all_proxy = if http_proxy.is_none() && https_proxy.is_none() && socks_enable {
        enabled_proxy_url(true, "socks5h", socks_host.as_deref(), socks_port)
    } else {
        None
    };

    if http_proxy.is_none() && https_proxy.is_none() && all_proxy.is_none() {
        return None;
    }

    Some(SystemProxyConfig {
        http_proxy,
        https_proxy,
        all_proxy,
        no_proxy: build_no_proxy(exceptions),
    })
}

#[cfg(target_os = "macos")]
fn enabled_proxy_url(
    enabled: bool,
    scheme: &str,
    host: Option<&str>,
    port: Option<u16>,
) -> Option<String> {
    if !enabled {
        return None;
    }
    proxy_url(scheme, host?, port?)
}

#[cfg(target_os = "macos")]
fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

#[cfg(target_os = "macos")]
fn parse_port(value: &str) -> Option<u16> {
    value.trim().parse().ok()
}

#[cfg(target_os = "macos")]
fn proxy_url(scheme: &str, host: &str, port: u16) -> Option<String> {
    let host = host.trim();
    if host.is_empty() {
        return None;
    }
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    Some(format!("{scheme}://{host}:{port}"))
}

#[cfg(target_os = "macos")]
fn parse_exception_values(value: &str) -> Vec<String> {
    value
        .split(',')
        .filter_map(|item| {
            let item = item.trim();
            if item.is_empty() {
                return None;
            }
            Some(item.to_owned())
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn build_no_proxy(exceptions: Vec<String>) -> Option<String> {
    let mut items = vec![
        "localhost".to_owned(),
        "127.0.0.1".to_owned(),
        "::1".to_owned(),
        "10.0.0.0/8".to_owned(),
        "127.0.0.0/8".to_owned(),
        "172.16.0.0/12".to_owned(),
        "192.168.0.0/16".to_owned(),
    ];
    for item in exceptions {
        if let Some(normalized) = normalize_no_proxy_item(&item) {
            items.push(normalized);
        }
    }
    items.sort();
    items.dedup();

    (!items.is_empty()).then(|| items.join(","))
}

#[cfg(target_os = "macos")]
fn normalize_no_proxy_item(item: &str) -> Option<String> {
    let item = item.trim();
    if item.is_empty() {
        return None;
    }
    match item {
        "127.*" => return Some("127.0.0.0/8".to_owned()),
        "10.*" => return Some("10.0.0.0/8".to_owned()),
        "192.168.*" => return Some("192.168.0.0/16".to_owned()),
        _ => {}
    }
    if item.starts_with("172.") {
        return Some("172.16.0.0/12".to_owned());
    }
    if let Some(domain) = item.strip_prefix("*.") {
        return Some(format!(".{domain}"));
    }
    if let Some(domain) = item.strip_prefix('*') {
        return (!domain.is_empty()).then(|| domain.to_owned());
    }
    Some(item.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_env_from_config_adds_upper_and_lowercase_keys() {
        let config = SystemProxyConfig {
            http_proxy: Some("http://127.0.0.1:7892".to_owned()),
            https_proxy: Some("http://127.0.0.1:7892".to_owned()),
            all_proxy: None,
            no_proxy: Some("localhost,127.0.0.1".to_owned()),
        };
        let vars = proxy_env_from_config(&config, &HashSet::new(), &HashSet::new());

        assert!(vars.contains(&("HTTP_PROXY".to_owned(), "http://127.0.0.1:7892".to_owned())));
        assert!(vars.contains(&("http_proxy".to_owned(), "http://127.0.0.1:7892".to_owned())));
        assert!(vars.contains(&("HTTPS_PROXY".to_owned(), "http://127.0.0.1:7892".to_owned())));
        assert!(vars.contains(&("NO_PROXY".to_owned(), "localhost,127.0.0.1".to_owned())));
    }

    #[test]
    fn proxy_env_from_config_respects_existing_names() {
        let config = SystemProxyConfig {
            http_proxy: Some("http://127.0.0.1:7892".to_owned()),
            https_proxy: Some("http://127.0.0.1:7892".to_owned()),
            all_proxy: None,
            no_proxy: Some("localhost,127.0.0.1".to_owned()),
        };
        let configured_names = HashSet::from(["HTTPS_PROXY".to_owned()]);
        let process_names = HashSet::from(["NO_PROXY".to_owned()]);

        let vars = proxy_env_from_config(&config, &configured_names, &process_names);

        assert!(!vars.iter().any(|(name, _)| name == "HTTPS_PROXY"));
        assert!(!vars.iter().any(|(name, _)| name == "https_proxy"));
        assert!(!vars.iter().any(|(name, _)| name == "NO_PROXY"));
        assert!(vars.contains(&("HTTP_PROXY".to_owned(), "http://127.0.0.1:7892".to_owned())));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn child_proxy_env_uses_current_macos_system_proxy_when_needed() {
        let Some(config) = detect_platform_proxy() else {
            return;
        };
        if config.http_proxy.is_none() && config.https_proxy.is_none() && config.all_proxy.is_none()
        {
            return;
        }

        let vars = child_proxy_env(std::iter::empty());

        if process_has_proxy_env() {
            assert!(vars.is_empty());
        } else {
            let values: HashSet<&str> = vars.iter().map(|(_, value)| value.as_str()).collect();
            if let Some(http_proxy) = config.http_proxy.as_deref() {
                assert!(values.contains(http_proxy));
            }
            if let Some(https_proxy) = config.https_proxy.as_deref() {
                assert!(values.contains(https_proxy));
            }
            if let Some(all_proxy) = config.all_proxy.as_deref() {
                assert!(values.contains(all_proxy));
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn parse_scutil_proxy_extracts_http_https_and_exceptions() {
        let input = r#"<dictionary> {
  ExceptionsList : <array> {
    0 : *zhihu.com,*zhimg.com,localhost,*.local,127.*,10.*,172.16.*,192.168.*
  }
  HTTPEnable : 1
  HTTPPort : 7892
  HTTPProxy : 127.0.0.1
  HTTPSEnable : 1
  HTTPSPort : 7892
  HTTPSProxy : 127.0.0.1
  SOCKSEnable : 1
  SOCKSPort : 7892
  SOCKSProxy : 127.0.0.1
}"#;

        let config = parse_scutil_proxy(input).expect("proxy config");

        assert_eq!(config.http_proxy.as_deref(), Some("http://127.0.0.1:7892"));
        assert_eq!(config.https_proxy.as_deref(), Some("http://127.0.0.1:7892"));
        assert_eq!(config.all_proxy, None);
        let no_proxy = config.no_proxy.expect("no_proxy");
        assert!(no_proxy.contains("zhihu.com"));
        assert!(no_proxy.contains(".local"));
        assert!(no_proxy.contains("127.0.0.0/8"));
        assert!(no_proxy.contains("192.168.0.0/16"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn parse_scutil_proxy_uses_socks_when_http_is_absent() {
        let input = r#"<dictionary> {
  SOCKSEnable : 1
  SOCKSPort : 7892
  SOCKSProxy : 127.0.0.1
}"#;

        let config = parse_scutil_proxy(input).expect("proxy config");

        assert_eq!(
            config.all_proxy.as_deref(),
            Some("socks5h://127.0.0.1:7892")
        );
    }
}
