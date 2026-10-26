//! Browser-facing authorization state. Secrets and device codes stay in memory.
use std::{sync::{Arc, LazyLock, Mutex}, time::{Duration, Instant}};
use chrono::{TimeDelta, Utc};
use regex::Regex;
use tokio_util::sync::CancellationToken;
use crate::{adapter, runtime::{CliObserver, CliStream}, service::{AuthEntry, OfficeHubService, lock, validate_profile}, *};

static URL_IN_OUTPUT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"https://[^\s\x1b"'<>]+"#).expect("fixed authorization URL expression"));

pub(crate) fn official_url(value: &str) -> bool {
    let Ok(url) = url::Url::parse(value) else { return false };
    url.scheme() == "https" && url.username().is_empty() && url.password().is_none()
        && url.port().is_none_or(|port| port == 443)
        && url.host_str().is_some_and(|host| ["feishu.cn", "larksuite.com", "larkoffice.com"].iter()
            .any(|domain| host == *domain || host.ends_with(&format!(".{domain}"))))
}

impl OfficeHubService {
    /// The host keeps the connection fence until the exact logout invocation
    /// and its verification finish, even if the HTTP observer goes away.
    pub async fn logout(self: &Arc<Self>, request: LogoutRequest) -> Result<(), HubError> {
        let service = self.clone();
        let (reply, receiver) = tokio::sync::oneshot::channel();
        if !(self.spawn)(Box::pin(async move {
            let result = service.logout_owned(request).await;
            let _ = reply.send(result);
        })) { return Err(HubError::unavailable("服务正在关闭，退出登录尚未开始")); }
        receiver.await.map_err(|_| HubError::unknown("退出结果尚未确认，请刷新连接状态"))?
    }

    async fn logout_owned(&self, request: LogoutRequest) -> Result<(), HubError> {
        let _admission = self.admission.lock().await;
        if self.is_executing() || self.is_authorizing()
            || lock(&self.prepare_view).as_ref().is_some_and(|view| view.phase == RuntimePhase::Preparing) {
            return Err(HubError::conflict("请等待当前操作或授权流程结束后再退出登录", None));
        }
        if self.shutdown.is_cancelled() { return Err(HubError::unavailable("服务正在关闭，退出登录尚未开始")); }
        validate_profile(&request.profile)?;
        let connection = self.connection().await?;
        if connection.profile.as_deref() != Some(request.profile.as_str()) {
            return Err(HubError::conflict("连接已变化，请刷新后再退出登录", None));
        }
        if let Some(id) = self.store.active_operation_id(&self.owner, &connection.id).await? {
            return Err(HubError::conflict("请等待已提交的办公操作结束后再退出登录", Some(id)));
        }
        let runtime = self.runtime.inspect(connection.executable.as_deref()).await;
        if runtime.phase != RuntimePhase::Ready {
            return Err(HubError::unavailable("连接组件不可用，请先修复组件后再退出登录"));
        }
        let executable = runtime.executable.ok_or_else(|| HubError::unavailable("连接组件路径不可用"))?;
        let users = self.runtime.run(&executable, adapter::auth_users_command(&request.profile)?, self.shutdown.child_token(), None).await?;
        let users = adapter::parse_auth_users(&users)?;
        if users.len() > 1 {
            return Err(HubError::conflict("此连接保存了多个账号，无法单独退出当前账号。请在“切换连接”中选择另一个配置", None));
        }
        let Some(actor) = users.first() else { return Ok(()); };
        if request.expected_actor.as_ref().is_some_and(|expected| expected.app_id != actor.app_id || expected.user_id != actor.user_id) {
            return Err(HubError::conflict("登录账号已变化，请刷新后确认要退出的账号", None));
        }
        let output = self.runtime.run(&executable, adapter::logout_command(&request.profile)?, self.shutdown.child_token(), None).await?;
        adapter::parse_logout(&output)?;
        let remaining = self.runtime.run(&executable, adapter::auth_users_command(&request.profile)?, self.shutdown.child_token(), None).await
            .and_then(|output| adapter::parse_auth_users(&output))
            .map_err(|_| HubError::unknown("退出请求已完成，但登录状态无法确认，请刷新连接状态"))?;
        if !remaining.is_empty() { return Err(HubError::unknown("登录状态尚未退出，请刷新后重试")); }
        Ok(())
    }

