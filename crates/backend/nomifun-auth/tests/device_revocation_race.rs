//! Credential revocation must survive every async lookup before admission or
//! renewal. Use explicit barriers rather than depending on scheduler timing.
use std::sync::Arc;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use nomifun_auth::{AuthPolicy, AuthRouterState, CookieConfig, DeviceStore, JwtService, QrTokenStore, auth_routes};
use nomifun_db::{DbError, IUserRepository, SqliteUserRepository, models::User};
use tokio::sync::Semaphore;
use tower::ServiceExt;

struct PausedUserLookup {
    inner: Arc<dyn IUserRepository>,
    entered: Arc<Semaphore>,
    release: Arc<Semaphore>,
}

#[async_trait::async_trait]
impl IUserRepository for PausedUserLookup {
    async fn has_users(&self)->Result<bool,DbError>{self.inner.has_users().await}
    async fn get_system_user(&self)->Result<Option<User>,DbError>{self.inner.get_system_user().await}
    async fn get_primary_webui_user(&self)->Result<Option<User>,DbError>{self.inner.get_primary_webui_user().await}
    async fn set_system_user_credentials(&self,n:&str,p:&str)->Result<(),DbError>{self.inner.set_system_user_credentials(n,p).await}
    async fn set_system_user_credentials_if_uninitialized(&self,n:&str,p:&str)->Result<bool,DbError>{self.inner.set_system_user_credentials_if_uninitialized(n,p).await}
    async fn set_system_user_password_if_uninitialized(&self,p:&str)->Result<bool,DbError>{self.inner.set_system_user_password_if_uninitialized(p).await}
    async fn create_user(&self,n:&str,p:&str)->Result<User,DbError>{self.inner.create_user(n,p).await}
    async fn find_by_username(&self,n:&str)->Result<Option<User>,DbError>{self.inner.find_by_username(n).await}
    async fn find_by_id(&self,id:&str)->Result<Option<User>,DbError>{
        let user=self.inner.find_by_id(id).await?;
        self.entered.add_permits(1);
        self.release.acquire().await.unwrap().forget();
        Ok(user)
    }
    async fn list_users(&self)->Result<Vec<User>,DbError>{self.inner.list_users().await}
    async fn count_users(&self)->Result<i64,DbError>{self.inner.count_users().await}
    async fn update_password(&self,id:&str,p:&str)->Result<(),DbError>{self.inner.update_password(id,p).await}
    async fn update_username(&self,id:&str,n:&str)->Result<(),DbError>{self.inner.update_username(id,n).await}
    async fn update_last_login(&self,id:&str)->Result<(),DbError>{self.inner.update_last_login(id).await}
    async fn update_jwt_secret(&self,id:&str,s:&str)->Result<(),DbError>{self.inner.update_jwt_secret(id,s).await}
}

