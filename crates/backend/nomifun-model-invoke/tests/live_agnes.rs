//! Opt-in Agnes contract smoke. Supply a key on stdin, never in arguments or
//! source. Uses the real media adapters and exercises catalog, streaming chat,
//! generation/edit, video submission/polling and downloadable output.

use std::io::BufRead;
use std::time::{Duration, Instant};
use nomifun_model_invoke::*;
use serde_json::{Value, json};

const BASE: &str = "https://apihub.agnes-ai.com/v1";

fn live_auth() -> (AuthMaterial, nomifun_net::secret_redaction::SecretRedactor) {
    let mut credential = String::new();
    std::io::stdin().lock().read_line(&mut credential).expect("read key on stdin");
    assert!(!credential.trim().is_empty(), "Agnes key required on stdin");
    let redactor = nomifun_net::secret_redaction::SecretRedactor::new([credential.trim()]);
    let auth = AuthMaterial { scheme: AuthScheme::Bearer, credentials: json!({"api_keys":[credential.trim()]}) };
    credential.clear();
    (auth, redactor)
}

fn call(auth: &AuthMaterial, model: &str, protocol: &str, request: TaskRequest) -> ResolvedCall {
    ResolvedCall {
        provider_id: "018f0000-0000-7000-8000-000000000001".into(), config_revision: 1,
        platform: "agnes".into(), model: model.into(), task: request.task(), protocol: protocol.into(),
        connection: ResolvedConnection { role: "default".into(), base_url: BASE.into(), auth: auth.clone(), extra: json!({}) },
        model_params: json!({"endpoint": if protocol == "agnes.images" { "/images/generations" } else { "/videos" },
            "poll_endpoint":"https://apihub.agnes-ai.com/agnesapi?video_id={id}"}),
        request,
    }
}

async fn asset_bytes(http: &reqwest::Client, result: TaskOutcome) -> Result<Vec<u8>, String> {
    let TaskOutcome::Done(TaskResult::Assets(assets)) = result else { return Err("missing completed assets".into()) };
    let asset = assets.into_iter().next().ok_or("empty assets")?;
    match asset.data {
        ProducedData::Bytes(bytes) => Ok(bytes),
        ProducedData::Url(url) => {
            let response = http.get(url).timeout(Duration::from_secs(60)).send().await.map_err(|_| "output download failed")?;
            if !response.status().is_success() { return Err(format!("output download HTTP {}", response.status().as_u16())) }
            let bytes = response.bytes().await.map_err(|_| "output read failed")?;
            Ok(bytes.to_vec())
        }
    }
}

fn image_mime(bytes: &[u8]) -> Result<&'static str, String> {
    if bytes.len() < 1024 { return Err("image output is empty or truncated".into()); }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") { return Ok("image/png"); }
    if bytes.starts_with(b"\xff\xd8\xff") { return Ok("image/jpeg"); }
    if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP".as_slice()) { return Ok("image/webp"); }
    Err("output is not a PNG, JPEG or WebP image".into())
}

async fn video(http: &reqwest::Client, registry: &AdapterRegistry, call: ResolvedCall,
    redactor: &nomifun_net::secret_redaction::SecretRedactor) -> Result<(), String> {
    let adapter = registry.get("agnes.video_jobs", call.task).map_err(|e| e.to_string())?;
    let mut outcome = adapter.submit(http, &call).await.map_err(|e| e.to_string())?;
    if let TaskOutcome::Pending(job) = &outcome {
        eprintln!("AGNES_LIVE video {}: accepted video_id={}", call.model, redactor.redact(&job.remote_id));
    }
    let deadline = Instant::now() + Duration::from_secs(360);
    while let TaskOutcome::Pending(job) = outcome {
        if Instant::now() >= deadline {
            return Err(format!("video_id={} did not complete within 360s", job.remote_id));
        }
        tokio::time::sleep(Duration::from_secs(10)).await;
        outcome = adapter.poll(http, &call, &job).await.map_err(|e| e.to_string())?;
    }
    let bytes = asset_bytes(http, outcome).await?;
    if bytes.len() < 1024 || bytes.get(4..8) != Some(b"ftyp".as_slice()) { return Err("output is not an MP4".into()) }
    Ok(())
}

