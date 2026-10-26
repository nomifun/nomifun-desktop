//! The verified v1.0.98 command/JSON boundary. No shell text or CLI hints execute.
use crate::{HubError, runtime::{CliCommand, CliOutput, PAYLOAD_FILE}, types::*};
use chrono::DateTime;
use serde_json::{Value, json};
use std::{collections::{BTreeMap,BTreeSet}, time::Duration};
pub struct CliIdentity {
    pub actor: Option<OfficeActor>,
    pub phase: ConnectionPhase,
    pub scopes: Vec<String>,
    pub message: Option<String>,
}
pub struct LoginStart {
    pub verification_url: String,
    pub device_code: String,
    pub expires_in: u64,
}
pub struct LoginCompletion {
    pub actor: Option<OfficeActor>,
    pub missing_scopes: Vec<String>,
    pub complete: bool,
    pub message: Option<String>,
}
pub struct MutationReceipt {
    pub status: ItemStatus,
    pub remote_id: Option<String>,
    pub remote_url: Option<String>,
    pub message: Option<String>,
    pub result: Value,
}

fn valid_text(value: &str, label: &str, max: usize) -> Result<(), HubError> {
    if value.trim().is_empty() || value.chars().count() > max || value.contains('\0') {
        return Err(HubError::invalid(format!("{label}为空、过长或含有非法字符")));
    }
    Ok(())
}
fn valid_id(value: &str, label: &str) -> Result<(), HubError> {
    valid_text(value, label, 512)?;
    if value.chars().any(|c| c.is_control()) { return Err(HubError::invalid(format!("{label}包含控制字符"))); }
    Ok(())
}
fn valid_page_token(value:&str)->Result<(),HubError>{
    // Provider continuation data is not a resource ID. Keep its exact bytes;
    // the separate transport budget also fits a Windows CLI command line.
    if value.is_empty() || value.len()>16*1024 || value.chars().any(char::is_control) {
        return Err(HubError::invalid("分页令牌为空、超过传输上限或包含控制字符"));
    }
    Ok(())
}
fn command(args: impl IntoIterator<Item = impl Into<String>>, profile: Option<&str>,
    mutating: bool) -> Result<CliCommand, HubError>
{
    let mut argv = Vec::new();
    if let Some(profile) = profile {
        valid_id(profile, "连接名称")?;
        argv.extend(["--profile".into(), profile.into()]);
    }
    argv.extend(args.into_iter().map(Into::into));
    Ok(CliCommand::new(argv, mutating))
}
fn user_command(args: impl IntoIterator<Item = impl Into<String>>, profile: Option<&str>,
    mutating: bool) -> Result<CliCommand, HubError>
{
    let mut value = command(args, profile, mutating)?;
    value.args.extend(["--as".into(), "user".into()]);
    Ok(value)
}
fn strings(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::String(s)) => s.split_whitespace().filter(|s| !s.is_empty()).map(str::to_owned).collect(),
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_owned).collect(),
        _ => Vec::new(),
    }
}
fn text(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}
fn first_text(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| text(value.get(*key)))
}

fn has_scope(scopes: &[String], required: &str) -> bool {
    // CLI shortcut preflight checks exact scopes, including delete metadata.
    scopes.iter().any(|scope| scope == required)
}
fn capability(name: &str, required: &[&str], granted: &[String]) -> OfficePermission {
    OfficePermission { capability: name.into(), required_scopes: required.iter().map(|s| (*s).into()).collect(),
        granted: required.iter().all(|scope| has_scope(granted, scope)) }
}
pub fn permissions(scopes: &[String]) -> Vec<OfficePermission> {
    vec![
        capability("documents.browse", &["search:docs:read"], scopes),
        capability("documents.create", &["docx:document:create"], scopes),
        capability("documents.delete", &["space:document:delete", "drive:drive.metadata:readonly"], scopes),
        capability("people.read", &["contact:user:search"], scopes),
        capability("tasks.read", &["task:task:read"], scopes),
        capability("tasks.write", &["task:task:write"], scopes),
        capability("tasklists.read", &["task:tasklist:read"], scopes),
        capability("calendar.read", &["calendar:calendar.event:read"], scopes),
        capability("calendar.write", &["calendar:calendar.event:create"], scopes),
        capability("base.read", &["base:record:read", "base:field:read"], scopes),
        capability("base.write", &["base:record:update"], scopes),
    ]
}
pub fn allowed_scopes() -> Vec<String> {
    let mut result: BTreeSet<String> = permissions(&[]).into_iter().flat_map(|p| p.required_scopes).collect();
    result.insert("offline_access".into());
    result.into_iter().collect()
}
fn named_permissions(names: &[&str], scopes: &[String]) -> Vec<OfficePermission> {
    permissions(scopes).into_iter().filter(|p| names.contains(&p.capability.as_str())).collect()
}
pub fn query_permissions(query: &OfficeQuery, scopes: &[String]) -> Vec<OfficePermission> {
    let names: &[&str] = match query {
        OfficeQuery::Documents { .. } => &["documents.browse"],
        OfficeQuery::People { .. } => &["people.read"],
        OfficeQuery::Tasklists { .. } => &["tasklists.read"],
        OfficeQuery::Tasks { relation: TaskRelation::Tasklist, .. } => &["tasklists.read"],
        OfficeQuery::Tasks { .. } => &["tasks.read"],
        OfficeQuery::Agenda { .. } | OfficeQuery::CalendarEvent { .. } => &["calendar.read"],
        OfficeQuery::BaseRecords { .. } => &["base.read"],
    };
    named_permissions(names, scopes)
}
pub fn mutation_permissions(mutation: &OfficeMutation, scopes: &[String]) -> Vec<OfficePermission> {
    let names: &[&str] = match mutation {
        OfficeMutation::TaskCreate { tasklist_id: Some(_), .. } => &["tasks.write", "tasklists.read"],
        OfficeMutation::TaskCreate { .. } | OfficeMutation::TaskComplete { .. } => &["tasks.write"],
        OfficeMutation::DocumentCreate { .. } => &["documents.create"],
        OfficeMutation::DocumentDelete { .. } => &["documents.delete"],
        OfficeMutation::CalendarCreate { .. } => &["calendar.write"],
        OfficeMutation::BaseUpdate { .. } => &["base.read", "base.write"],
    };
    named_permissions(names, scopes)
}

pub fn status_command(profile: Option<&str>, verify: bool) -> CliCommand {
    let mut args:Vec<String> = vec!["auth".into(), "status".into(), "--json".into()];
    if verify { args.push("--verify".into()); }
    // An invalid selector must fail; dropping it could read the user's default
    // account instead of the requested connection.
    command(args, profile, false).unwrap_or_else(|_| CliCommand::new(["--nomifun-invalid-profile"], false))
}
pub fn profiles_command() -> CliCommand { CliCommand::new(["profile", "list"], false) }
pub fn auth_users_command(profile: &str) -> Result<CliCommand, HubError> {
    command(["auth", "list", "--json"], Some(profile), false)
}
pub fn logout_command(profile: &str) -> Result<CliCommand, HubError> {
    command(["auth", "logout", "--json"], Some(profile), true)
}
pub fn configure_command(request: &AuthStartRequest) -> Result<CliCommand, HubError> {
    let name = request.profile.as_deref().ok_or_else(|| HubError::invalid("新应用配置必须使用专用连接名称"))?;
    valid_id(name, "连接名称")?;
    let mut args:Vec<String> = vec!["config".into(), "init".into(), "--brand".into(), "feishu".into(),
        "--name".into(), name.into()];
    let mut stdin = None;
    match request.kind {
        AuthKind::ConfigureNew => args.push("--new".into()),
        AuthKind::ConfigureExisting => {
            let app_id = request.app_id.as_deref().ok_or_else(|| HubError::invalid("缺少应用 ID"))?;
            valid_id(app_id, "应用 ID")?;
            let secret = request.app_secret.as_deref().ok_or_else(|| HubError::invalid("缺少应用密钥"))?;
            if secret.trim().is_empty() || secret.len() > 4096 || secret.contains(['\0', '\r', '\n']) {
                return Err(HubError::invalid("应用密钥为空、过长或格式不正确"));
            }
            args.extend(["--app-id".into(), app_id.into(), "--app-secret-stdin".into()]);
            stdin = Some(secret.as_bytes().to_vec());
        }
        AuthKind::Login => return Err(HubError::invalid("用户登录不使用应用配置命令")),
    }
    let mut result = CliCommand::new(args, false);
    result.stdin = stdin;result.timeout = Duration::from_secs(900);
    Ok(result)
}
pub fn login_command(scopes: &[String], profile: Option<&str>) -> Result<CliCommand, HubError> {
    if scopes.is_empty() || scopes.len() > 64 { return Err(HubError::invalid("请选择具体办公权限")); }
    let allowed = allowed_scopes();
    if scopes.iter().any(|scope| !allowed.contains(scope)) {
        return Err(HubError::invalid("授权请求包含首期不支持的权限"));
    }
    let scopes: BTreeSet<&str> = scopes.iter().map(String::as_str).collect();
    command(["auth".into(), "login".into(), "--scope".into(), scopes.into_iter().collect::<Vec<_>>().join(" "),
        "--no-wait".into(), "--json".into()], profile, false)
}
pub fn poll_login_command(device_code: &str, profile: Option<&str>) -> Result<CliCommand, HubError> {
    valid_text(device_code, "本次授权标识", 4096)?;
    let mut result = command(["auth", "login", "--device-code", device_code, "--json"], profile, false)?;
    result.timeout = Duration::from_secs(900);
    Ok(result)
}