#[tokio::test]
async fn revocation_or_repair_during_lookup_rejects_old_http_and_refresh_credentials() {
    for repair in [false,true] {
    for refresh in [false,true] {
        let database=nomifun_db::init_database_memory().await.unwrap();
        let real_repo=Arc::new(SqliteUserRepository::new(database.pool().clone()));
        let user=real_repo.get_system_user().await.unwrap().unwrap();
        let devices=Arc::new(DeviceStore::default());
        let a=devices.register(user.user_id.as_str(),"Phone A",&"a".repeat(64),None).await.unwrap();
        let b=devices.register(user.user_id.as_str(),"Phone B",&"b".repeat(64),None).await.unwrap();
        let jwt=Arc::new(JwtService::with_devices("test-race-secret".into(),devices.clone()));
        let token=jwt.sign_for_device(user.user_id.as_str(),&user.username,&a.device.device_id,Some(&a.credential_id)).unwrap();
        let b_token=jwt.sign_for_device(user.user_id.as_str(),&user.username,&b.device.device_id,Some(&b.credential_id)).unwrap();
        let entered=Arc::new(Semaphore::new(0));let release=Arc::new(Semaphore::new(0));
        let app=auth_routes(AuthRouterState{
            auth_policy:AuthPolicy::Required,
            jwt_service:jwt.clone(),user_repo:Arc::new(PausedUserLookup{inner:real_repo,entered:entered.clone(),release:release.clone()}),
            cookie_config:Arc::new(CookieConfig{secure:false,same_site:"Lax"}),qr_token_store:Arc::new(QrTokenStore::new()),
        });
        let request=if refresh{
            Request::post("/api/auth/refresh").header("content-type","application/json").body(Body::from(serde_json::json!({"token":token}).to_string())).unwrap()
        }else{Request::get("/api/auth/user").header("authorization",format!("Bearer {token}")).body(Body::empty()).unwrap()};
        let waiting=tokio::spawn(app.clone().oneshot(request));
        tokio::time::timeout(std::time::Duration::from_secs(2),entered.acquire()).await.unwrap().unwrap().forget();
        let replacement = if repair {
            Some(devices.register(user.user_id.as_str(),"Phone A",&"a".repeat(64),None).await.unwrap())
        } else {
            devices.revoke(&a.device.device_id,user.user_id.as_str()).await.unwrap();
            None
        };
        release.add_permits(1);
        let response=tokio::time::timeout(std::time::Duration::from_secs(2),waiting).await.unwrap().unwrap().unwrap();
        assert_eq!(response.status(),if refresh{StatusCode::UNAUTHORIZED}else{StatusCode::FORBIDDEN});
        let body:serde_json::Value=serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(body["code"],"DEVICE_REVOKED");
        assert!(body.get("token").is_none());
        assert!(jwt.verify(&b_token).is_ok());
        if let Some(replacement) = replacement {
            assert_eq!(replacement.device.device_id, a.device.device_id);
            let fresh = jwt.sign_for_device(user.user_id.as_str(), &user.username, &replacement.device.device_id, Some(&replacement.credential_id)).unwrap();
            assert!(jwt.verify(&fresh).is_ok());
        }
        let browser_failure=app.oneshot(Request::get("/api/auth/user").header("authorization","Bearer not-a-token").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(browser_failure.status(),StatusCode::FORBIDDEN);
        let body:serde_json::Value=serde_json::from_slice(&browser_failure.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(body["code"],"FORBIDDEN");
    }
    }
}

#[tokio::test]
async fn refresh_cannot_renew_a_token_revoked_by_secret_rotation_during_lookup() {
    let database=nomifun_db::init_database_memory().await.unwrap();
    let real_repo=Arc::new(SqliteUserRepository::new(database.pool().clone()));
    let user=real_repo.get_system_user().await.unwrap().unwrap();
    let jwt=Arc::new(JwtService::new("old-test-secret".into()));
    let token=jwt.sign(user.user_id.as_str(),&user.username).unwrap();
    let entered=Arc::new(Semaphore::new(0));let release=Arc::new(Semaphore::new(0));
    let app=auth_routes(AuthRouterState{
        auth_policy:AuthPolicy::Required,
        jwt_service:jwt.clone(),user_repo:Arc::new(PausedUserLookup{inner:real_repo,entered:entered.clone(),release:release.clone()}),
        cookie_config:Arc::new(CookieConfig{secure:false,same_site:"Lax"}),qr_token_store:Arc::new(QrTokenStore::new()),
    });
    let waiting=tokio::spawn(app.oneshot(Request::post("/api/auth/refresh").header("content-type","application/json").body(Body::from(serde_json::json!({"token":token}).to_string())).unwrap()));
    tokio::time::timeout(std::time::Duration::from_secs(2),entered.acquire()).await.unwrap().unwrap().forget();
    jwt.install_secret("replacement-test-secret".into()).unwrap();
    release.add_permits(1);
    let response=tokio::time::timeout(std::time::Duration::from_secs(2),waiting).await.unwrap().unwrap().unwrap();
    assert_eq!(response.status(),StatusCode::UNAUTHORIZED);
    let body:serde_json::Value=serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(body.get("token").is_none());
}
