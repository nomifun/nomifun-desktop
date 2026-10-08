//! Agnes Video 2.5 request and queue-admission contract.
//! The accepted-job lifecycle stays in `agnes`; only explicit negative
//! acknowledgements may retry submission here.
//! https://wiki.agnes-ai.com/zh-Hans/docs/agnes-video-25
//! https://wiki.agnes-ai.com/zh-Hans/docs/agnes-video-25-flash

use std::time::Duration;

use nomifun_api_types::{AgnesModelContract, agnes_model_contract};
use serde_json::{Value, json};

use crate::adapters::json_request_body;
use crate::auth::AuthMaterial;
use crate::error::{InvokeError, InvokeErrorKind, ModelFailureReason};
use crate::transport::{error_response_with_json, post_json};
use crate::types::VideoGenRequest;

use super::{VIDEO_ADAPTER_ID, VIDEO_SUBMIT_TIMEOUT, image_data_uri};

#[derive(Clone, Copy)]
struct QueueRetryPolicy {
    attempts: usize,
    initial_delay: Duration,
    max_wait: Duration,
    error_body_timeout: Duration,
}

const QUEUE_RETRY_POLICY: QueueRetryPolicy = QueueRetryPolicy {
    attempts: 3,
    initial_delay: Duration::from_secs(5),
    max_wait: Duration::from_secs(30),
    error_body_timeout: Duration::from_secs(10),
};

/// Agnes currently returns this root envelope when its video queue refuses a
/// submission. Require the complete known negative acknowledgement: an ID,
/// status, output, nested error or unknown field makes acceptance ambiguous.
/// HTTP status alone and words in `message` are never retry authority.
fn queue_rejected(status: Option<u16>, value: Option<&Value>) -> bool {
    let Some(root) = value.and_then(Value::as_object) else { return false; };
    status == Some(503)
        && root.len() == 3
        && root.get("code").and_then(Value::as_str) == Some("video_queue_full")
        && root.get("message").and_then(Value::as_str).is_some_and(|message| !message.trim().is_empty())
        && root.get("data") == Some(&Value::Null)
}

pub(super) async fn submit(
    http: &reqwest::Client,
    url: &str,
    auth: &AuthMaterial,
    body: &Value,
) -> Result<reqwest::Response, InvokeError> {
    submit_with_policy(http, url, auth, body, QUEUE_RETRY_POLICY).await
}

async fn submit_with_policy(
    http: &reqwest::Client,
    url: &str,
    auth: &AuthMaterial,
    body: &Value,
    policy: QueueRetryPolicy,
) -> Result<reqwest::Response, InvokeError> {
    let mut waited = Duration::ZERO;
    for attempt in 1..=policy.attempts {
        // Do not repeat transport failures: the remote side may have already
        // created a job even if no response reached this client.
        let response = post_json(http, url, VIDEO_SUBMIT_TIMEOUT, auth, body).await?;
        if response.status().is_success() {
            return Ok(response);
        }
        let (mut error, value) = error_response_with_json(
            response, policy.error_body_timeout, VIDEO_ADAPTER_ID,
        ).await;
        if !queue_rejected(error.http_status, value.as_ref()) {
            return Err(error);
        }
        if let Some(diagnostic) = error.diagnostic.as_mut() {
            diagnostic.reason = ModelFailureReason::ProviderOverloaded;
            diagnostic.provider_code = Some("video_queue_full".to_owned());
        }
        let backoff = policy.initial_delay.saturating_mul(1 << (attempt - 1));
        let delay = backoff.max(Duration::from_millis(error.retry_after_ms.unwrap_or(0)));
        if attempt == policy.attempts || delay > policy.max_wait.saturating_sub(waited) {
            // Preserve the actual HTTP status and redacted provider evidence.
            // Queue refusal is not a broken model configuration or a success.
            error.message.push_str(&format!(
                "; Agnes video queue is temporarily full after {attempt} submission attempts; no video_id was returned. Please retry later"
            ));
            return Err(error);
        }
        // Ordinary async cancellation also drops this wait; no detached retry
        // worker or second job ledger is introduced.
        tokio::time::sleep(delay).await;
        waited += delay;
    }
    unreachable!("bounded submission loop returns on its final attempt")
}