/// Keep the exact URL query. Never reconstruct an OAuth link or accept userinfo,
/// non-HTTPS, an encoded host, or unrelated domains.
pub fn official_url(value: &str) -> Option<String> {
    let remaining = value.strip_prefix("https://")?;
    let authority = remaining.split(['/', '?', '#']).next()?;
    if authority.is_empty() || authority.contains('@') || value.contains(['\\', '\r', '\n']) { return None; }
    let host = authority.strip_suffix(":443").unwrap_or(authority);
    if !host.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-') { return None; }
    let host = host.to_ascii_lowercase();
    let allowed = ["feishu.cn", "larksuite.com", "larkoffice.com"].iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")));
    allowed.then(|| value.to_owned())
}

pub fn document_type(kind: &str) -> bool {
    matches!(kind, "docx" | "doc" | "sheet" | "mindnote" | "slides" | "file")
}
fn valid_resource_token(token: &str) -> Result<(), HubError> {
    if token.is_empty() || token.len() > 512 || !token.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)) {
        return Err(HubError::invalid("资源标识必须是确切的飞书 token"));
    }
    Ok(())
}
fn link_target(value: &str) -> Result<(String, String), HubError> {
    let official=official_url(value).ok_or_else(||HubError::invalid("请使用飞书或 Lark 官方 HTTPS 文档链接"))?;
    let url=url::Url::parse(&official).map_err(|_|HubError::invalid("文档链接格式不正确"))?;
    let parts=url.path().trim_matches('/').split('/').collect::<Vec<_>>();
    if parts.len()!=2 { return Err(HubError::invalid("请使用完整的飞书文档链接")); }
    let kind=match parts[0] {"docs"=>"doc","sheets"=>"sheet","base"=>"bitable",kind=>kind};
    if !document_type(kind) && !matches!(kind,"bitable"|"wiki") { return Err(HubError::invalid("此链接不是飞书文档资源")); }
    valid_resource_token(parts[1])?;
    Ok((kind.into(),parts[1].into()))
}
pub fn document_target(item: &OfficeItem) -> Option<(String, String)> {
    if document_type(&item.kind) && valid_resource_token(&item.id).is_ok() {
        return Some((item.kind.clone(),item.id.clone()));
    }
    let (kind,id)=link_target(item.url.as_deref()?).ok()?;
    document_type(&kind).then_some((kind,id))
}

