//! Structural media-response checks. Network origin policy remains in the
//! invocation/materialization boundary; these helpers never fetch artifacts.
use serde_json::Value;

use crate::error::{InvokeError, InvokeErrorKind};

pub(super) fn media_url(raw: &str, context: &str) -> Result<String, InvokeError> {
    let raw = raw.trim();
    let invalid = || InvokeError::parse(format!("{context} requires a non-empty HTTP(S) artifact URL without userinfo or fragment"));
    if raw.is_empty() || raw.chars().any(char::is_control) { return Err(invalid()); }
    let parsed = reqwest::Url::parse(raw).map_err(|_| invalid())?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none()
        || !parsed.username().is_empty() || parsed.password().is_some() || parsed.fragment().is_some() {
        return Err(invalid());
    }
    // Keep signed URL spelling intact; parsing is validation, not rewriting.
    Ok(raw.to_owned())
}

pub(super) fn meaningful_error(error: &Value) -> bool {
    match error {
        Value::String(message) => !message.trim().is_empty(),
        Value::Object(fields) => {
            if fields.get("type").and_then(Value::as_str) == Some("warning") { return false; }
            ["message", "type", "code"].iter().any(|field| fields.get(*field).is_some_and(|value| {
                value.as_str().is_some_and(|value| !value.trim().is_empty() && value.trim() != "0")
                    || value.as_i64().is_some_and(|value| value != 0)
            }))
        }
        _ => false,
    }
}

pub(super) fn reject_provider_error(value: &Value, context: &str) -> Result<(), InvokeError> {
    if value.get("status").and_then(Value::as_str).is_some_and(|status| matches!(status.trim().to_ascii_lowercase().as_str(),
        "failed"|"error"|"cancelled"|"canceled"|"expired"|"failure"|"fail")) {
        return Err(InvokeError::new(InvokeErrorKind::JobFailed, format!("{context} reported a failed or cancelled task")));
    }
    if value.get("error").is_some_and(meaningful_error)
        || value.get("type").and_then(Value::as_str) == Some("error")
        || value.get("success") == Some(&Value::Bool(false)) {
        // Do not include arbitrary upstream JSON, credential echoes, or URLs.
        return Err(InvokeError::new(InvokeErrorKind::ProviderError, format!("{context} returned a provider error")));
    }
    Ok(())
}