    pub fn authorization(&self, id: &str) -> Result<AuthorizationFlow, HubError> {
        lock(&self.flows).get_mut(id).map(|entry| {
            if matches!(entry.flow.phase,AuthPhase::WaitingUser)
                && entry.flow.expires_at.as_deref().and_then(|value|chrono::DateTime::parse_from_rfc3339(value).ok())
                    .is_some_and(|expiry|expiry.with_timezone(&Utc)<=Utc::now()){
                entry.cancel.cancel();entry.flow.phase=AuthPhase::Expired;
                entry.flow.verification_url=None;entry.flow.message=Some("This authorization flow expired; existing permissions and local input are retained".into());
            }
            entry.flow.clone()
        }).ok_or_else(|| {
            let mut error=HubError::invalid("Authorization flow not found or expired");
            error.code="OFFICE_NOT_FOUND".into();error
        })
    }
    pub async fn cancel_authorization(&self, id: &str) -> Result<AuthorizationFlow, HubError> {
        let _admission=self.admission.lock().await;
        let mut flows=lock(&self.flows);
        let entry=flows.get_mut(id).ok_or_else(|| HubError::invalid("Authorization flow not found"))?;
        if matches!(entry.flow.phase, AuthPhase::Starting|AuthPhase::WaitingUser|AuthPhase::WaitingAdmin) {
            entry.cancel.cancel();entry.flow.phase=if entry.worker_active{AuthPhase::Cancelling}else{AuthPhase::Cancelled};
            entry.flow.verification_url=None;
            entry.flow.message=Some("Authorization was cancelled; existing permissions and local input are retained".into());
        }
        Ok(entry.flow.clone())
    }

    pub async fn start_authorization(self: &Arc<Self>, mut request: AuthStartRequest) -> Result<AuthorizationFlow, HubError> {
        let _admission=self.admission.lock().await;
        if self.is_executing() { return Err(HubError::conflict("Wait for the original office invocation before changing authorization", None)); }
        if self.is_authorizing(){
            let mut error=HubError::conflict("Observe or cancel the original authorization flow before starting another",None);
            error.related_flow_id=lock(&self.flows).iter().find(|(_,entry)|entry.worker_active||matches!(entry.flow.phase,AuthPhase::Starting|AuthPhase::WaitingUser|AuthPhase::WaitingAdmin|AuthPhase::Cancelling)).map(|(id,_)|id.clone());
            return Err(error)
        }
        if self.shutdown.is_cancelled() { return Err(HubError::unavailable("The host is shutting down")); }
        let allowed=adapter::allowed_scopes();
        if request.scopes.len()>64 || request.scopes.iter().any(|scope| !allowed.contains(scope)) {
            return Err(HubError::invalid("Authorization accepts only the supported office operation scopes"));
        }
        if request.kind==AuthKind::Login&&request.scopes.is_empty() {
            return Err(HubError::invalid("Choose the exact permissions needed for this operation"));
        }
        let mut connection=self.connection().await?;
        let info=self.runtime.inspect(connection.executable.as_deref()).await;
        if info.phase!=RuntimePhase::Ready { return Err(HubError::unavailable("Prepare a compatible CLI before connecting Feishu")); }
        let executable=info.executable.ok_or_else(|| HubError::unavailable("CLI path unavailable"))?;
        let profile=request.profile.clone().or_else(|| {
            if request.kind==AuthKind::Login { connection.profile.clone() }
            else { Some(format!("nomifun-{}", connection.id.replace('-', ""))) }
        }).ok_or_else(|| HubError::invalid("Select a configured profile before signing in"))?;
        validate_profile(&profile)?; request.profile=Some(profile.clone());
        if request.kind!=AuthKind::Login {
            let output=self.runtime.run(&executable,adapter::profiles_command(),self.shutdown.child_token(),None).await?;
            let profiles=adapter::parse_profiles(&output)?;
            if profiles.iter().any(|existing|existing.name==profile) {
                return Err(HubError::conflict("This profile is already configured; select it instead of replacing its application",None));
            }
        }
        connection.enabled=true;connection.executable=Some(executable.clone());
        // Login targets only a profile explicitly chosen by this owner.
        if request.kind==AuthKind::Login { connection.profile=Some(profile.clone()); }
        self.store.save_connection(&self.owner,&connection).await?;
        let id=uuid::Uuid::now_v7().to_string();
        let cancel=self.shutdown.child_token();
        let flow=AuthorizationFlow { id:id.clone(),kind:request.kind.clone(),phase:AuthPhase::Starting,
            verification_url:None,expires_at:None,missing_scopes:Vec::new(),console_url:None,message:None };
        {
            let mut flows=lock(&self.flows);
            flows.retain(|_,entry|entry.created.elapsed()<Duration::from_secs(1800));
            if flows.values().any(|entry|matches!(entry.flow.phase,AuthPhase::Starting|AuthPhase::WaitingUser|AuthPhase::WaitingAdmin)) {
                return Err(HubError::conflict("Observe or cancel the original authorization flow before starting another",None));
            }
            flows.insert(id.clone(),AuthEntry {flow:flow.clone(),cancel:cancel.clone(),created:Instant::now(),worker_active:true});
        }
        let command=if request.kind==AuthKind::Login { adapter::login_command(&request.scopes,Some(&profile)) }
            else { adapter::configure_command(&request) };
        let kind=request.kind.clone();
        // No request carrying app_secret is retained by the flow or its task.
        drop(request);
        let command=match command {
            Ok(command)=>command,
            Err(error)=>{lock(&self.flows).remove(&id);return Err(error)}
        };
        let service=self.clone();let task_id=id.clone();
        if !(self.spawn)(Box::pin(async move {
            let outcome=if kind==AuthKind::Login {
                service.run_login(&task_id,&executable,&profile,command,cancel.clone()).await
            } else {
                service.run_configuration(&task_id,&executable,&profile,&connection,command,cancel.clone()).await
            };
            if let Err(error)=outcome { service.fail_authorization(&task_id,&error,cancel.is_cancelled()); }
            if let Some(entry)=lock(&service.flows).get_mut(&task_id){
                entry.worker_active=false;
                if entry.flow.phase==AuthPhase::Cancelling{entry.flow.phase=AuthPhase::Cancelled}
            }
        })) {
            lock(&self.flows).remove(&id);
            return Err(HubError::unavailable("Authorization did not start because the host is shutting down"));
        }
        Ok(flow)
    }