#[tokio::test]
#[ignore = "requires an Agnes key on stdin and makes real, potentially billable model calls"]
async fn live_agnes_catalog_chat_images_and_current_video_contracts() {
    let (auth, redactor) = live_auth();
    let http = nomifun_net::http_client();
    let registry = AdapterRegistry::new(default_adapters());
    let catalog = auth.apply(http.get(format!("{BASE}/models")).timeout(Duration::from_secs(30))).unwrap()
        .send().await.expect("catalog request");
    assert!(catalog.status().is_success(), "catalog HTTP {}", catalog.status());
    let catalog: Value = catalog.json().await.expect("catalog JSON");
    let ids: Vec<_> = catalog["data"].as_array().expect("catalog data").iter()
        .filter_map(|model| model["id"].as_str()).collect();
    let mut failures = Vec::new();
    for model in ids.iter().copied().filter(|model| !model.contains("image") && !model.contains("video")) {
        eprintln!("AGNES_LIVE chat {model}: starting");
        let response = auth.apply(http.post(format!("{BASE}/chat/completions"))
            .timeout(Duration::from_secs(120)).json(&json!({"model":model,"messages":[{"role":"user","content":"Reply only OK."}],
                "max_tokens":1024,"stream":true,"chat_template_kwargs":{"enable_thinking":false}}))).unwrap().send().await;
        let result = match response {
            Ok(response) if response.status().is_success() => {
                let text = response.text().await.unwrap_or_default();
                let has_output = text.lines().filter_map(|line| line.strip_prefix("data:"))
                    .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
                    .any(|value| value["choices"][0]["delta"]["content"].as_str().is_some_and(|value| !value.trim().is_empty()));
                if has_output { Ok(()) } else { Err("stream returned no visible output".to_owned()) }
            }
            Ok(response) => {
                let status = response.status().as_u16();
                let detail = redactor.redact(&response.text().await.unwrap_or_default());
                Err(format!("HTTP {status}: {}", detail.chars().take(2048).collect::<String>()))
            }
            Err(_) => Err("network/timeout".into()),
        };
        eprintln!("AGNES_LIVE chat {model}: {}", result.as_ref().map(|_| "ok").unwrap_or_else(|e| e));
        if let Err(error) = result { failures.push(format!("chat {model}: {error}")); }
    }
    let mut reference = None;
    for model in ["agnes-image-2.0-flash", "agnes-image-2.1-flash", "agnes-image-2.5-flash"] {
        assert!(ids.contains(&model), "image model missing from catalog");
        eprintln!("AGNES_LIVE generation {model}: starting");
        let request = TaskRequest::ImageGeneration(ImageGenRequest {
            prompt:"A blue glass cube on a white studio background, clean product photograph.".into(),
            count:1, size:Some("1024x1024".into()), quality:None, extra:json!({}) });
        let call = call(&auth, model, "agnes.images", request);
        let adapter = registry.get("agnes.images", call.task).unwrap();
        let result = match adapter.submit(&http, &call).await {
            Ok(outcome) => asset_bytes(&http, outcome).await.and_then(|bytes| {
                let mime = image_mime(&bytes)?;
                Ok((bytes, mime))
            }),
            Err(error) => Err(redactor.redact(&error.to_string())),
        };
        match result {
            Ok((bytes, mime)) => {
                eprintln!("AGNES_LIVE generation {model}: ok bytes={}", bytes.len());
                let input = InputAsset { id:None,role:"reference".into(),bytes:bytes.clone(),mime:mime.into() };
                reference = Some(input.clone());
                let mut edit = call.clone();
                edit.request = TaskRequest::ImageEdit(ImageEditRequest {
                    prompt:"Make the cube orange, preserve the composition and white background.".into(),
                    count:1,size:Some("1024x1024".into()),quality:None,inputs:vec![input],extra:json!({}) });
                edit.task = edit.request.task();
                eprintln!("AGNES_LIVE edit {model}: starting");
                let result = match adapter.submit(&http, &edit).await {
                    Ok(outcome) => asset_bytes(&http, outcome).await.and_then(|bytes| image_mime(&bytes).map(|_| bytes.len())),
                    Err(error) => Err(redactor.redact(&error.to_string())),
                };
                eprintln!("AGNES_LIVE edit {model}: {}", result.as_ref().map(|_| "ok").unwrap_or_else(|e| e));
                if let Err(error) = result { failures.push(format!("edit {model}: {error}")); }
            }
            Err(error) => {
                eprintln!("AGNES_LIVE generation {model}: {error}");
                failures.push(format!("image {model}: {error}"));
            }
        }
    }
    for model in ["agnes-video-2.5", "agnes-video-2.5-flash"] {
        assert!(ids.contains(&model), "video model missing from catalog");
        let mut modes = vec![("text", vec![])];
        if model.ends_with("flash") {
            if let Some(input) = &reference {
                modes.push(("reference", vec![input.clone()]));
                modes.push(("keyframe", vec![InputAsset{role:"first_frame".into(),..input.clone()},
                    InputAsset{role:"last_frame".into(),..input.clone()}]));
            }
        }
        for (mode, inputs) in modes {
            eprintln!("AGNES_LIVE video {model} {mode}: starting");
            let request=TaskRequest::VideoGeneration(VideoGenRequest {
                prompt:"A blue glass cube slowly rotates on a white studio background, fixed camera.".into(),
                seconds:Some(4),size:Some("1280x720".into()),resolution:None,inputs,extra:json!({}) });
            let result=video(&http,&registry,call(&auth,model,"agnes.video_jobs",request),&redactor).await
                .map_err(|error| redactor.redact(&error));
            eprintln!("AGNES_LIVE video {model} {mode}: {}",result.as_ref().map(|_| "ok").unwrap_or_else(|e| e));
            if let Err(error)=result { failures.push(format!("video {model} {mode}: {}",redactor.redact(&error))); }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Isolate a Flash-only recheck so it does not invoke paid standard models or
/// regenerate images. Success requires a completed, downloadable MP4, not just
/// a healthy catalog entry or an accepted job.
#[tokio::test]
#[ignore = "requires an Agnes key on stdin and creates one real Flash video task"]
async fn live_agnes_flash_video() {
    let (auth, redactor) = live_auth();
    let request = TaskRequest::VideoGeneration(VideoGenRequest {
        prompt: "A blue glass cube slowly rotates on a white studio background, fixed camera.".into(),
        seconds: Some(4), size: Some("1280x704".into()), resolution: None,
        inputs: vec![], extra: json!({}),
    });
    let http = nomifun_net::http_client();
    let registry = AdapterRegistry::new(default_adapters());
    eprintln!("AGNES_LIVE Flash video: starting");
    let result = video(&http, &registry, call(&auth, "agnes-video-2.5-flash", "agnes.video_jobs", request), &redactor)
        .await.map_err(|error| redactor.redact(&error));
    eprintln!("AGNES_LIVE Flash video: {}", result.as_ref().map(|_| "ok: completed MP4").unwrap_or_else(|error| error));
    assert!(result.is_ok(), "{}", result.unwrap_err());
}

/// Recheck only the restored v2.0 contract. One submit, no queue retries;
/// accepted jobs are polled to distinguish connectivity from usable output.
#[tokio::test]
#[ignore = "requires an Agnes key on stdin and creates one real v2.0 video task"]
async fn live_agnes_v20_video() {
    let (auth, redactor) = live_auth();
    let request = TaskRequest::VideoGeneration(VideoGenRequest {
        prompt: "A blue glass cube slowly rotates on a white studio background, fixed camera.".into(),
        // Use precisely the pre-eaf6a0b defaults: 1152x768, 24 fps, 121 frames.
        seconds: None, size: None, resolution: None, inputs: vec![], extra: json!({}),
    });
    let http = nomifun_net::http_client();
    let registry = AdapterRegistry::new(default_adapters());
    eprintln!("AGNES_LIVE v2.0 video: starting native /v1/videos contract");
    let result = video(&http, &registry, call(&auth, "agnes-video-v2.0", "agnes.video_jobs", request), &redactor)
        .await.map_err(|error| redactor.redact(&error));
    eprintln!("AGNES_LIVE v2.0 video: {}", result.as_ref().map(|_| "ok: completed MP4").unwrap_or_else(|error| error));
    assert!(result.is_ok(), "{}", result.unwrap_err());
}