const VIDEO_IMAGE_BYTES: usize = 15 * 1024 * 1024;
const VIDEO_TOTAL_IMAGE_BYTES: usize = 50 * 1024 * 1024;
const RATIOS: &[&str] = &["21:9", "16:9", "4:3", "1:1", "3:4", "9:16"];
const FLASH_PIXEL_SIZES: &[(&str, &str, &str)] = &[
    ("1680x720", "720P", "21:9"),
    ("1280x704", "720P", "16:9"),
    ("960x720", "720P", "4:3"),
    ("720x720", "720P", "1:1"),
    ("720x960", "720P", "3:4"),
    ("720x1280", "720P", "9:16"),
];
// Exact official mappings; pixel sizes belong to the shared request vocabulary,
// while the provider receives a resolution tier and aspect ratio.
const PIXEL_SIZES: &[(&str, &str, &str)] = &[
    ("1470x630", "720P", "21:9"),
    ("1280x720", "720P", "16:9"),
    ("1112x834", "720P", "4:3"),
    ("960x960", "720P", "1:1"),
    ("834x1112", "720P", "3:4"),
    ("720x1280", "720P", "9:16"),
    ("2206x946", "1080P", "21:9"),
    ("1920x1080", "1080P", "16:9"),
    ("1664x1248", "1080P", "4:3"),
    ("1440x1440", "1080P", "1:1"),
    ("1248x1664", "1080P", "3:4"),
    ("1080x1920", "1080P", "9:16"),
    ("1024x1024", "1K", "1:1"),
    ("2940x1260", "2K", "21:9"),
    ("2560x1440", "2K", "16:9"),
    ("2224x1668", "2K", "4:3"),
    ("1920x1920", "2K", "1:1"),
    ("1668x2224", "2K", "3:4"),
    ("1440x2560", "2K", "9:16"),
];

fn invalid(message: impl Into<String>) -> InvokeError {
    InvokeError::new(InvokeErrorKind::InvalidParams, message)
}

pub(super) fn validate_video_model(model: &str) -> Result<bool, InvokeError> {
    match agnes_model_contract(model) {
        Some(AgnesModelContract::Video { flash }) => Ok(flash),
        _ => Err(invalid(format!(
            "agnes.video_jobs has no documented video contract for {model:?}"
        ))),
    }
}

fn string_field<'a>(body: &'a Value, key: &str, fallback: &'a str) -> Result<&'a str, InvokeError> {
    match body.get(key) {
        None => Ok(fallback),
        Some(Value::String(value)) if !value.trim().is_empty() => Ok(value.trim()),
        _ => Err(invalid(format!("agnes.video_jobs {key} must be a non-empty string"))),
    }
}

fn wire_size(value: &str, flash: bool) -> Result<(String, Option<&'static str>), InvokeError> {
    let normalized = value.trim().to_ascii_uppercase();
    // Keep the previous shared 720P pixel choices as input aliases so saved
    // creations still run. New Flash choices use its own official dimensions.
    let sizes = if flash { FLASH_PIXEL_SIZES } else { &[] };
    let (tier, ratio) = if let Some((_, tier, ratio)) = sizes
        .iter().chain(PIXEL_SIZES.iter())
        .find(|(pixels, _, _)| pixels.eq_ignore_ascii_case(value.trim()))
    {
        ((*tier).to_owned(), Some(*ratio))
    } else {
        (normalized, None)
    };
    if !["720P", "1080P", "1K", "2K"].contains(&tier.as_str()) || (flash && tier != "720P") {
        return Err(invalid(if flash {
            "Agnes Video 2.5 Flash supports only 720P; choose a 720P size or the standard 2.5 model"
        } else {
            "Agnes Video 2.5 size must be 720P, 1080P, 1K, 2K or an exact documented pixel size"
        }));
    }
    Ok((tier, ratio))
}