    async fn run_login(&self,id:&str,executable:&str,profile:&str,command:crate::runtime::CliCommand,cancel:CancellationToken)->Result<(),HubError>{
        // Reauthorization must work precisely when the old user token is
        // expired. Resolve the application from configuration, not a ready
        // user identity or successful auth-status invocation.
        let configured=self.runtime.run(executable,adapter::profiles_command(),cancel.clone(),None).await?;
        let app_id=adapter::parse_profiles(&configured)?.into_iter().find(|item|item.name==profile)
            .map(|item|item.app_id).ok_or_else(||HubError::unavailable("The selected application profile is not configured"))?;
        let output=self.runtime.run(executable,command,cancel.clone(),None).await?;
        let start=adapter::parse_login_start(&output)?;
        if !official_url(&start.verification_url) { return Err(HubError::invalid("CLI returned an untrusted authorization URL")); }
        {
            let mut flows=lock(&self.flows);
            if let Some(entry)=flows.get_mut(id) {
                if entry.cancel.is_cancelled(){return Ok(())}
                entry.flow.phase=AuthPhase::WaitingUser;entry.flow.verification_url=Some(start.verification_url);
                entry.flow.expires_at=Some((Utc::now()+TimeDelta::seconds(start.expires_in.min(900) as i64)).to_rfc3339());
            }
        }
        let mut poll=adapter::poll_login_command(&start.device_code,Some(profile))?;
        poll.timeout=Duration::from_secs(start.expires_in.clamp(30,900)+10);
        let polled=self.runtime.run(executable,poll,cancel.clone(),None).await?;
        let completed=adapter::parse_login_completion(&polled,&app_id)?;
        {
            let mut flows=lock(&self.flows);
            if let Some(entry)=flows.get_mut(id) {
                if entry.cancel.is_cancelled(){return Ok(())}
                entry.flow.phase=if completed.complete {AuthPhase::Complete}else{AuthPhase::Partial};
                entry.flow.missing_scopes=completed.missing_scopes;
                entry.flow.message=completed.message;entry.flow.verification_url=None;
            }
        }
        Ok(())
    }