fn json_values(text: &str) -> Vec<Value> {
    if let Ok(value) = serde_json::from_str(text.trim()) { return vec![value]; }
    let mut values = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(['{', '[']) {
        rest = &rest[start..];
        let mut stream = serde_json::Deserializer::from_str(rest).into_iter::<Value>();
        if let Some(Ok(value)) = stream.next() {
            let offset = stream.byte_offset();
            if offset == 0 { break; }
            values.push(value);rest = &rest[offset..];
        } else {
            // Increment along UTF-8 boundaries, never interpret surrounding text.
            rest = &rest[rest.chars().next().map(char::len_utf8).unwrap_or(1)..];
        }
    }
    values
}
fn output_value(output: &CliOutput) -> Option<Value> { json_values(&output.stdout).into_iter().last() }
fn error_value(output: &CliOutput) -> Option<Value> {
    json_values(&output.stderr).into_iter().rev().find(|v| v.get("error").is_some())
        .or_else(|| json_values(&output.stdout).into_iter().rev().find(|v| v.get("error").is_some()))
}
fn safe_message(message: &str) -> String {
    let scrubbed = nomi_redact::redact_secrets(message);
    let pattern = regex::Regex::new(r#"(?i)(device[_-]?code|app[_-]?secret|access[_-]?token|refresh[_-]?token)(\s*[:=]\s*["']?)([^"'\s,&}]+)"#)
        .expect("static auth redaction pattern");
    let result = pattern.replace_all(&scrubbed, "$1$2[REDACTED]").into_owned();
    result.chars().take(2000).collect()
}
pub fn output_error(output: &CliOutput, mutating: bool) -> HubError {
    let envelope = error_value(output).unwrap_or(Value::Null);
    let error = envelope.get("error").unwrap_or(&Value::Null);
    let error_type = error.get("type").and_then(Value::as_str).unwrap_or("");
    let subtype = error.get("subtype").and_then(Value::as_str).unwrap_or("");
    let missing = strings(error.get("missing_scopes"));
    let permission = matches!(error_type,"authorization"|"permission") || !missing.is_empty();
    let confirmed_gate = subtype == "confirmation_required" && output.exit_code == Some(10);
    let not_executed = !output.started || confirmed_gate;
    let message = if output.cancelled { "操作已取消，未取得完整业务回执".into() }
        else if output.timed_out { "飞书 CLI 等待回执超时，写入结果不能据此判定".into() }
        else if output.truncated { "飞书 CLI 返回不完整，无法确认完整结果".into() }
        else if output.cleanup_error.is_some() { "原业务进程的清理尚未得到确认".into() }
        else { error.get("message").and_then(Value::as_str).map(safe_message)
            .unwrap_or_else(|| "飞书 CLI 未返回可确认的结果".into()) };
    HubError { code: if permission { "OFFICE_PERMISSION" } else if mutating && !not_executed {
            "OFFICE_RESULT_UNKNOWN" } else { "OFFICE_UNAVAILABLE" }.into(),
        message, not_executed: if mutating { not_executed } else { true }, missing_scopes: missing,
        console_url: error.get("console_url").and_then(Value::as_str).and_then(official_url),
        related_operation_id: None, related_flow_id: None }
}
fn business_value(output: &CliOutput, mutating: bool) -> Result<Value, HubError> {
    let value = output_value(output).ok_or_else(|| output_error(output, mutating))?;
    if !output.successful() || value.get("ok") != Some(&Value::Bool(true)) {
        return Err(output_error(output, mutating));
    }
    value.get("data").cloned().ok_or_else(|| {
        if mutating { HubError::unknown("飞书 CLI 成功回执缺少业务数据") }
        else { HubError::unavailable("飞书 CLI 结果缺少业务数据") }
    })
}
/// Auth presentation only. A typed credential/policy error does not prove that
/// an unrelated mutation was never dispatched, so output_error remains the sole
/// mutation classifier.
pub fn auth_output_error(output:&CliOutput)->HubError{
    let mut error=output_error(output,false);
    if output.cancelled||output.timed_out||output.truncated||output.cleanup_error.is_some(){return error}
    let envelope=error_value(output).unwrap_or(Value::Null);
    let kind=envelope.pointer("/error/type").and_then(Value::as_str);
    let subtype=envelope.pointer("/error/subtype").and_then(Value::as_str);
    match (kind,subtype){
        (Some("authentication"),Some("token_expired"|"refresh_token_expired"))=>{
            error.code="OFFICE_AUTH_EXPIRED".into();
        },
        (Some("policy"),Some("access_denied"))=>{
            error.code="OFFICE_AUTH_DENIED".into();
        },
        _=>{},
    }
    error
}

pub fn parse_profiles(output: &CliOutput) -> Result<Vec<OfficeProfile>, HubError> {
    if !output.successful() { return Err(output_error(output, false)); }
    let value = output_value(output).ok_or_else(|| HubError::unavailable("无法解析飞书连接列表"))?;
    let entries = value.as_array().ok_or_else(|| HubError::unavailable("飞书连接列表格式不兼容"))?;
    entries.iter().map(|item| {
        let name = text(item.get("name")).ok_or_else(|| HubError::unavailable("连接条目缺少名称"))?;
        let app_id = text(item.get("appId")).ok_or_else(|| HubError::unavailable("连接条目缺少应用标识"))?;
        Ok(OfficeProfile { name, app_id, brand: text(item.get("brand")).unwrap_or_else(|| "feishu".into()),
            user_name: text(item.get("user")), token_status: text(item.get("tokenStatus")) })
    }).collect()
}
/// v1.0.98 lists users only in the selected profile. Its logout clears all of
/// those users, so the caller must prove this is a zero- or single-user profile.
pub fn parse_auth_users(output: &CliOutput) -> Result<Vec<OfficeActor>, HubError> {
    if !output.successful() { return Err(auth_output_error(output)); }
    let value = output_value(output).ok_or_else(|| HubError::unavailable("无法核验此连接的登录账号，请刷新后重试"))?;
    let entries = if let Some(entries) = value.as_array() { entries }
        else if value.get("ok") == Some(&Value::Bool(true))
            && matches!(value.get("reason").and_then(Value::as_str), Some("not_logged_in" | "not_configured")) {
            value.get("users").and_then(Value::as_array).filter(|users| users.is_empty())
                .ok_or_else(|| HubError::unavailable("飞书账号列表格式不兼容，请刷新后重试"))?
        } else { return Err(HubError::unavailable("飞书账号列表格式不兼容，请刷新后重试")); };
    entries.iter().map(|item| {
        let app_id = text(item.get("appId")).ok_or_else(|| HubError::unavailable("登录账号缺少应用标识，无法安全退出"))?;
        let user_id = text(item.get("userOpenId")).ok_or_else(|| HubError::unavailable("登录账号缺少用户标识，无法安全退出"))?;
        Ok(OfficeActor { app_id, user_id, name: text(item.get("userName")).unwrap_or_default() })
    }).collect()
}
pub fn parse_logout(output: &CliOutput) -> Result<(), HubError> {
    if !output.successful() { return Err(output_error(output, true)); }
    let value = output_value(output).ok_or_else(|| HubError::unknown("退出结果无法确认，请刷新连接状态后重试"))?;
    if value.get("ok") == Some(&Value::Bool(true))
        && (value.get("loggedOut") == Some(&Value::Bool(true))
            || (value.get("loggedOut") == Some(&Value::Bool(false))
                && matches!(value.get("reason").and_then(Value::as_str), Some("not_logged_in" | "not_configured")))) {
        Ok(())
    } else { Err(HubError::unknown("退出结果无法确认，请刷新连接状态后重试")) }
}
pub fn parse_identity(output: &CliOutput) -> Result<CliIdentity, HubError> {
    if !output.successful() {
        let problem = error_value(output).unwrap_or(Value::Null);
        if problem.pointer("/error/type").and_then(Value::as_str) == Some("config")
            && problem.pointer("/error/subtype").and_then(Value::as_str) == Some("not_configured")
        {
            return Ok(CliIdentity { actor: None, phase: ConnectionPhase::NotConfigured, scopes: Vec::new(),
                message: Some("请先配置飞书企业应用".into()) });
        }
        return Err(auth_output_error(output));
    }
    let value = output_value(output).ok_or_else(|| HubError::unavailable("无法解析飞书身份状态"))?;
    let user = value.pointer("/identities/user").unwrap_or(&Value::Null);
    let scopes = strings(user.get("scope"));
    let status = user.get("status").and_then(Value::as_str).unwrap_or("");
    let available = user.get("available").and_then(Value::as_bool).unwrap_or(false);
    let verification_failed = user.get("verified") == Some(&Value::Bool(false))
        || value.get("verified") == Some(&Value::Bool(false));
    let actor = if available && !verification_failed {
        match (text(value.get("appId")), text(user.get("openId"))) {
            (Some(app_id), Some(user_id)) => Some(OfficeActor { app_id, user_id,
                name: text(user.get("userName")).unwrap_or_else(|| "飞书用户".into()) }),
            _ => None,
        }
    } else { None };
    let phase = if actor.is_some() { ConnectionPhase::Ready }
        else if matches!(status, "expired" | "refresh_expired") { ConnectionPhase::Expired }
        else if verification_failed || status == "error" { ConnectionPhase::Error }
        else { ConnectionPhase::NeedsLogin };
    Ok(CliIdentity { actor, phase, scopes,
        message: first_text(user, &["message", "hint"]).map(|s| safe_message(&s))
            .or_else(|| text(value.get("note")).map(|s| safe_message(&s))) })
}
pub fn parse_login_start(output: &CliOutput) -> Result<LoginStart, HubError> {
    if !output.successful() { return Err(auth_output_error(output)); }
    let value = output_value(output).ok_or_else(|| HubError::unavailable("无法解析本次授权流程"))?;
    let verification_url = value.get("verification_url").and_then(Value::as_str).and_then(official_url)
        .ok_or_else(|| HubError::unavailable("CLI 未返回有效的飞书官方授权地址"))?;
    let device_code = text(value.get("device_code")).ok_or_else(|| HubError::unavailable("本次授权缺少设备标识"))?;
    let expires_in = value.get("expires_in").and_then(Value::as_u64).filter(|n| *n > 0 && *n <= 3600)
        .ok_or_else(|| HubError::unavailable("本次授权缺少有效期"))?;
    Ok(LoginStart { verification_url, device_code, expires_in })
}
pub fn parse_login_completion(output: &CliOutput, app_id: &str) -> Result<LoginCompletion, HubError> {
    // Partial grants can be a nonzero exit and still be a completed login.
    let value = json_values(&output.stdout).into_iter()
        .find(|v| v.get("event").and_then(Value::as_str) == Some("authorization_complete"))
        .ok_or_else(|| auth_output_error(output))?;
    if output.timed_out || output.cancelled || output.truncated || output.cleanup_error.is_some() {
        return Err(auth_output_error(output));
    }
    let user_id = text(value.get("user_open_id")).ok_or_else(|| HubError::unavailable("授权完成但缺少用户身份"))?;
    let missing_scopes = strings(value.get("missing"));
    let complete = missing_scopes.is_empty() && output.exit_code == Some(0);
    Ok(LoginCompletion { actor: Some(OfficeActor { app_id: app_id.into(), user_id,
            name: text(value.get("user_name")).unwrap_or_else(|| "飞书用户".into()) }),
        missing_scopes, complete, message: value.pointer("/warning/hint").and_then(Value::as_str).map(safe_message) })
}
pub fn parse_configured(output: &CliOutput) -> Result<String, HubError> {
    if !output.successful() { return Err(auth_output_error(output)); }
    output_value(output).and_then(|v| text(v.get("appId")))
        .ok_or_else(|| HubError::unavailable("应用配置未返回有效的应用标识"))
}

pub fn query_commands(query: &OfficeQuery, profile: Option<&str>) -> Result<Vec<CliCommand>, HubError> {
    let commands = match query {
        OfficeQuery::Documents { query, filter, cursor } => {
            if query.chars().count() > 30 || query.contains('\0') { return Err(HubError::invalid("搜索关键词最多 30 个字符")); }
            // Empty-query browsing and keyword search share the official Search
            // v2 index. A Drive root's direct children are not a document list.
            let types=match filter {
                DocumentFilter::All=>"doc,docx,sheet,bitable,mindnote,file,wiki,slides",
                DocumentFilter::Document=>"doc,docx,sheet,mindnote,file,wiki,slides",
                DocumentFilter::Base=>"bitable",
            };
            let mut args=vec!["drive".into(), "+search".into(), "--query".into(), query.clone(),
                "--doc-types".into(), types.into(), "--sort".into(), "edit_time".into(),
                "--page-size".into(), "20".into(), "--format".into(), "json".into()];
            if let Some(cursor)=cursor { valid_page_token(cursor)?;args.extend(["--page-token".into(),cursor.clone()]); }
            vec![user_command(args,profile,false)?]
        }
        OfficeQuery::People { query } => {
            valid_text(query, "人员关键词", 50)?;
            vec![user_command(["contact", "+search-user", "--query", query,
                "--exclude-external-users", "--page-size", "20", "--format", "json"], profile, false)?]
        }
        OfficeQuery::Tasklists { cursor } => {
            let mut params = json!({"page_size":50,"user_id_type":"open_id"});
            if let Some(cursor) = cursor { valid_page_token(cursor)?;params["page_token"] = json!(cursor); }
            vec![user_command(["task".into(), "tasklists".into(), "list".into(), "--params".into(),
                params.to_string()], profile, false)?]
        }
        OfficeQuery::Tasks { relation, tasklist_id, cursor } => {
            let mut args = match relation {
                TaskRelation::Assigned => vec!["task".into(), "+get-my-tasks".into(), "--page-limit".into(), "1".into(), "--format".into(), "json".into()],
                TaskRelation::Created => vec!["task".into(), "+get-related-tasks".into(), "--created-by-me".into(), "--page-limit".into(), "1".into(), "--format".into(), "json".into()],
                TaskRelation::Tasklist => {
                    let id = tasklist_id.as_deref().ok_or_else(|| HubError::invalid("请选择确切任务清单"))?;
                    valid_id(id, "任务清单标识")?;
                    let mut params = json!({"tasklist_guid":id,"page_size":50,"user_id_type":"open_id"});
                    if let Some(cursor) = cursor { valid_page_token(cursor)?;params["page_token"] = json!(cursor); }
                    return Ok(vec![user_command(["task".into(),"tasklists".into(),"tasks".into(),"--params".into(),params.to_string()],profile,false)?]);
                }
            };
            if let Some(cursor) = cursor { valid_page_token(cursor)?;args.extend(["--page-token".into(),cursor.clone()]); }
            vec![user_command(args, profile, false)?]
        }
        OfficeQuery::Agenda { start, end } => {
            let from = parse_datetime(start)?;let to = parse_datetime(end)?;
            if to <= from || (to - from).num_days() > 90 { return Err(HubError::invalid("日程时间范围无效或超过 90 天")); }
            vec![user_command(["calendar","+agenda","--start",start,"--end",end,"--format","json"],profile,false)?]
        }
        OfficeQuery::CalendarEvent { calendar_id, event_id } => {
            valid_id(calendar_id,"日历标识")?;valid_id(event_id,"日程标识")?;
            vec![user_command(["calendar","+get","--calendar-id",calendar_id,"--event-id",event_id,"--format","json"],profile,false)?]
        }
        OfficeQuery::BaseRecords { base_token, table_id, offset, filter } => {
            valid_id(base_token,"多维表格标识")?;valid_id(table_id,"数据表标识")?;
            let fields = user_command(["base","+field-list","--base-token",base_token,"--table-id",table_id],profile,false)?;
            let mut args = vec!["base".into(),"+record-list".into(),"--base-token".into(),base_token.clone(),
                "--table-id".into(),table_id.clone(),"--format".into(),"json".into(),"--limit".into(),"100".into(),
                "--offset".into(),offset.unwrap_or(0).to_string()];
            if let Some(filter) = filter {
                if !filter.is_object() || filter.to_string().len() > 4096 { return Err(HubError::invalid("表格筛选条件格式不正确或过长")); }
                args.extend(["--filter-json".into(),filter.to_string()]);
            }
            vec![fields,user_command(args,profile,false)?]
        }
    };
    Ok(commands)
}

fn plain_highlight(value: &str) -> String {
    regex::Regex::new(r"</?h(?:b)?>").expect("static highlighted title pattern").replace_all(value,"").into_owned()
}
fn task_item(value: &Value) -> Result<OfficeItem, HubError> {
    let id = first_text(value,&["guid","task_guid"]).ok_or_else(|| HubError::unavailable("任务结果缺少稳定标识"))?;
    let mut item = OfficeItem::new(id,"task",first_text(value,&["summary","title"]).unwrap_or_default());
    item.url = value.get("url").and_then(Value::as_str).and_then(official_url);
    item.description = text(value.get("description"));
    item.creator_id = value.pointer("/creator/id").and_then(Value::as_str).map(str::to_owned)
        .or_else(|| text(value.get("creator_id")));
    item.due = first_text(value,&["due_at"]).or_else(|| value.pointer("/due/timestamp").and_then(Value::as_str).map(str::to_owned));
    item.completed = value.get("completed").and_then(Value::as_bool).or_else(|| {
        value.get("completed_at").and_then(Value::as_str).map(|s| !s.is_empty() && s != "0")
    });
    item.start = first_text(value,&["start_at"]).or_else(|| text(value.pointer("/start/timestamp")));
    if let Some(members) = value.get("members").and_then(Value::as_array) {
        item.assignee_ids = members.iter().filter(|v| v.get("role").and_then(Value::as_str)==Some("assignee"))
            .filter_map(|v| first_text(v,&["id","open_id"])).collect();
    }
    if let Some(lists) = value.get("tasklists").and_then(Value::as_array) {
        item.tasklist_ids = lists.iter().filter_map(|v| first_text(v,&["tasklist_guid","guid"])).collect();
    }
    Ok(item)
}
fn event_item(value: &Value, calendar: Option<&str>) -> Result<OfficeItem, HubError> {
    let id = first_text(value,&["event_id"]).ok_or_else(|| HubError::unavailable("日程缺少稳定标识"))?;
    let mut item = OfficeItem::new(id,"calendar_event",first_text(value,&["summary","title"]).unwrap_or_default());
    item.calendar_id = first_text(value,&["calendar_id"]).or_else(|| calendar.map(str::to_owned));
    item.url = first_text(value,&["app_link","url"]).and_then(|url| official_url(&url));
    item.description = first_text(value,&["description_rich","description"]);
    let time = |key: &str| value.get(key).and_then(|v| first_text(v,&["datetime","date","timestamp"]));
    item.start = time("start_time");item.end = time("end_time");
    Ok(item)
}
fn warnings(data: &Value) -> Vec<String> {
    let mut result = strings(data.get("warnings"));
    if let Some(warnings) = data.get("warnings").and_then(Value::as_array) {
        for warning in warnings {
            if let Some(message) = first_text(warning,&["message","hint"]) { result.push(safe_message(&message)); }
            else if !warning.is_string(){result.push("CLI 返回了需核验的未识别警告".into());}
        }
    }
    if let Some(notice) = text(data.get("notice")) { result.push(safe_message(&notice)); }
    result
}
fn array_field<'a>(value: &'a Value, key: &str, message: &str) -> Result<&'a [Value], HubError> {
    match value.get(key) {
        Some(Value::Array(items)) => Ok(items),
        // Go's empty slice may serialize to null, notably +get-my-tasks.
        Some(Value::Null) => Ok(&[]),
        _ => Err(HubError::unavailable(message)),
    }
}
pub fn normalize_query(query: &OfficeQuery, outputs: &[CliOutput]) -> Result<OfficeQueryResult, HubError> {
    if outputs.is_empty() { return Err(HubError::unavailable("办公查询缺少 CLI 返回")); }
    let data = business_value(outputs.last().unwrap(),false)?;
    let mut result = OfficeQueryResult { has_more: data.get("has_more").and_then(Value::as_bool).unwrap_or(false),
        next_cursor: first_text(&data,&["page_token","next_page_token","next_offset"]),
        warnings: warnings(&data), ..OfficeQueryResult::default() };
    match query {
        OfficeQuery::Documents { cursor, .. } => {
            result=normalize_documents(&data,cursor.as_deref())?;
        }
        OfficeQuery::People { .. } => {
            for user in array_field(&data,"users","人员结果格式不兼容")? {
                if user.get("is_cross_tenant")==Some(&Value::Bool(true)) { continue; }
                let id=text(user.get("open_id")).ok_or_else(|| HubError::unavailable("人员结果缺少唯一身份"))?;
                let mut item=OfficeItem::new(id,"person",first_text(user,&["localized_name","name"]).unwrap_or_default());
                item.description=text(user.get("department"));result.items.push(item);
            }
        }
        OfficeQuery::Tasklists { .. } => {
            for list in array_field(&data,"items","任务清单格式不兼容")? {
                let id=first_text(list,&["guid","tasklist_guid"]).ok_or_else(|| HubError::unavailable("任务清单缺少稳定标识"))?;
                let mut item=OfficeItem::new(id,"tasklist",first_text(list,&["name","summary"]).unwrap_or_default());
                item.url=list.get("url").and_then(Value::as_str).and_then(official_url);result.items.push(item);
            }
        }
        OfficeQuery::Tasks { .. } => {
            for task in array_field(&data,"items","任务列表格式不兼容")? {
                result.items.push(task_item(task)?);
            }
        }
        OfficeQuery::Agenda { .. } => {
            for event in data.as_array().ok_or_else(|| HubError::unavailable("日程列表格式不兼容"))? {
                result.items.push(event_item(event,Some("primary"))?);
            }
        }
        OfficeQuery::CalendarEvent { calendar_id, .. } => {
            let event=data.get("event").unwrap_or(&data);
            result.items.push(event_item(event,Some(calendar_id))?);
        }
        OfficeQuery::BaseRecords { base_token,table_id,offset,.. } => {
            if outputs.len()!=2 { return Err(HubError::unavailable("表格查询必须包含字段结构与记录结果")); }
            let fields=business_value(&outputs[0],false)?;
            result=normalize_matrix(&data)?;
            if let Some(entries)=fields.get("fields").and_then(Value::as_array) {
                result.fields=entries.iter().map(field).collect::<Result<_,_>>()?;
            }else{return Err(HubError::unavailable("表格字段结构格式不兼容"))}
            for item in &mut result.items { item.base_token=Some(base_token.clone());item.table_id=Some(table_id.clone()); }
            if result.has_more && result.next_cursor.is_none() {
                result.next_cursor=Some(offset.unwrap_or(0).saturating_add(result.items.len() as u32).to_string());
            }
        }
    }
    Ok(result)
}
/// Exact CLI v1.0.98 Search v2 projection. Result identity is the returned
/// resource token; browser URLs and highlight markup never become identities.
fn normalize_documents(data:&Value,cursor:Option<&str>)->Result<OfficeQueryResult,HubError>{
    let has_more=data.get("has_more").and_then(Value::as_bool)
        .ok_or_else(||HubError::unavailable("文档查询缺少有效的分页状态"))?;
    let next_cursor=if has_more {
        let next=text(data.get("page_token")).filter(|next|Some(next.as_str())!=cursor)
            .ok_or_else(||HubError::unavailable("文档查询分页信息不完整或重复，请重新读取"))?;
        valid_page_token(&next)?;
        Some(next)
    }else{None};
    let mut result=OfficeQueryResult{has_more,next_cursor,warnings:warnings(data),..Default::default()};
    let mut seen=BTreeSet::new();
    for entry in array_field(data,"results","文档查询结果格式不兼容")? {
        let meta=entry.get("result_meta").filter(|meta|meta.is_object())
            .ok_or_else(||HubError::unavailable("文档查询缺少资源元数据"))?;
        let id=text(meta.get("token")).ok_or_else(||HubError::unavailable("文档查询缺少资源 token"))?;
        valid_resource_token(&id)?;
        let native_type=meta.get("doc_types").and_then(Value::as_str)
            .ok_or_else(||HubError::unavailable("文档查询缺少资源类型"))?.to_ascii_lowercase();
        let url=meta.get("url").and_then(Value::as_str).and_then(official_url);
        let wiki=entry.get("entity_type").and_then(Value::as_str)==Some("WIKI")
            || url.as_deref().and_then(|url|link_target(url).ok()).is_some_and(|(kind,_)|kind=="wiki");
        let kind=if native_type=="bitable"{"base"}else if wiki||native_type=="wiki"{"wiki"}
            else if document_type(&native_type){native_type.as_str()}
            else{return Err(HubError::unavailable("文档查询返回了不支持的资源类型"))};
        if !seen.insert((kind.to_owned(),id.clone())) { continue; }
        let mut item=OfficeItem::new(id,kind,plain_highlight(&first_text(entry,&["title","title_highlighted"]).unwrap_or_default()));
        item.url=url;item.modified_at=first_text(meta,&["update_time_iso","update_time"]);
        if kind=="base" { if let Some(url)=&item.url { let (base,table)=base_coordinates(url);item.base_token=base;item.table_id=table; } }
        result.items.push(item);
    }
    Ok(result)
}
fn base_coordinates(url: &str) -> (Option<String>,Option<String>) {
    let base=url.split("/base/").nth(1).and_then(|s|s.split(['?','#','/']).next()).filter(|s|!s.is_empty()).map(str::to_owned);
    let table=url.split('?').nth(1).and_then(|query|query.split('&').find_map(|p|p.strip_prefix("table="))).filter(|s|!s.is_empty()).map(str::to_owned);
    (base,table)
}
fn writable_type(kind: &str) -> bool {
    matches!(kind,"text"|"number"|"select"|"checkbox"|"datetime"|"phone"|"url")
}
fn field(value: &Value) -> Result<OfficeField, HubError> {
    let id=first_text(value,&["id","field_id"]).ok_or_else(||HubError::unavailable("表格字段缺少标识"))?;
    let kind=first_text(value,&["type"]).unwrap_or_else(||"not_support".into());
    Ok(OfficeField {id,name:first_text(value,&["name","field_name"]).unwrap_or_default(),
        writable:writable_type(&kind),field_type:kind,options:value.get("options").cloned()
            .or_else(||value.pointer("/property/options").cloned())})
}
fn normalize_matrix(data: &Value) -> Result<OfficeQueryResult, HubError> {
    let names=data.get("fields").and_then(Value::as_array).ok_or_else(||HubError::unavailable("表格结果缺少字段名称"))?;
    let ids=data.get("field_id_list").and_then(Value::as_array).ok_or_else(||HubError::unavailable("表格结果缺少字段标识"))?;
    let types=data.get("field_type_list").and_then(Value::as_array).ok_or_else(||HubError::unavailable("表格结果缺少字段类型"))?;
    let record_ids=data.get("record_id_list").and_then(Value::as_array).ok_or_else(||HubError::unavailable("表格结果缺少记录标识"))?;
    let rows=data.get("data").and_then(Value::as_array).ok_or_else(||HubError::unavailable("表格结果缺少记录矩阵"))?;
    if names.len()!=ids.len()||ids.len()!=types.len()||record_ids.len()!=rows.len() {
        return Err(HubError::unavailable("表格结果的结构长度不一致"));
    }
    let mut result=OfficeQueryResult {has_more:data.get("has_more").and_then(Value::as_bool).unwrap_or(false),
        next_cursor:first_text(data,&["next_offset"]),warnings:warnings(data),..OfficeQueryResult::default()};
    for n in 0..ids.len() {
        let id=ids[n].as_str().ok_or_else(||HubError::unavailable("字段标识格式错误"))?;
        let name=names[n].as_str().ok_or_else(||HubError::unavailable("字段名称格式错误"))?;
        let kind=types[n].as_str().ok_or_else(||HubError::unavailable("字段类型格式错误"))?;
        result.fields.push(OfficeField {id:id.into(),name:name.into(),field_type:kind.into(),
            writable:writable_type(kind),options:None});
    }
    for (n,row) in rows.iter().enumerate() {
        let id=record_ids[n].as_str().ok_or_else(||HubError::unavailable("记录标识格式错误"))?;
        let row=row.as_array().filter(|row|row.len()==ids.len()).ok_or_else(||HubError::unavailable("表格记录列数与结构不一致"))?;
        let title=row.iter().find_map(|v|v.as_str().filter(|s|!s.is_empty())).unwrap_or(id);
        let mut item=OfficeItem::new(id,"base_record",title);
        for (column,value) in result.fields.iter().zip(row) { item.fields.insert(column.id.clone(),value.clone()); }
        result.items.push(item);
    }
    if let Some(missing)=data.get("record_not_found").and_then(Value::as_array) {
        if !missing.is_empty() {result.warnings.push("部分原记录本次未读回；不能据此断言写入未执行".into());}
    }
    Ok(result)
}

