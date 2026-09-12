use axum::Router;
use axum::body::Body;
use axum::extract::Path;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use axum::routing::get;

use nomifun_common::AppError;

use crate::AssetService;

// Logo URLs have no content hash or version; revalidate across app upgrades.
const CACHE_CONTROL_VALUE: &str = "public, no-cache";

/// Build the public `/api/assets/*` router.
pub fn asset_routes() -> Router {
    Router::new()
        .route("/api/assets/logos/{*asset_path}", get(get_logo_asset))
}

async fn get_logo_asset(
    Path(asset_path): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let asset = AssetService.get_logo(&asset_path)?;
    let not_modified = headers.get_all(header::IF_NONE_MATCH).iter()
        .any(|value| AssetService.etag_matches(Some(value), &asset.etag));
    let response = Response::builder()
        .header(header::CACHE_CONTROL, CACHE_CONTROL_VALUE)
        .header(header::ETAG, asset.etag);
    if not_modified {
        response.status(StatusCode::NOT_MODIFIED).body(Body::empty())
    } else {
        response.header(header::CONTENT_TYPE, asset.content_type).body(Body::from(asset.bytes))
    }.map_err(|error| AppError::Internal(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    #[tokio::test]
    async fn get_logo_asset_serves_embedded_logo() {
        let router = asset_routes();
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api/assets/logos/ai-major/claude.svg")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "image/svg+xml");
        assert_eq!(response.headers()[header::CACHE_CONTROL], CACHE_CONTROL_VALUE);
        assert!(response.headers().contains_key(header::ETAG));
        assert!(!response.into_body().collect().await.unwrap().to_bytes().is_empty());
    }

    #[tokio::test]
    async fn get_logo_asset_returns_not_modified_for_matching_etag() {
        let router = asset_routes();
        let first = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/assets/logos/ai-major/claude.svg")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let etag = first.headers()[header::ETAG].clone();

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api/assets/logos/ai-major/claude.svg")
                    .header(header::IF_NONE_MATCH, etag)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(response.headers()[header::CACHE_CONTROL], CACHE_CONTROL_VALUE);
        assert_eq!(response.into_body().collect().await.unwrap().to_bytes().len(), 0);
    }

    #[tokio::test]
    async fn stable_logo_urls_require_revalidation() {
        let response = asset_routes()
            .oneshot(Request::builder().uri("/api/assets/logos/brand/nomi.svg").body(Body::empty()).unwrap())
            .await.unwrap();
        assert_eq!(response.headers()[header::CACHE_CONTROL], "public, no-cache");
    }

    #[tokio::test]
    async fn conditional_get_accepts_weak_and_repeated_etags() {
        let router = asset_routes();
        let asset = crate::AssetService.get_logo("brand/nomi.svg").unwrap();
        for candidates in [
            vec![format!("W/{}", asset.etag.to_str().unwrap())],
            vec!["\"different\"".to_owned(), asset.etag.to_str().unwrap().to_owned()],
        ] {
            let mut request = Request::builder().uri("/api/assets/logos/brand/nomi.svg");
            for candidate in candidates {
                request = request.header(header::IF_NONE_MATCH, candidate);
            }
            let response = router.clone().oneshot(request.body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
            assert_eq!(response.headers()[header::ETAG], asset.etag);
            assert!(response.into_body().collect().await.unwrap().to_bytes().is_empty());
        }
    }

    #[tokio::test]
    async fn get_logo_asset_rejects_traversal() {
        let router = asset_routes();
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api/assets/logos/%2E%2E%2Fbrand%2Fnomi.svg")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn get_logo_asset_returns_not_found_for_missing_file() {
        let router = asset_routes();
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api/assets/logos/ai-major/missing.svg")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