    async fn run_configuration(&self,id:&str,executable:&str,profile:&str,original:&OfficeConnection,command:crate::runtime::CliCommand,cancel:CancellationToken)->Result<(),HubError>{
        let tail=Arc::new(Mutex::new(String::new()));
        let flows=self.flows.clone();let flow_id=id.to_owned();
        let observer:CliObserver=Arc::new(move |stream,text|{
            if !matches!(stream,CliStream::Stderr){return}
            let mut buffer=lock(&tail);buffer.push_str(text);
            if buffer.len()>32768 {
                *buffer=buffer.chars().rev().take(12000).collect::<String>().chars().rev().collect();
            }
            let candidate=URL_IN_OUTPUT.find_iter(&buffer).map(|m|m.as_str().trim_end_matches([')', '）', ',', '，']))
                .filter(|value|official_url(value)).last().map(str::to_owned);
            if let Some(url)=candidate && let Some(entry)=lock(&flows).get_mut(&flow_id) {
                if entry.cancel.is_cancelled(){return}
                entry.flow.verification_url=Some(url);entry.flow.phase=AuthPhase::WaitingUser;
            }
        });
        let output=self.runtime.run(executable,command,cancel.clone(),Some(observer)).await?;
        adapter::parse_configured(&output)?;
        let _admission=self.admission.lock().await;
        if cancel.is_cancelled(){return Ok(())}
        let mut connection=self.connection().await?;
        if connection.id!=original.id||connection.profile!=original.profile||connection.executable!=original.executable{
            return Err(HubError::conflict("Connection changed while application configuration was in progress",None))
        }
        if cancel.is_cancelled()||lock(&self.flows).get(id).is_none_or(|entry|matches!(entry.flow.phase,AuthPhase::Cancelled|AuthPhase::Cancelling|AuthPhase::Expired)){
            return Ok(())
        }
        connection.profile=Some(profile.into());connection.enabled=true;
        connection.updated_at=Utc::now().to_rfc3339();
        self.store.save_connection(&self.owner,&connection).await?;
        if let Some(entry)=lock(&self.flows).get_mut(id) {
            entry.flow.phase=AuthPhase::Complete;entry.flow.verification_url=None;
            entry.flow.message=Some("Application configured; choose user permissions separately".into());
        }
        Ok(())
    }

    fn fail_authorization(&self,id:&str,error:&HubError,cancelled:bool){
        if let Some(entry)=lock(&self.flows).get_mut(id) {
            if matches!(entry.flow.phase,AuthPhase::Cancelled|AuthPhase::Expired){return}
            entry.flow.phase=if cancelled{AuthPhase::Cancelled}else if error.console_url.as_ref().is_some_and(|url|official_url(url)){AuthPhase::WaitingAdmin}else if error.code=="OFFICE_AUTH_EXPIRED"{AuthPhase::Expired}else if error.code=="OFFICE_AUTH_DENIED"{AuthPhase::Denied}else{AuthPhase::Error};
            entry.flow.console_url=error.console_url.clone().filter(|url|official_url(url));
            entry.flow.missing_scopes=error.missing_scopes.clone();
            // Never return raw auth stdout/stderr; those may contain device credentials.
            entry.flow.message=Some(if cancelled {"Authorization cancelled"}else if entry.flow.phase==AuthPhase::WaitingAdmin{"The application needs administrator approval; local input is retained"}else if entry.flow.phase==AuthPhase::Denied{"Authorization was denied; existing permissions and local input are retained"}else if entry.flow.phase==AuthPhase::Expired{"Authorization expired; start a new authorization flow when ready"}else{"Authorization could not complete; inspect application configuration and the requested permissions"}.into());
            entry.flow.verification_url=None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authorization_urls_require_exact_official_https_hosts() {
        for url in ["https://accounts.feishu.cn/device?x=1","https://open.larksuite.com/oauth/authorize"] {assert!(official_url(url))}
        for url in ["http://open.feishu.cn/a","https://feishu.cn.evil.invalid/a","https://evilfeishu.cn/a","https://u:p@open.feishu.cn/a","https://open.feishu.cn:444/a","javascript:alert(1)"] {assert!(!official_url(url))}
    }
}