fn parse_datetime(value:&str)->Result<DateTime<chrono::FixedOffset>,HubError>{
        DateTime::parse_from_rfc3339(value)
            .or_else(|_| DateTime::parse_from_str(value,"%Y-%m-%dT%H:%M%:z"))
            .map_err(|_|HubError::invalid("日期时间必须包含明确时区偏移"))
}
pub fn mutation_command(mutation:&OfficeMutation,client_token:&str,profile:Option<&str>)->Result<CliCommand,HubError>{
    valid_id(client_token,"操作幂等标识")?;
    match mutation {
        OfficeMutation::TaskCreate {title,assignee_id,due,tasklist_id,..}=>{
            valid_text(title,"任务内容",2000)?;valid_id(assignee_id,"负责人标识")?;
            if !assignee_id.starts_with("ou_"){return Err(HubError::invalid("负责人必须是已确认的飞书用户身份"))}
            if let Some(due)=due { chrono::NaiveDate::parse_from_str(due,"%Y-%m-%d").map_err(|_|HubError::invalid("任务截止日期无效"))?; }
            let mut args=vec!["task".into(),"+create".into(),"--summary".into(),title.clone(),"--assignee".into(),assignee_id.clone(),
                "--idempotency-key".into(),client_token.into(),"--format".into(),"json".into()];
            if let Some(due)=due { args.extend(["--due".into(),due.clone()]); }
            if let Some(list)=tasklist_id {valid_id(list,"任务清单标识")?;args.extend(["--tasklist-id".into(),list.clone()]);}
            user_command(args,profile,true)
        }
        OfficeMutation::TaskComplete {task_id,completed,..}=>{
            valid_id(task_id,"任务标识")?;
            user_command(["task",if *completed{"+complete"}else{"+reopen"},"--task-id",task_id,"--format","json"],profile,true)
        }
        OfficeMutation::DocumentCreate {title,..}=>{
            valid_text(title,"文档标题",500)?;
            user_command(["docs","+create","--title",title,"--doc-format","xml",
                "--parent-position","my_library"],profile,true)
        }
        OfficeMutation::DocumentDelete {document_id,document_type:kind,title,..}=>{
            valid_resource_token(document_id)?;
            valid_text(title,"文档标题",500)?;
            if !document_type(kind) { return Err(HubError::invalid("此资源类型不支持文档删除")); }
            user_command(["drive","+delete","--file-token",document_id,"--type",kind,"--yes"],profile,true)
        }
        OfficeMutation::CalendarCreate {title,start,end,timezone,..}=>{
            valid_text(title,"日程名称",500)?;valid_id(timezone,"时区")?;
            let from=parse_datetime(start)?;let to=parse_datetime(end)?;
            if to<=from{return Err(HubError::invalid("日程结束时间必须晚于开始时间"))}
            let data=json!({"summary":title,"start_time":{"timestamp":from.timestamp().to_string(),"timezone":timezone},
                "end_time":{"timestamp":to.timestamp().to_string(),"timezone":timezone},"vchat":{"vc_type":"no_meeting"}});
            user_command(["calendar".into(),"events".into(),"create".into(),"--params".into(),
                json!({"calendar_id":"primary","idempotency_key":client_token,"user_id_type":"open_id"}).to_string(),
                "--data".into(),data.to_string()],profile,true)
        }
        OfficeMutation::BaseUpdate {base_token,table_id,record_id,fields,before,..}=>{
            valid_id(base_token,"多维表格标识")?;valid_id(table_id,"数据表标识")?;valid_id(record_id,"记录标识")?;
            validate_base_fields(fields,before)?;
            for key in fields.keys(){valid_id(key,"字段标识")?}
            let mut updates=serde_json::Map::new();updates.insert(record_id.clone(),json!(fields));
            let mut result=user_command(["base".into(),"+record-batch-update".into(),"--base-token".into(),base_token.clone(),
                "--table-id".into(),table_id.clone(),"--json".into(),format!("@{PAYLOAD_FILE}")],profile,true)?;
            result.input_file=Some(json!({"update_records":updates}).to_string().into_bytes());Ok(result)
        }
    }
}
pub fn normalize_mutation(mutation:&OfficeMutation,output:&CliOutput)->Result<MutationReceipt,HubError>{
    let envelope=output_value(output);
    let data=envelope.as_ref().and_then(|v|v.get("data")).cloned().unwrap_or(Value::Null);
    let (id,url)=match mutation {
        OfficeMutation::TaskCreate {..}|OfficeMutation::TaskComplete {..}=>
            (first_text(&data,&["guid"]),data.get("url").and_then(Value::as_str).and_then(official_url)),
        OfficeMutation::DocumentCreate {..}=>(text(data.pointer("/document/document_id")),data.pointer("/document/url").and_then(Value::as_str).and_then(official_url)),
        OfficeMutation::DocumentDelete {document_id,..}=>(Some(document_id.clone()),None),
        OfficeMutation::CalendarCreate {..}=>(text(data.pointer("/event/event_id")),data.pointer("/event/app_link").and_then(Value::as_str).and_then(official_url)),
        OfficeMutation::BaseUpdate {record_id,..}=>(Some(record_id.clone()),None),
    };
    if !output.successful() || envelope.as_ref().and_then(|v|v.get("ok"))!=Some(&Value::Bool(true)) {
        let error=output_error(output,true);
        // An exact original resource identity remains useful even when output or
        // native cleanup is incomplete. Keep it without manufacturing success.
        if id.is_some() && !data.is_null() {
            return Ok(MutationReceipt {status:ItemStatus::Unknown,remote_id:id,remote_url:url,
                message:Some(error.message),result:envelope.unwrap_or(Value::Null)});
        }
        return Err(error);
    }
    if data.is_null(){return Err(HubError::unknown("成功回执缺少业务数据"))}
    let result=data.get("result").and_then(Value::as_str);
    let warning_messages=warnings(&data);
    let status=if result.is_some_and(|result|result!="success") {ItemStatus::Unknown}
        else if matches!(mutation,OfficeMutation::BaseUpdate{..}) {ItemStatus::AcceptedPendingConfirmation}
        else if !warning_messages.is_empty(){ItemStatus::Unknown}
        else if id.is_none(){ItemStatus::Unknown}
        else if let OfficeMutation::DocumentDelete {document_id,document_type,..}=mutation {
            if data.get("deleted")==Some(&Value::Bool(true)) && data.get("file_token").and_then(Value::as_str)==Some(document_id)
                && data.get("type").and_then(Value::as_str)==Some(document_type)
                && data.get("ready")!=Some(&Value::Bool(false)) && data.get("timed_out")!=Some(&Value::Bool(true))
                && data.get("failed")!=Some(&Value::Bool(true)) && data.get("status").and_then(Value::as_str).is_none_or(|status|status=="success")
                && data.get("next_command").is_none() {ItemStatus::Succeeded}
            else {ItemStatus::Unknown}
        }
        else {ItemStatus::Succeeded};
    let message=if status==ItemStatus::Unknown {Some(if !warning_messages.is_empty(){warning_messages.join("；")}
            else if let Some(result)=result{format!("CLI 返回 {result}，尚不能确认完整结果；请核验原操作")}
            else{"尚未取得可以确认完整结果的业务回执".into()})}
        else if matches!(mutation,OfficeMutation::BaseUpdate{..}) {Some("批次已返回，尚需按原记录与字段读回确认".into())}
        else if !warnings(&data).is_empty(){Some(warnings(&data).join("；"))}else{None};
    Ok(MutationReceipt {status,remote_id:id,remote_url:url,message,result:envelope.unwrap_or(Value::Null)})
}