/// Submission acknowledgements may legally contain only a job ID. If the
/// provider supplies a status, it must belong to that dialect's known set.
pub(super) fn validate_submit_status(
    value: Option<&Value>, pending: &[&str], completed: &[&str], failed: &[&str], context: &str,
) -> Result<(), InvokeError> {
    let Some(value) = value else { return Ok(()); };
    let status = value.as_str().map(str::trim).filter(|status| !status.is_empty())
        .ok_or_else(|| InvokeError::parse(format!("{context} returned an invalid task status")))?.to_ascii_lowercase();
    if failed.contains(&status.as_str()) {
        return Err(InvokeError::new(InvokeErrorKind::JobFailed, format!("{context} reported a failed or cancelled task")));
    }
    if pending.contains(&status.as_str()) || completed.contains(&status.as_str()) { return Ok(()); }
    Err(InvokeError::parse(format!("{context} returned an unrecognized task status")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use crate::types::{ImageGenRequest,TaskRequest,TaskOutcome,VideoGenRequest};
    use crate::adapter::ProtocolAdapter;
    use wiremock::{Mock,MockServer,ResponseTemplate};
    use wiremock::matchers::{method,path,header};

    fn image_call(server: &MockServer, protocol: &str) -> crate::call::ResolvedCall {
        let model = match protocol {"agnes.images"=>"agnes-image-2.1-flash","stepfun.images"=>"step-2x-large",_=>"image-model"};
        super::super::test_support::call_with_endpoint(&server.uri(),model,protocol,"/media",TaskRequest::ImageGeneration(ImageGenRequest {
            prompt:"fixture".into(),count:1,size:None,quality:None,extra:json!({}),
        }))
    }

    fn image_response(protocol: &str, url: &str) -> Value {
        if protocol == "siliconflow.images" {json!({"images":[{"url":url}]})} else {json!({"data":[{"url":url}]})}
    }

    fn submission(protocol: &str) -> (Value, &'static str, &'static str, &'static str) {
        match protocol {
            "dashscope.images" => (json!({"output":{"task_id":"job"}}),"/output/task_status","PENDING","FAILED"),
            "siliconflow.video_jobs" => (json!({"requestId":"job"}),"/status","InQueue","Failed"),
            "xai.video_jobs" => (json!({"request_id":"job"}),"/status","pending","failed"),
            "agnes.video_jobs" => (json!({"video_id":"job"}),"/status","queued","failed"),
            "zhipu.video_jobs" => (json!({"id":"job"}),"/task_status","PROCESSING","FAIL"),
            _ => (json!({"id":"job"}),"/status","queued","failed"),
        }
    }

    fn job_call(server: &MockServer, protocol: &str) -> crate::call::ResolvedCall {
        if protocol == "dashscope.images" {return image_call(server,protocol);}
        let model = if protocol == "agnes.video_jobs" {"agnes-video-2.5-flash"} else {"video-model"};
        super::super::test_support::call_with_endpoint(&server.uri(),model,protocol,"/media",TaskRequest::VideoGeneration(VideoGenRequest {
            prompt:"fixture".into(),seconds:None,size:None,resolution:None,inputs:vec![],extra:json!({}),
        }))
    }

    fn with_status(mut value: Value, pointer: &str, status: Value) -> Value {
        let field = pointer.rsplit('/').next().unwrap();
        if pointer.starts_with("/output/") {value["output"][field] = status;} else {value[field] = status;}
        value
    }

    #[test]
    fn artifact_urls_require_http_host_and_no_credentials_or_fragment() {
        for value in ["", " ", "/relative", "not a URL", "file:///tmp/a", "data:image/png;base64,aGk=",
            "https://user:secret@example.test/a", "https://example.test/a#fragment", "https://exam\nple.test/a"] {
            let error = media_url(value, "media").unwrap_err();
            assert_eq!(error.kind, InvokeErrorKind::ParseError);
            assert!(!error.to_string().contains("secret"));
        }
        assert_eq!(media_url(" https://cdn.test/a?signature=a%2Fb ", "media").unwrap(), "https://cdn.test/a?signature=a%2Fb");
    }

    #[test]
    fn warnings_and_empty_error_placeholders_do_not_become_provider_failures() {
        for value in [json!({"error":null}),json!({"error":{}}),json!({"error":""}),json!({"warning":"notice"}),
            json!({"error":{"type":"warning","message":"notice"}})] {
            reject_provider_error(&value,"media").unwrap();
        }
        for value in [json!({"error":{"message":"secret"}}),json!({"error":"secret"}),json!({"success":false})] {
            let error = reject_provider_error(&value,"media").unwrap_err();
            assert_eq!(error.kind,InvokeErrorKind::ProviderError);
            assert!(!error.to_string().contains("secret"));
        }
    }

    #[tokio::test]
    async fn image_submit_rejects_invalid_urls_and_business_errors_without_fetching_outputs() {
        for protocol in ["openai.images","ark.images","agnes.images","stepfun.images","siliconflow.images","xai.images_json"] {
            for url in ["", " ", "not-a-url", "file:///tmp/image", "https://user:sk-test@cdn.test/a", "https://cdn.test/a#fragment"] {
                let server=MockServer::start().await;
                Mock::given(method("POST")).and(path("/media")).and(header("authorization","Bearer sk-test"))
                    .respond_with(ResponseTemplate::new(200).set_body_json(image_response(protocol,url))).expect(1).mount(&server).await;
                let adapter=super::super::default_adapters().into_iter().find(|adapter| adapter.id()==protocol).unwrap();
                let error=adapter.submit(&reqwest::Client::new(),&image_call(&server,protocol)).await.unwrap_err();
                assert!(!error.to_string().contains("sk-test"),"{protocol}: {error}");
                assert_eq!(server.received_requests().await.unwrap().len(),1,"{protocol} must submit once without fetching a URL");
            }
            for error in [json!({"message":"sk-test"}),json!({}),Value::Null] {
                let server=MockServer::start().await;
                let mut body=image_response(protocol,"https://cdn.test/image.png");body["error"]=error.clone();body["warning"]=json!("notice");
                Mock::given(method("POST")).and(path("/media")).respond_with(ResponseTemplate::new(200).set_body_json(body)).expect(1).mount(&server).await;
                let adapter=super::super::default_adapters().into_iter().find(|adapter| adapter.id()==protocol).unwrap();
                let result=adapter.submit(&reqwest::Client::new(),&image_call(&server,protocol)).await;
                if meaningful_error(&error) {assert!(result.is_err(),"{protocol}");assert!(!result.unwrap_err().to_string().contains("sk-test"));}
                else {assert!(matches!(result.unwrap(),TaskOutcome::Done(_)),"{protocol}");}
            }
        }
    }

    #[tokio::test]
    async fn image_submit_rejects_empty_and_partial_assets_instead_of_reporting_done() {
        for protocol in ["openai.images","ark.images","agnes.images","stepfun.images","siliconflow.images","xai.images_json"] {
            for body in [json!({}),json!({"data":[],"images":[]}),json!({"data":[{"b64_json":""}],"images":[{}]})] {
                let server=MockServer::start().await;
                Mock::given(method("POST")).and(path("/media")).respond_with(ResponseTemplate::new(200).set_body_json(body)).expect(1).mount(&server).await;
                let adapter=super::super::default_adapters().into_iter().find(|adapter| adapter.id()==protocol).unwrap();
                assert!(adapter.submit(&reqwest::Client::new(),&image_call(&server,protocol)).await.is_err(),"{protocol}");
                assert_eq!(server.received_requests().await.unwrap().len(),1);
            }
        }
        for (protocol,body) in [
            ("gemini.generate_content",json!({"candidates":[{"content":{"parts":[{"inlineData":{"data":"","mimeType":"image/png"}}]}}]})),
            ("gemini.interactions_images",json!({"status":"completed","steps":[{"type":"model_output","content":[{"type":"image","data":"","mime_type":"image/png"}]}]})),
        ] {
            let server=MockServer::start().await;
            Mock::given(method("POST")).and(path("/media")).respond_with(ResponseTemplate::new(200).set_body_json(body)).expect(1).mount(&server).await;
            let adapter=super::super::default_adapters().into_iter().find(|adapter| adapter.id()==protocol).unwrap();
            let mut call=image_call(&server,protocol);
            call.connection.auth.scheme=crate::AuthScheme::HeaderKey("x-goog-api-key".into());
            assert!(adapter.submit(&reqwest::Client::new(),&call).await.is_err(),"{protocol}");
            assert_eq!(server.received_requests().await.unwrap().len(),1);
        }
    }

    #[tokio::test]
    async fn async_media_submit_rejects_failed_unknown_and_error_envelopes_but_preserves_real_acknowledgements() {
        for protocol in ["openai.videos","ark.video_jobs","dashscope.images","siliconflow.video_jobs","xai.video_jobs","agnes.video_jobs","zhipu.video_jobs"] {
            let (ack,field,pending,failed)=submission(protocol);
            for status in [json!(failed),json!("unrecognized"),json!(""),Value::Null] {
                let server=MockServer::start().await;
                let value=with_status(ack.clone(),field,status);
                Mock::given(method("POST")).and(path("/media")).respond_with(ResponseTemplate::new(200).set_body_json(value)).expect(1).mount(&server).await;
                let adapter=super::super::default_adapters().into_iter().find(|adapter| adapter.id()==protocol).unwrap();
                assert!(adapter.submit(&reqwest::Client::new(),&job_call(&server,protocol)).await.is_err(),"{protocol}");
                assert_eq!(server.received_requests().await.unwrap().len(),1);
            }
            for value in [ack.clone(),with_status(ack.clone(),field,json!(pending))] {
                let server=MockServer::start().await;
                Mock::given(method("POST")).and(path("/media")).respond_with(ResponseTemplate::new(200).set_body_json(value)).expect(1).mount(&server).await;
                let adapter=super::super::default_adapters().into_iter().find(|adapter| adapter.id()==protocol).unwrap();
                assert!(matches!(adapter.submit(&reqwest::Client::new(),&job_call(&server,protocol)).await.unwrap(),TaskOutcome::Pending(_)),"{protocol}");
                assert_eq!(server.received_requests().await.unwrap().len(),1,"accepted jobs must not be polled automatically");
            }
            let server=MockServer::start().await;
            let mut failed_body=ack;failed_body["error"]=json!({"message":"sk-test"});
            Mock::given(method("POST")).and(path("/media")).respond_with(ResponseTemplate::new(200).set_body_json(failed_body)).expect(1).mount(&server).await;
            let adapter=super::super::default_adapters().into_iter().find(|adapter| adapter.id()==protocol).unwrap();
            let error=adapter.submit(&reqwest::Client::new(),&job_call(&server,protocol)).await.unwrap_err();
            assert!(!error.to_string().contains("sk-test"),"{protocol}: {error}");
        }
    }

    #[tokio::test]
    async fn async_media_poll_rejects_missing_and_unknown_status_instead_of_waiting_forever() {
        for protocol in ["openai.videos","ark.video_jobs","dashscope.images","siliconflow.video_jobs","xai.video_jobs","agnes.video_jobs","zhipu.video_jobs"] {
            for body in [json!({}),with_status(json!({"output":{}}),submission(protocol).1,json!("unrecognized"))] {
                let server=MockServer::start().await;
                let method_name=if protocol=="siliconflow.video_jobs" {"POST"} else {"GET"};
                Mock::given(method(method_name)).respond_with(ResponseTemplate::new(200).set_body_json(body)).expect(1).mount(&server).await;
                let adapter=super::super::default_adapters().into_iter().find(|adapter| adapter.id()==protocol).unwrap();
                let mut call=job_call(&server,protocol);
                let descriptor=crate::protocol_task_descriptor(protocol,call.task).unwrap();
                call.model_params["poll_endpoint"]=json!(descriptor.endpoints.iter().find(|endpoint| endpoint.field=="poll_endpoint").unwrap().default_value);
                if protocol=="agnes.video_jobs" {call.model_params["poll_endpoint"]=json!(format!("{}/agnesapi?video_id={{id}}",server.uri()));}
                let job=crate::types::JobHandle {adapter_id:protocol.into(),config_revision:1,remote_id:"job".into(),poll_state:json!({})};
                assert!(adapter.poll(&reqwest::Client::new(),&call,&job).await.is_err(),"{protocol}");
                assert_eq!(server.received_requests().await.unwrap().len(),1,"{protocol} must reach the poll endpoint exactly once");
            }
        }
    }

    #[tokio::test]
    async fn completed_media_poll_rejects_invalid_artifact_urls() {
        for protocol in ["ark.video_jobs","dashscope.images","siliconflow.video_jobs","xai.video_jobs","agnes.video_jobs","zhipu.video_jobs"] {
            for url in ["", "not-a-url", "file:///tmp/video", "https://user:sk-test@cdn.test/video"] {
                let server=MockServer::start().await;
                let body=match protocol {
                    "ark.video_jobs"=>json!({"status":"succeeded","content":{"video_url":url}}),
                    "dashscope.images"=>json!({"output":{"task_status":"SUCCEEDED","results":[{"url":"https://cdn.test/valid.png"},{"url":url}]}}),
                    "siliconflow.video_jobs"=>json!({"status":"Succeed","results":{"videos":[{"url":url}]}}),
                    "xai.video_jobs"=>json!({"status":"done","video":{"url":url}}),
                    "agnes.video_jobs"=>json!({"status":"completed","url":url}),
                    _=>json!({"task_status":"SUCCESS","video_result":[{"url":url}]}),
                };
                let method_name=if protocol=="siliconflow.video_jobs" {"POST"} else {"GET"};
                Mock::given(method(method_name)).respond_with(ResponseTemplate::new(200).set_body_json(body)).expect(1).mount(&server).await;
                let adapter=super::super::default_adapters().into_iter().find(|adapter| adapter.id()==protocol).unwrap();
                let mut call=job_call(&server,protocol);
                let descriptor=crate::protocol_task_descriptor(protocol,call.task).unwrap();
                call.model_params["poll_endpoint"]=json!(descriptor.endpoints.iter().find(|endpoint| endpoint.field=="poll_endpoint").unwrap().default_value);
                if protocol=="agnes.video_jobs" {call.model_params["poll_endpoint"]=json!(format!("{}/agnesapi?video_id={{id}}",server.uri()));}
                let job=crate::types::JobHandle {adapter_id:protocol.into(),config_revision:1,remote_id:"job".into(),poll_state:json!({})};
                let error=adapter.poll(&reqwest::Client::new(),&call,&job).await.unwrap_err();
                assert!(!error.to_string().contains("sk-test"),"{protocol}: {error}");
                assert_eq!(server.received_requests().await.unwrap().len(),1);
            }
        }
    }

    #[tokio::test]
    async fn completed_openai_video_rejects_an_empty_content_body() {
        let server=MockServer::start().await;
        Mock::given(method("GET")).and(path("/videos/job"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status":"completed"}))).expect(1).mount(&server).await;
        Mock::given(method("GET")).and(path("/videos/job/content"))
            .respond_with(ResponseTemplate::new(200).insert_header("content-type","video/mp4").set_body_bytes(Vec::new())).expect(1).mount(&server).await;
        let mut call=job_call(&server,"openai.videos");
        call.model_params["poll_endpoint"]=json!("/videos/{id}");call.model_params["content_endpoint"]=json!("/videos/{id}/content");
        let job=crate::types::JobHandle {adapter_id:call.protocol.clone(),config_revision:1,remote_id:"job".into(),poll_state:json!({})};
        let error=super::super::openai_videos::OpenAiVideosAdapter.poll(&reqwest::Client::new(),&call,&job).await.unwrap_err();
        assert_eq!(error.kind,InvokeErrorKind::ParseError);
        assert_eq!(server.received_requests().await.unwrap().len(),2);
    }
}
