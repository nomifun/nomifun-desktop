//! End-to-end coverage for the clean-start MiniApp M1 HTTP surface.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

const LOCAL_TRUST: &str = "miniapp-m1-local-trust";
const LEGACY_MINIAPP_ID: &str =
    "0190f5fe-7c00-7000-8000-000000000451";

#[tokio::test]
async fn m1_routes_replace_the_legacy_product_chain() {
    let (router, services) = common::build_local_trust_app(LOCAL_TRUST).await;
    let owner_id = services.authoritative_user_id.to_string();
    let owner_jwt = services
        .jwt_service
        .sign(&owner_id, "admin")
        .expect("owner JWT");

    nomifun_db::sqlx::query(
        "INSERT INTO miniapps (
            miniapp_id, user_id, name, description, html, html_size,
            created_at, updated_at
         ) VALUES (?, ?, 'retired', '', '<p>retired</p>', 14, 1, 1)",
    )
    .bind(LEGACY_MINIAPP_ID)
    .bind(&owner_id)
    .execute(services.database.pool())
    .await
    .expect("legacy audit row");

    let response = request(&router, Method::GET, "/api/miniapps", None)
        .header("authorization", format!("Bearer {owner_jwt}"))
        .send()
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let library = response_json(response).await;
    assert_eq!(library["data"]["library_revision"], 0);
    assert_eq!(library["data"]["miniapps"], json!([]));

    let create_body = json!({
        "expected_library_revision": 0,
        "display_name": "M1 Notes",
        "description": "clean-start project",
        "kind": "ui_only"
    });
    let response = request(
        &router,
        Method::POST,
        "/api/miniapps/projects",
        Some(create_body.clone()),
    )
    .header("authorization", format!("Bearer {owner_jwt}"))
    .send()
    .await;
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "an owner JWT alone must not authorize host-local creation"
    );

    let response = request(
        &router,
        Method::POST,
        "/api/miniapps/projects",
        Some(create_body),
    )
    .header(nomifun_auth::LOCAL_TRUST_HEADER, LOCAL_TRUST)
    .send()
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let created = response_json(response).await;
    let miniapp_id = created["data"]["miniapp"]["miniapp_id"]
        .as_str()
        .expect("created miniapp_id")
        .to_owned();
    nomifun_common::MiniAppId::parse(miniapp_id.clone())
        .expect("canonical MiniApp UUIDv7");
    assert_eq!(created["data"]["miniapp"]["display_name"], "M1 Notes");
    assert_eq!(created["data"]["source_state"], "empty");
    assert_eq!(created["data"]["project_revision"], 1);
    assert_eq!(created["data"]["build_generation"], 0);
    assert_eq!(created["data"]["miniapp"]["surface_available"], false);

    let response = request(
        &router,
        Method::GET,
        &format!("/api/miniapps/{miniapp_id}/workshop"),
        None,
    )
    .header("authorization", format!("Bearer {owner_jwt}"))
    .send()
    .await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "owner-scoped reads do not require local trust"
    );
    assert_eq!(response_json(response).await["data"], created["data"]);

    let product_owner: String = nomifun_db::sqlx::query_scalar(
        "SELECT owner_user_id FROM miniapp_products WHERE miniapp_id = ?",
    )
    .bind(&miniapp_id)
    .fetch_one(services.database.pool())
    .await
    .expect("M1 product owner");
    assert_eq!(product_owner, owner_id);
    let legacy_name: String = nomifun_db::sqlx::query_scalar(
        "SELECT name FROM miniapps WHERE miniapp_id = ?",
    )
    .bind(LEGACY_MINIAPP_ID)
    .fetch_one(services.database.pool())
    .await
    .expect("legacy row remains frozen");
    assert_eq!(legacy_name, "retired");

    for (method, path) in [
        (Method::POST, "/api/miniapps".to_owned()),
        (Method::GET, format!("/api/miniapps/{miniapp_id}")),
        (
            Method::PUT,
            format!("/api/miniapps/{miniapp_id}"),
        ),
        (
            Method::DELETE,
            format!("/api/miniapps/{miniapp_id}"),
        ),
        (
            Method::GET,
            format!("/api/miniapps/{miniapp_id}/serve"),
        ),
        (
            Method::POST,
            format!("/api/miniapps/{miniapp_id}/workspace"),
        ),
        (
            Method::POST,
            format!("/api/miniapps/{miniapp_id}/publish"),
        ),
        (Method::POST, "/api/miniapps/validate".to_owned()),
        (Method::POST, "/api/miniapps/import".to_owned()),
    ] {
        let response = request(&router, method.clone(), &path, Some(json!({})))
            .header(nomifun_auth::LOCAL_TRUST_HEADER, LOCAL_TRUST)
            .send()
            .await;
        assert!(
            matches!(
                response.status(),
                StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
            ),
            "{method} {path} must not reach the retired MiniApp chain: {}",
            response.status()
        );
    }

    services
        .shutdown_browser_platform()
        .await
        .expect("background cleanup");
    services.database.close().await;
}

struct RequestBuilder<'a> {
    router: &'a axum::Router,
    builder: axum::http::request::Builder,
    body: Body,
}

impl RequestBuilder<'_> {
    fn header(
        mut self,
        name: &'static str,
        value: impl AsRef<str>,
    ) -> Self {
        self.builder = self.builder.header(name, value.as_ref());
        self
    }

    async fn send(self) -> axum::response::Response {
        self.router
            .clone()
            .oneshot(self.builder.body(self.body).expect("request"))
            .await
            .expect("route response")
    }
}

fn request<'a>(
    router: &'a axum::Router,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> RequestBuilder<'a> {
    let mut builder = Request::builder().method(method).uri(path);
    let body = match body {
        Some(body) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(serde_json::to_vec(&body).expect("request JSON"))
        }
        None => Body::empty(),
    };
    RequestBuilder {
        router,
        builder,
        body,
    }
}

async fn response_json(response: axum::response::Response) -> Value {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("response body")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("response JSON")
}