/// One real Base batch request; the domain owner preserves each item's distinct
/// attempt and associates the shared batch acknowledgement with all of them.
pub fn base_batch_command(items:&[OfficeMutation],profile:Option<&str>)->Result<CliCommand,HubError>{
    if items.is_empty()||items.len()>200{return Err(HubError::invalid("单次表格更新必须包含 1 至 200 条记录"))}
    let mut base=None;let mut table=None;let mut updates=serde_json::Map::new();
    for mutation in items {
        let OfficeMutation::BaseUpdate{base_token,table_id,record_id,fields,before,..}=mutation else{
            return Err(HubError::invalid("表格批次不能混入其他办公操作"))
        };
        valid_id(base_token,"多维表格标识")?;valid_id(table_id,"数据表标识")?;valid_id(record_id,"记录标识")?;
        if base.is_some_and(|value:&str|value!=base_token)||table.is_some_and(|value:&str|value!=table_id){
            return Err(HubError::invalid("表格批次必须属于同一个 Base 和数据表"))
        }
        base=Some(base_token.as_str());table=Some(table_id.as_str());
        validate_base_fields(fields,before)?;
        for key in fields.keys(){valid_id(key,"字段标识")?}
        if updates.insert(record_id.clone(),json!(fields)).is_some(){
            return Err(HubError::invalid("表格批次包含重复记录，拒绝覆盖其中一项"))
        }
    }
    let mut result=user_command(["base".into(),"+record-batch-update".into(),"--base-token".into(),base.unwrap().into(),
        "--table-id".into(),table.unwrap().into(),"--json".into(),format!("@{PAYLOAD_FILE}")],profile,true)?;
    result.input_file=Some(json!({"update_records":updates}).to_string().into_bytes());Ok(result)
}
fn validate_base_fields(fields:&BTreeMap<String,Value>,before:&BTreeMap<String,Value>)->Result<(),HubError>{
    if fields.is_empty()||fields.len()>50||fields.keys().ne(before.keys()){
        return Err(HubError::invalid("每个待修改字段都必须携带读取时的确切原值；请刷新记录后再保存"))
    }
    Ok(())
}
pub fn base_readback_command(base_token:&str,table_id:&str,record_ids:&[String],profile:Option<&str>)->Result<CliCommand,HubError>{
    valid_id(base_token,"多维表格标识")?;valid_id(table_id,"数据表标识")?;
    if record_ids.is_empty()||record_ids.len()>200{return Err(HubError::invalid("读回必须指定 1 至 200 条确切记录"))}
    let mut args:Vec<String>=vec!["base".into(),"+record-get".into(),"--base-token".into(),base_token.into(),
        "--table-id".into(),table_id.into(),"--format".into(),"json".into()];
    for id in record_ids {valid_id(id,"记录标识")?;args.extend(["--record-id".into(),id.clone()]);}
    user_command(args,profile,false)
}
pub fn normalize_base_readback(output:&CliOutput)->Result<OfficeQueryResult,HubError>{
    normalize_matrix(&business_value(output,false)?)
}
pub fn task_get_command(task_id:&str,profile:Option<&str>)->Result<CliCommand,HubError>{
    valid_id(task_id,"任务标识")?;
    user_command(["task".into(),"tasks".into(),"get".into(),"--params".into(),
        json!({"task_guid":task_id,"user_id_type":"open_id"}).to_string()],profile,false)
}
pub fn normalize_task_get(output:&CliOutput)->Result<OfficeItem,HubError>{
    let data=business_value(output,false)?;task_item(data.get("task").unwrap_or(&data))
}
pub fn tasklist_get_command(tasklist_id:&str,profile:Option<&str>)->Result<CliCommand,HubError>{
    valid_id(tasklist_id,"任务清单标识")?;
    user_command(["task".into(),"tasklists".into(),"get".into(),"--params".into(),
        json!({"tasklist_guid":tasklist_id,"user_id_type":"open_id"}).to_string()],profile,false)
}
pub fn tasklist_editable(output:&CliOutput,user_id:&str)->Result<bool,HubError>{
    let data=business_value(output,false)?;let list=data.get("tasklist").unwrap_or(&data);
    if list.pointer("/owner/id").and_then(Value::as_str)==Some(user_id){return Ok(true)}
    Ok(list.get("members").and_then(Value::as_array).is_some_and(|members|members.iter().any(|member|
        member.get("id").and_then(Value::as_str)==Some(user_id) &&
            matches!(member.get("role").and_then(Value::as_str),Some("editor"|"owner")))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    fn output(data:Value)->CliOutput{CliOutput::fixture(0,json!({"ok":true,"identity":"user","data":data}).to_string(),"")}
    #[test]
    fn logout_uses_the_explicit_profile_and_requires_the_official_auth_response_shapes(){
        let users=auth_users_command("work").unwrap();assert_eq!(users.args,["--profile","work","auth","list","--json"]);assert!(!users.mutating);
        let logout=logout_command("work").unwrap();assert_eq!(logout.args,["--profile","work","auth","logout","--json"]);assert!(logout.mutating);
        let users=parse_auth_users(&CliOutput::fixture(0,r#"[{"appId":"cli_x","userOpenId":"ou_x","userName":"User","tokenStatus":"expired"}]"#," ")).unwrap();
        assert_eq!(users.len(),1);assert_eq!(users[0].app_id,"cli_x");assert_eq!(users[0].user_id,"ou_x");
        assert!(parse_auth_users(&CliOutput::fixture(0,r#"{"ok":true,"users":[],"reason":"not_logged_in"}"#,"")).unwrap().is_empty());
        for value in [json!({"ok":true,"users":[]}),json!([{"userName":"display name only"}]),json!({"ok":true,"data":[]})]{
            assert!(parse_auth_users(&CliOutput::fixture(0,value.to_string(),"")).is_err());
        }
        assert!(parse_logout(&CliOutput::fixture(0,r#"{"ok":true,"loggedOut":true}"#,"")).is_ok());
        assert!(parse_logout(&CliOutput::fixture(0,r#"{"ok":true,"loggedOut":false,"reason":"not_logged_in"}"#,"")).is_ok());
        assert!(parse_logout(&CliOutput::fixture(0,r#"{"ok":true}"#,"")).is_err());
        assert!(!parse_logout(&CliOutput{started:true,timed_out:true,..Default::default()}).unwrap_err().not_executed);
    }
    #[test]
    fn scope_mapping_separates_read_write_and_tasklist_permissions(){
        let p=permissions(&["task:task:read".into()]);
        assert!(p.iter().find(|p|p.capability=="tasks.read").unwrap().granted);
        assert!(!p.iter().find(|p|p.capability=="tasks.write").unwrap().granted);
        assert!(!p.iter().find(|p|p.capability=="tasklists.read").unwrap().granted);
        let p=permissions(&["calendar:calendar.event:create".into()]);
        assert!(p.iter().find(|p|p.capability=="calendar.write").unwrap().granted);
        assert!(!p.iter().find(|p|p.capability=="calendar.read").unwrap().granted);
    }
    #[test]
    fn browse_reuses_the_connected_users_search_scope_without_drive_root_permission(){
        let p=permissions(&["search:docs:read".into()]);
        let browse=p.iter().find(|p|p.capability=="documents.browse").unwrap();
        assert!(browse.granted);assert_eq!(browse.required_scopes,vec!["search:docs:read"]);
        assert!(p.iter().all(|p|!matches!(p.capability.as_str(),"documents.list"|"documents.search"|"documents.read"|"documents.write")));
        for scope in ["space:document:retrieve","docx:document:readonly","docx:document:write_only"] {assert!(!allowed_scopes().contains(&scope.into()));}
        let old=permissions(&["space:document:retrieve".into(),"drive:drive:readonly".into()]);
        assert!(!old.iter().find(|p|p.capability=="documents.browse").unwrap().granted);
        let deleting=permissions(&["space:document:delete".into()]);
        assert!(!deleting.iter().find(|p|p.capability=="documents.delete").unwrap().granted);
    }
    #[test]
    fn document_browse_and_search_share_the_official_cli_query_and_server_filters(){
        for (filter,types) in [(DocumentFilter::All,"doc,docx,sheet,bitable,mindnote,file,wiki,slides"),
            (DocumentFilter::Document,"doc,docx,sheet,mindnote,file,wiki,slides"),(DocumentFilter::Base,"bitable")] {
            for keyword in ["","项目"] {
                let query=OfficeQuery::Documents{query:keyword.into(),filter:filter.clone(),cursor:Some("page-two".into())};
                let commands=query_commands(&query,Some("work")).unwrap();
                assert_eq!(commands.len(),1);assert!(!commands[0].mutating);
                assert_eq!(commands[0].args,vec!["--profile","work","drive","+search","--query",keyword,
                    "--doc-types",types,"--sort","edit_time","--page-size","20","--format","json","--page-token","page-two","--as","user"]);
                assert!(query_permissions(&query,&["search:docs:read".into()]).iter().all(|permission|permission.granted));
            }
        }
    }
    fn search_entry(token:&str,kind:&str,entity:&str,path:&str)->Value{
        json!({"entity_type":entity,"title_highlighted":"Existing <hb>document</hb>","summary_highlighted":"Do not expose a body preview",
            "result_meta":{"token":token,"doc_types":kind,"url":format!("https://team.feishu.cn/{path}/{token}"),
                "update_time":1793000000,"update_time_iso":"2026-10-26T06:13:20+08:00"}})
    }
    #[test]
    fn real_search_v2_shape_keeps_wiki_and_drive_resources_and_uses_stable_tokens(){
        let query=OfficeQuery::Documents{query:String::new(),filter:DocumentFilter::All,cursor:None};
        let wiki=search_entry("wiki_real","DOCX","WIKI","wiki");
        let mut duplicate=wiki.clone();duplicate["result_meta"]["url"]=json!("https://team.feishu.cn/wiki/wiki_real?from=another-link");
        let result=normalize_query(&query,&[output(json!({"results":[wiki,search_entry("doc_real","DOCX","DOC","docx"),
            search_entry("base_real","BITABLE","DOC","base"),duplicate],"has_more":true,"page_token":"page-two"}))]).unwrap();
        assert_eq!(result.items.len(),3);assert_eq!(result.items[0].id,"wiki_real");assert_eq!(result.items[0].kind,"wiki");
        assert_eq!(result.items[0].title,"Existing document");assert!(document_target(&result.items[0]).is_none());
        assert_eq!(document_target(&result.items[1]),Some(("docx".into(),"doc_real".into())));
        assert_eq!(result.items[2].kind,"base");assert_eq!(result.next_cursor.as_deref(),Some("page-two"));
        assert!(result.items.iter().all(|item|item.description.is_none()));
    }
    #[test]
    fn incomplete_search_pages_and_retired_root_shapes_fail_instead_of_becoming_empty_lists(){
        let query=OfficeQuery::Documents{query:String::new(),filter:DocumentFilter::All,cursor:Some("same-page".into())};
        for data in [json!({"results":[],"has_more":true}),json!({"results":[],"has_more":true,"page_token":"same-page"}),
            json!({"results":[]}),json!({"files":[],"has_more":false}),
            json!({"results":[{"result_meta":{"token":"bad/url","doc_types":"DOCX"}}],"has_more":false})] {
            assert!(normalize_query(&query,&[output(data)]).is_err());
        }
        let empty=normalize_query(&query,&[output(json!({"results":null,"has_more":false,"page_token":"unused-last-page"}))]).unwrap();
        assert!(empty.items.is_empty());assert!(!empty.has_more);assert!(empty.next_cursor.is_none());
        assert!(serde_json::from_value::<OfficeQuery>(json!({"kind":"search","query":"","cursor":null})).is_err());
        assert!(serde_json::from_value::<OfficeQuery>(json!({"kind":"documents"})).is_err());
    }
    #[test]
    fn opaque_page_tokens_keep_the_provider_value_beyond_resource_id_limits(){
        let cursor="opaque/+=\"".repeat(90);
        assert!(cursor.len()>512);assert!(valid_id(&cursor,"resource").is_err());
        let first=OfficeQuery::Documents{query:String::new(),filter:DocumentFilter::All,cursor:None};
        let page=normalize_query(&first,&[output(json!({"results":[],"has_more":true,"page_token":cursor}))]).unwrap();
        assert_eq!(page.next_cursor.as_deref(),Some(cursor.as_str()));
        let next=OfficeQuery::Documents{query:String::new(),filter:DocumentFilter::All,cursor:page.next_cursor};
        let command=query_commands(&next,Some("work")).unwrap().remove(0);
        assert!(command.args.windows(2).any(|args|args[0]=="--page-token"&&args[1]==cursor));
        for invalid in [String::new(),"bad\0cursor".into(),"bad\ncursor".into(),"x".repeat(16*1024+1)] {
            assert!(valid_page_token(&invalid).is_err());
        }
    }
    /// Opt-in contract evidence against an existing user login. It only reads
    /// status and at most two metadata pages; no login, scope grant or write.
    #[tokio::test]
    #[ignore = "requires explicit CLI path/profile and an account with existing documents"]
    async fn live_document_discovery_read_only(){
        let executable=std::env::var("NOMIFUN_OFFICE_PROBE_CLI").expect("Set NOMIFUN_OFFICE_PROBE_CLI to the existing verified CLI");
        let profile=std::env::var("NOMIFUN_OFFICE_PROBE_PROFILE").expect("Set NOMIFUN_OFFICE_PROBE_PROFILE to the existing connected profile");
        let directory=tempfile::tempdir().unwrap();
        let runtime=crate::runtime::CliRuntime::new(directory.path().to_path_buf());
        let observed=runtime.inspect(Some(&executable)).await;
        assert_eq!(observed.phase,RuntimePhase::Ready,"Use an existing compatible CLI; this probe never installs");
        let executable=observed.executable.unwrap();
        let cancel=tokio_util::sync::CancellationToken::new();
        let identity=runtime.run(&executable,status_command(Some(&profile),false),cancel.child_token(),None).await.unwrap();
        let identity=parse_identity(&identity).unwrap();assert_eq!(identity.phase,ConnectionPhase::Ready);
        let mut cursor=None;let mut count=0;let mut kinds=BTreeMap::<String,usize>::new();
        for page in 1..=2 {
            let query=OfficeQuery::Documents{query:String::new(),filter:DocumentFilter::All,cursor:cursor.clone()};
            assert!(query_permissions(&query,&identity.scopes).iter().all(|permission|permission.granted),"Existing login needs search:docs:read; this probe never starts authorization");
            let mut outputs=Vec::new();
            for command in query_commands(&query,Some(&profile)).unwrap(){
                assert!(!command.mutating);
                outputs.push(runtime.run(&executable,command,cancel.child_token(),None).await.unwrap());
            }
            let result=normalize_query(&query,&outputs).unwrap();
            assert!(result.items.iter().all(|item|item.description.is_none()));
            println!("read-only page {page}: {} resources, has_more={}",result.items.len(),result.has_more);
            count+=result.items.len();for item in &result.items{*kinds.entry(item.kind.clone()).or_default()+=1;}
            if !result.has_more { break; }
            cursor=result.next_cursor;
        }
        assert!(count>0,"Use an account with existing documents to reproduce the list regression");
        println!("read-only projection: {count} resources; kinds={kinds:?}");
    }
    #[test]
    fn document_create_is_title_only_and_content_contracts_are_retired(){
        let mutation=OfficeMutation::DocumentCreate{client_item_id:"create".into(),title:"New document".into()};
        let cmd=mutation_command(&mutation,"request",Some("work")).unwrap();
        assert!(cmd.stdin.is_none());assert!(cmd.input_file.is_none());assert!(!cmd.args.contains(&"--content".into()));
        assert!(serde_json::from_value::<OfficeQuery>(json!({"kind":"document","document":"doc_real"})).is_err());
        assert!(serde_json::from_value::<OfficeMutation>(json!({"kind":"document_save","client_item_id":"save","document_id":"doc_real","revision":"1","block_id":"block","content":"text"})).is_err());
        assert!(serde_json::from_value::<OfficeMutation>(json!({"kind":"document_create","client_item_id":"create","title":"New","content":"retired"})).is_err());
    }
    #[test]
    fn document_delete_requires_an_exact_terminal_receipt(){
        let mutation=OfficeMutation::DocumentDelete{client_item_id:"delete".into(),document_id:"doc_real".into(),document_type:"docx".into(),title:"Document".into()};
        let cmd=mutation_command(&mutation,"request",Some("work")).unwrap();
        assert_eq!(cmd.args,vec!["--profile","work","drive","+delete","--file-token","doc_real","--type","docx","--yes","--as","user"]);
        assert!(cmd.mutating);
        let receipt=json!({"deleted":true,"file_token":"doc_real","type":"docx"});
        assert_eq!(normalize_mutation(&mutation,&output(receipt.clone())).unwrap().status,ItemStatus::Succeeded);
        for data in [json!({"task_id":"pending","ready":false}),json!({}),json!({"deleted":true,"file_token":"another","type":"docx"}),
            json!({"deleted":true,"file_token":"doc_real","type":"docx","status":"processing"}),
            json!({"deleted":true,"file_token":"doc_real","type":"docx","next_command":"untrusted command"})] {
            assert_eq!(normalize_mutation(&mutation,&output(data)).unwrap().status,ItemStatus::Unknown);
        }
        let mut timed_out=output(receipt);timed_out.timed_out=true;
        assert_eq!(normalize_mutation(&mutation,&timed_out).unwrap().status,ItemStatus::Unknown);
        for kind in ["wiki","folder","shortcut","bitable"] {
            let mut unsupported=mutation.clone();if let OfficeMutation::DocumentDelete{document_type,..}=&mut unsupported{*document_type=kind.into();}
            assert!(mutation_command(&unsupported,"request",None).is_err());
        }
    }
    #[test]
    fn document_targets_never_infer_deletion_authority_from_a_wiki_or_invalid_link(){
        let mut item=OfficeItem::new("wiki_real","wiki","Wiki");item.url=Some("https://team.feishu.cn/wiki/wiki_real?from=link".into());
        assert!(document_target(&item).is_none());
        for url in ["https://evil.test/docx/a","https://team.feishu.cn/docx/a/blocks","https://team.feishu.cn/","https://team.feishu.cn/docx/a%2Fb"] {
            item.url=Some(url.into());assert!(document_target(&item).is_none());
        }
    }
    #[test]
    fn profile_and_status_use_their_real_unwrapped_contracts(){
        let profiles=parse_profiles(&CliOutput::fixture(0,r#"[{"name":"work","appId":"cli_x","brand":"feishu","user":"Lin"}]"#,"")).unwrap();
        assert_eq!(profiles[0].name,"work");
        let identity=parse_identity(&CliOutput::fixture(0,json!({"appId":"cli_x","identity":"bot","identities":{"user":{"status":"missing","available":false},"bot":{"available":true}}}).to_string(),"")).unwrap();
        assert!(identity.actor.is_none());assert_eq!(identity.phase,ConnectionPhase::NeedsLogin);
    }
    #[test]
    fn partial_login_is_not_discarded_as_a_generic_nonzero_failure(){
        let value=json!({"event":"authorization_complete","user_open_id":"ou_x","user_name":"Lin","scope":"task:task:read","missing":["task:task:write"]});
        let completion=parse_login_completion(&CliOutput::fixture(4,value.to_string(),""),"cli_x").unwrap();
        assert!(!completion.complete);assert!(completion.actor.is_some());
        assert_eq!(completion.missing_scopes,["task:task:write"]);
    }
    #[test]
    fn original_auth_links_are_preserved_and_nonofficial_hosts_rejected(){
        let url="https://accounts.feishu.cn/path?redirect_uri=a%2Fb&user_code=x";
        assert_eq!(official_url(url).as_deref(),Some(url));
        for bad in ["http://accounts.feishu.cn/a","https://accounts.feishu.cn.evil.test/a","https://accounts.feishu.cn@evil.test/a","https://accounts.feishu.cn\\@evil.test/a"]{assert!(official_url(bad).is_none())}
    }

    #[test]
    fn auth_expiry_and_denial_use_typed_subtypes_without_mutation_shortcuts(){
        let expired=CliOutput::fixture(4,"",r#"{"ok":false,"error":{"type":"authentication","subtype":"token_expired","message":"Expired"}}"#);
        assert_eq!(auth_output_error(&expired).code,"OFFICE_AUTH_EXPIRED");
        assert!(!output_error(&expired,true).not_executed);
        assert_ne!(output_error(&expired,true).code,"OFFICE_AUTH_EXPIRED");
        let denied=CliOutput::fixture(4,"",r#"{"ok":false,"error":{"type":"policy","subtype":"access_denied","message":"Denied"}}"#);
        assert_eq!(auth_output_error(&denied).code,"OFFICE_AUTH_DENIED");
        let unknown=CliOutput::fixture(4,"",r#"{"ok":false,"error":{"type":"authentication","subtype":"unknown","message":"Device code expired, authorization denied"}}"#);
        assert_eq!(auth_output_error(&unknown).code,"OFFICE_UNAVAILABLE");
    }
    #[test]
    fn base_matrix_uses_stable_field_ids_and_rejects_misaligned_rows(){
        let data=json!({"timezone":"Asia/Shanghai","fields":["Name","Status"],"field_id_list":["fld_name","fld_status"],"field_type_list":["text","select"],"record_id_list":["rec_1"],"data":[["A",["Todo"]]],"has_more":false});
        let result=normalize_base_readback(&output(data.clone())).unwrap();
        assert_eq!(result.items[0].fields["fld_status"],json!(["Todo"]));
        assert!(!result.items[0].fields.contains_key("Status"));
        let mut malformed=data;malformed["data"]=json!([["A"]]);
        assert!(normalize_base_readback(&output(malformed)).is_err());
    }
    #[test]
    fn base_write_returns_pending_and_uses_a_private_json_file(){
        let mutation=OfficeMutation::BaseUpdate {client_item_id:"i".into(),base_token:"base_x".into(),table_id:"tbl_x".into(),
            record_id:"rec_1".into(),fields:BTreeMap::from([("fld_status".into(),json!(["Done"]))]),before:BTreeMap::from([("fld_status".into(),json!(["Todo"]))])};
        let cmd=mutation_command(&mutation,"operation-token",Some("work")).unwrap();
        assert!(cmd.args.contains(&format!("@{PAYLOAD_FILE}")));assert!(cmd.input_file.is_some());
        let receipt=normalize_mutation(&mutation,&output(json!({}))).unwrap();
        assert_eq!(receipt.status,ItemStatus::AcceptedPendingConfirmation);
    }
    #[test]
    fn base_batch_is_one_request_and_rejects_duplicate_or_cross_table_records(){
        let first=OfficeMutation::BaseUpdate{client_item_id:"i1".into(),base_token:"base".into(),table_id:"table".into(),
            record_id:"rec1".into(),fields:BTreeMap::from([("fld".into(),json!(["Done"]))]),before:BTreeMap::from([("fld".into(),json!(["Todo"]))])};
        let mut second=first.clone();
        if let OfficeMutation::BaseUpdate{record_id,..}=&mut second{*record_id="rec2".into()}
        let cmd=base_batch_command(&[first.clone(),second.clone()],None).unwrap();
        let body:Value=serde_json::from_slice(cmd.input_file.as_ref().unwrap()).unwrap();
        assert_eq!(body["update_records"].as_object().unwrap().len(),2);
        assert!(base_batch_command(&[first.clone(),first.clone()],None).is_err());
        if let OfficeMutation::BaseUpdate{table_id,..}=&mut second{*table_id="other".into()}
        assert!(base_batch_command(&[first,second],None).is_err());
    }
    #[test]
    fn calendar_creation_explicitly_disables_video_and_carries_idempotency(){
        let mutation=OfficeMutation::CalendarCreate {client_item_id:"i".into(),title:"Review".into(),
            start:"2026-10-28T14:00:00+08:00".into(),end:"2026-10-28T14:30:00+08:00".into(),timezone:"Asia/Shanghai".into()};
        let cmd=mutation_command(&mutation,"stable-key",Some("work")).unwrap();
        let body:Value=serde_json::from_str(&cmd.args[cmd.args.iter().position(|a|a=="--data").unwrap()+1]).unwrap();
        assert_eq!(body["vchat"]["vc_type"],"no_meeting");
        assert!(cmd.args.iter().any(|a|a.contains("stable-key")));
    }
    #[test]
    fn timeout_is_unknown_and_keeps_exact_original_resource_receipt(){
        let mutation=OfficeMutation::TaskCreate {client_item_id:"i".into(),title:"T".into(),assignee_id:"ou_x".into(),assignee_name:None,due:Some("2026-10-28".into()),tasklist_id:None};
        let mut partial=output(json!({"guid":"original-guid"}));partial.timed_out=true;
        let receipt=normalize_mutation(&mutation,&partial).unwrap();
        assert_eq!(receipt.status,ItemStatus::Unknown);assert_eq!(receipt.remote_id.as_deref(),Some("original-guid"));
        let error=output_error(&CliOutput::fixture(1,"",r#"{"ok":false,"error":{"type":"api","message":"server failed"}}"#),true);
        assert!(!error.not_executed);
    }
}