fn media_array<'a>(body: &'a Value, field: &str, limit: usize) -> Result<&'a [Value], InvokeError> {
    let values = match body.get(field) {
        None => &[][..],
        Some(Value::Array(values)) => values.as_slice(),
        _ => return Err(invalid(format!("Agnes video {field} must be an array"))),
    };
    if values.len() > limit {
        return Err(invalid(format!("Agnes video {field} supports at most {limit} inputs")));
    }
    for value in values {
        let url = if field == "videos" { value.get("url") } else { Some(value) };
        if !url.and_then(Value::as_str).is_some_and(|url| !url.trim().is_empty()) {
            return Err(invalid(format!("Agnes video {field} requires non-empty media URLs")));
        }
    }
    Ok(values)
}

fn native_params(params: &Value) -> Result<Value, InvokeError> {
    // Video SDK extra_body is merged into the root, unlike image extra_body.
    // Normalize each layer separately so a request's nested extras can still
    // override a configured root default, and typed invariants remain last.
    let mut root = json_request_body(params, &Value::Null, json!({}))?;
    let extra = root.as_object_mut().unwrap().remove("extra_body");
    match extra {
        Some(extra) => json_request_body(&root, &extra, json!({})),
        None => Ok(root),
    }
}

pub(super) fn build_video_body(
    model: &str,
    configured: &Value,
    request: &VideoGenRequest,
) -> Result<Value, InvokeError> {
    let flash = validate_video_model(model)?;
    let mut body = json_request_body(
        &native_params(configured)?,
        &native_params(&request.extra)?,
        json!({"model": model, "prompt": request.prompt}),
    )?;
    for key in [
        "width", "height", "fps", "frame_rate", "num_frames", "quality",
        "num_inference_steps", "image", "video_url", "video_path",
        "video_reference", "input_reference", "reference_url",
    ] {
        if body.get(key).is_some() {
            return Err(invalid(format!(
                "Agnes Video 2.5 does not support {key}; use size/aspect_ratio and first_frame/last_frame/images/audios/videos",
            )));
        }
    }
    let selected_size = request.size.as_deref().or(request.resolution.as_deref())
        .map(Ok)
        .unwrap_or_else(|| string_field(&body, "size", "720P"))?;
    let (tier, pixel_ratio) = wire_size(selected_size, flash)?;
    let ratio = match (tier.as_str(), pixel_ratio) {
        ("1K", _) => "1:1",
        (_, Some(ratio)) => ratio,
        _ => string_field(&body, "aspect_ratio", "16:9")?,
    };
    if !RATIOS.contains(&ratio) {
        return Err(invalid("Agnes Video 2.5 aspect_ratio must be 21:9, 16:9, 4:3, 1:1, 3:4 or 9:16"));
    }
    let ratio = ratio.to_owned();
    body["size"] = json!(tier);
    body["aspect_ratio"] = json!(ratio);
    let seconds = match request.seconds {
        Some(seconds) => seconds,
        None => match body.get("seconds") {
            None => 5,
            Some(Value::String(value)) => value.parse::<u32>()
                .map_err(|_| invalid("Agnes video seconds must be an integer from 4 to 12"))?,
            Some(value) => value.as_u64().and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| invalid("Agnes video seconds must be an integer from 4 to 12"))?,
        },
    };
    if !(4..=12).contains(&seconds) {
        return Err(invalid("Agnes Video 2.5 seconds must be from 4 to 12"));
    }
    body["seconds"] = json!(seconds.to_string());
    if body.get("n").is_some_and(|value| value.as_u64() != Some(1)) {
        return Err(invalid("Agnes Video 2.5 n supports only 1"));
    }
    body["n"] = json!(1);

    if !request.inputs.is_empty() {
        let max_images = if flash { 5 } else { 8 };
        let total_bytes = request.inputs.iter()
            .try_fold(0_usize, |total, input| total.checked_add(input.bytes.len()));
        if request.inputs.len() > max_images
            || total_bytes.is_none_or(|total| total >= VIDEO_TOTAL_IMAGE_BYTES)
        {
            return Err(invalid(format!("Agnes video supports at most {max_images} images totaling less than 50 MiB")));
        }
        for key in ["first_frame", "last_frame", "images", "audios", "videos"] {
            body.as_object_mut().unwrap().remove(key);
        }
        let keyframes = request.inputs.iter()
            .any(|input| matches!(input.role.as_str(), "first_frame" | "last_frame"));
        let mut images = Vec::new();
        for (index, input) in request.inputs.iter().enumerate() {
            if input.bytes.len() >= VIDEO_IMAGE_BYTES {
                return Err(invalid("Agnes video input images must be smaller than 15 MiB"));
            }
            let image = image_data_uri(input, index + 1, VIDEO_ADAPTER_ID)?;
            match (keyframes, input.role.as_str()) {
                (true, "first_frame" | "last_frame") => {
                    if body.get(&input.role).is_some() {
                        return Err(invalid("Agnes video accepts at most one first_frame and one last_frame"));
                    }
                    body[&input.role] = json!(image);
                }
                (false, "reference" | "image") => images.push(image),
                _ => return Err(invalid("Agnes video cannot mix keyframes with reference images or use non-image roles")),
            }
        }
        body["mode"] = json!(if keyframes { "keyframe" } else { "reference" });
        if !keyframes {
            body["images"] = json!(images);
        }
    }

    let images = media_array(&body, "images", if flash { 5 } else { 8 })?.len();
    let audios = media_array(&body, "audios", 3)?.len();
    let videos = media_array(&body, "videos", if flash { 0 } else { 1 })?.len();
    if images + audios + videos > 12 {
        return Err(invalid("Agnes video supports at most 12 reference files"));
    }
    let frames = ["first_frame", "last_frame"].iter().try_fold(0, |count, key| {
        if body.get(*key).is_none() {
            Ok(count)
        } else {
            string_field(&body, key, "").map(|_| count + 1)
        }
    })?;
    let mode = string_field(&body, "mode", "text")?.to_owned();
    let has_references = images + audios + videos > 0;
    let valid_mode = match mode.as_str() {
        "text" => frames == 0 && !has_references,
        "keyframe" => frames > 0 && !has_references,
        "reference" => frames == 0 && has_references,
        _ => false,
    };
    if !valid_mode {
        return Err(invalid("Agnes video mode must match its text, keyframe or reference media inputs"));
    }
    body["mode"] = json!(mode);
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthScheme;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const FAST_POLICY: QueueRetryPolicy = QueueRetryPolicy {
        attempts: 3,
        initial_delay: Duration::from_millis(1),
        max_wait: Duration::from_secs(2),
        error_body_timeout: Duration::from_millis(100),
    };

    fn client() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    fn auth() -> AuthMaterial {
        AuthMaterial { scheme: AuthScheme::Bearer, credentials: json!({"api_keys":["fixture-private-key"]}) }
    }

    fn queue_full() -> Value {
        json!({"code":"video_queue_full", "message":"video queue is full, please retry later", "data":null})
    }

    #[test]
    fn only_the_exact_negative_admission_acknowledgement_allows_a_retry() {
        assert!(queue_rejected(Some(503), Some(&queue_full())));
        for status in [200, 400, 403, 429, 500, 502, 504] {
            assert!(!queue_rejected(Some(status), Some(&queue_full())));
        }
        for body in [
            json!({"code":"video_queue_full", "message":"queue full"}),
            json!({"code":"provider_error", "message":"video_queue_full", "data":null}),
            json!({"error":{"code":"video_queue_full", "message":"queue full"}, "data":null}),
            json!({"code":"video_queue_full", "message":"queue full", "data":{"video_id":"accepted"}}),
            json!({"code":"video_queue_full", "message":"queue full", "data":null, "video_id":"accepted"}),
            json!({"code":"video_queue_full", "message":"queue full", "data":null, "status":"queued"}),
            json!({"code":"video_queue_full", "message":"queue full", "data":null, "output":["accepted"]}),
            json!({"code":"video_queue_full", "message":null, "data":null}),
        ] {
            assert!(!queue_rejected(Some(503), Some(&body)), "ambiguous body: {body}");
        }
        assert!(!queue_rejected(Some(503), None));
    }

    #[tokio::test]
    async fn queue_recovery_repeats_the_same_request_then_returns_one_accepted_job() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/videos"))
            .respond_with(ResponseTemplate::new(503).set_body_json(queue_full()))
            .up_to_n_times(1).with_priority(1).expect(1).mount(&server).await;
        Mock::given(method("POST")).and(path("/v1/videos"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"video_id":"accepted-once"})))
            .with_priority(2).expect(1).mount(&server).await;
        let body = json!({"model":"agnes-video-2.5-flash", "size":"720P", "mode":"text", "seconds":"4", "prompt":"waves"});
        let response = submit_with_policy(&client(), &format!("{}/v1/videos", server.uri()), &auth(), &body, FAST_POLICY).await.unwrap();
        assert_eq!(response.json::<Value>().await.unwrap()["video_id"], "accepted-once");
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2);
        for request in requests {
            assert_eq!(serde_json::from_slice::<Value>(&request.body).unwrap(), body);
            assert_eq!(request.headers["authorization"], "Bearer fixture-private-key");
        }
    }

    #[tokio::test]
    async fn a_persistently_full_queue_stops_at_the_bound_and_preserves_truthful_diagnostics() {
        let server = MockServer::start().await;
        let mut body = queue_full();
        body["message"] = json!(format!("{} fixture-private-key", "x".repeat(800)));
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503).set_body_json(body).insert_header("x-request-id", "queue-trace"))
            .expect(3).mount(&server).await;
        let error = submit_with_policy(&client(), &server.uri(), &auth(), &json!({}), FAST_POLICY).await.unwrap_err();
        assert_eq!(error.kind, InvokeErrorKind::ProviderError);
        assert_eq!(error.http_status, Some(503));
        assert!(error.message.contains("after 3 submission attempts"));
        assert!(!error.to_string().contains("fixture-private-key"));
        let diagnostic = error.diagnostic.unwrap();
        assert_eq!(diagnostic.reason, ModelFailureReason::ProviderOverloaded);
        assert_eq!(diagnostic.provider_code.as_deref(), Some("video_queue_full"));
        assert_eq!(diagnostic.request_id.as_deref(), Some("queue-trace"));
        assert_eq!(diagnostic.protocol.as_deref(), Some(VIDEO_ADAPTER_ID));
    }

    #[tokio::test]
    async fn ambiguous_failures_auth_and_parameter_errors_never_resubmit() {
        for (status, body) in [
            (503, json!({"message":"video_queue_full"}).to_string()),
            (503, json!({"code":"video_queue_full", "message":"busy", "data":null, "id":"accepted"}).to_string()),
            (503, "{\"code\":\"video_queue_full\",\"message\":\"busy\",\"data\":null".into()),
            (503, json!({"code":"video_queue_full", "message":"x".repeat(65536), "data":null}).to_string()),
            (500, queue_full().to_string()),
            (400, queue_full().to_string()),
            (403, queue_full().to_string()),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST")).respond_with(ResponseTemplate::new(status).set_body_string(body))
                .expect(1).mount(&server).await;
            let error = submit_with_policy(&client(), &server.uri(), &auth(), &json!({}), FAST_POLICY).await.unwrap_err();
            assert_eq!(error.http_status, Some(status));
            assert!(!error.message.contains("submission attempts"));
        }
    }

    #[tokio::test]
    async fn a_free_tier_rate_limit_after_queue_refusal_stops_retries_immediately() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503).set_body_json(queue_full()))
            .up_to_n_times(1).with_priority(1).expect(1).mount(&server).await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(429).set_body_json(json!({
                "error":{"code":"rate_limit_exceeded", "message":"Free account API limit reached"}
            })))
            .with_priority(2).expect(1).mount(&server).await;
        let error = submit_with_policy(&client(), &server.uri(), &auth(), &json!({}), FAST_POLICY).await.unwrap_err();
        assert_eq!(error.kind, InvokeErrorKind::RateLimited);
        assert_eq!(error.http_status, Some(429));
        assert!(error.message.contains("rate_limit_exceeded"));
        assert!(!error.message.contains("queue is temporarily full"));
    }

    #[tokio::test]
    async fn a_connection_lost_after_sending_the_request_never_resubmits_an_uncertain_job() {
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 4096];
            assert!(stream.read(&mut request).await.unwrap() > 0);
            drop(stream); // The job may exist remotely; no acknowledgement reaches us.
            assert!(tokio::time::timeout(Duration::from_millis(100), listener.accept()).await.is_err());
        });
        let error = submit_with_policy(&client(), &format!("http://{address}/v1/videos"), &auth(), &json!({}), FAST_POLICY).await.unwrap_err();
        assert_eq!(error.kind, InvokeErrorKind::Network);
        assert_eq!(error.http_status, None);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn retry_after_is_a_minimum_delay_and_an_excessive_wait_exits_without_retrying_early() {
        for retry_after in ["1", "3600"] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(503).set_body_json(queue_full()).insert_header("retry-after", retry_after))
                .up_to_n_times(1).with_priority(1).expect(1).mount(&server).await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"video_id":"accepted"})))
                .with_priority(2).expect(if retry_after == "1" { 1 } else { 0 }).mount(&server).await;
            let started = std::time::Instant::now();
            let result = submit_with_policy(&client(), &server.uri(), &auth(), &json!({}), FAST_POLICY).await;
            if retry_after == "1" {
                assert!(result.is_ok());
                assert!(started.elapsed() >= Duration::from_secs(1));
            } else {
                let error = result.unwrap_err();
                assert_eq!(error.retry_after_ms, Some(3_600_000));
                assert!(error.message.contains("after 1 submission attempts"));
            }
        }
    }

    #[tokio::test]
    async fn successful_http_responses_are_never_repeated_even_without_a_video_id() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_json(queue_full()))
            .expect(1).mount(&server).await;
        let response = submit_with_policy(&client(), &server.uri(), &auth(), &json!({}), FAST_POLICY).await.unwrap();
        assert!(response.status().is_success());
        // The adapter will reject this as malformed, never retry an ambiguous
        // success and possibly create a second video task.
    }

    #[tokio::test]
    async fn cancellation_during_backoff_does_not_start_a_detached_retry() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(503).set_body_json(queue_full()))
            .expect(1).mount(&server).await;
        let url = server.uri();
        let operation = tokio::spawn(async move {
            submit_with_policy(&client(), &url, &auth(), &json!({}), QueueRetryPolicy {
                initial_delay: Duration::from_millis(100), ..FAST_POLICY
            }).await
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            while server.received_requests().await.unwrap().is_empty() {
                tokio::task::yield_now().await;
            }
        }).await.unwrap();
        operation.abort();
        assert!(operation.await.unwrap_err().is_cancelled());
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[test]
    fn flash_sizes_use_the_flash_table_but_saved_standard_720p_aliases_still_work() {
        for (pixels, tier, ratio) in FLASH_PIXEL_SIZES {
            assert_eq!(wire_size(pixels, true).unwrap(), (tier.to_string(), Some(*ratio)));
            if *pixels != "720x1280" {
                assert!(wire_size(pixels, false).is_err());
            }
        }
        for (pixels, tier, ratio) in PIXEL_SIZES.iter().filter(|(_, tier, _)| *tier == "720P") {
            assert_eq!(wire_size(pixels, true).unwrap(), (tier.to_string(), Some(*ratio)));
        }
        assert!(wire_size("1920x1080", true).is_err());
    }
}
