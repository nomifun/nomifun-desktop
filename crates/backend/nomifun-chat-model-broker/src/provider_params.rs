//! Pure wire configuration shared by Agent and standalone model attempts.
use serde_json::Value;
use crate::contracts::{ChatProtocol, ChatModelError, ChatModelErrorCode, ChatRetryDirective};

pub fn merge_chat_provider_params(
    mut body: Value,
    configured: &Value,
    protocol: ChatProtocol,
    output_limit: Option<i64>,
) -> Result<Value, ChatModelError> {
    let configured = configured.as_object().ok_or_else(|| {
        ChatModelError::new(
            ChatModelErrorCode::InvalidRequest,
            "provider chat parameters are not a JSON object",
            ChatRetryDirective::Never,
        )
    })?;
    let body_object = body.as_object_mut().ok_or_else(|| {
        ChatModelError::new(
            ChatModelErrorCode::ProtocolViolation,
            "provider chat request body is not a JSON object",
            ChatRetryDirective::Never,
        )
    })?;
    let reasoning_effort = configured
        .get("reasoning_effort")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let typed_openai_output = if protocol == ChatProtocol::OpenaiChat {
        body_object.get("max_tokens").or_else(|| body_object.get("max_completion_tokens"))
            .or_else(|| body_object.get("max_output_tokens"))
            .or_else(|| configured.get("max_tokens_field").and_then(Value::as_str)
                .and_then(|key| body_object.get(key.trim()))).cloned()
    } else { None };
    let typed_gemini_thinking = body_object.get("generationConfig").and_then(|value| value.get("thinkingConfig"));
    let typed_gemini_level = typed_gemini_thinking.is_some_and(|value| value.get("thinkingLevel").is_some());
    let typed_gemini_budget = typed_gemini_thinking.is_some_and(|value| value.get("thinkingBudget").is_some());

    for (key, value) in configured {
        // Provider defaults may tune sampling/thinking; active thinking is
        // checked against the exact route before transport. Defaults cannot
        // replace model-owned messages/tools or reinsert cloud routing fields.
        if protocol.uses_anthropic_messages() && matches!(key.as_str(),
            "model" | "stream" | "messages" | "system" | "tools" | "tool_choice"
            | "anthropic_version") {
            continue;
        }
        if matches!(
            key.as_str(),
            "max_tokens_field"
                | "chain_rounds"
                | "require_reasoning_content"
                | "reasoning_effort"
                | "_nomifun_context_limit_kind"
                | nomifun_agent_contracts::MODEL_GATEWAY_CATALOG_BASELINE_PARAM
        ) {
            continue;
        }
        merge_missing_json_value(body_object, key, value);
    }
    if protocol == ChatProtocol::Gemini {
        if let Some(thinking) = body_object.get_mut("generationConfig")
            .and_then(|value| value.get_mut("thinkingConfig")).and_then(Value::as_object_mut) {
            // A caller's typed control overrides the model's default mode.
            // Level and budget are mutually exclusive provider controls.
            if typed_gemini_level && !typed_gemini_budget { thinking.remove("thinkingBudget"); }
            if typed_gemini_budget && !typed_gemini_level { thinking.remove("thinkingLevel"); }
        }
    }

    let configured_ceiling_key = configured
        .get("max_tokens_field")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|key| !key.trim().is_empty());
    let ceiling = output_limit
        .map(|limit| {
            u32::try_from(limit).map_err(|_| {
                ChatModelError::new(
                    ChatModelErrorCode::InvalidRequest,
                    "provider output ceiling exceeds the supported range",
                    ChatRetryDirective::Never,
                )
            })
        })
        .transpose()?;
    match protocol {
        ChatProtocol::Gemini => {
            if let Some(ceiling) = ceiling {
                let generation = body_object
                    .entry("generationConfig".to_owned())
                    .or_insert_with(|| Value::Object(Default::default()));
                let generation = generation.as_object_mut().ok_or_else(|| {
                    ChatModelError::protocol_violation(
                        "provider generationConfig is not a JSON object",
                    )
                })?;
                cap_json_number(generation, "maxOutputTokens", ceiling);
            }
        }
        ChatProtocol::OpenaiResponses => {
            if let Some(ceiling) = ceiling {
                cap_json_number(body_object, "max_output_tokens", ceiling);
            }
        }
        ChatProtocol::OpenaiChat => {
            let key = configured_ceiling_key.unwrap_or("max_tokens");
            // The Broker uses a canonical output field. Selecting a vendor's
            // wire alias changes its name, not the caller's requested amount.
            // This mapping also applies when the model leaves its own ceiling
            // unspecified, rather than leaking the legacy field in that case.
            let mut requested = typed_openai_output.or_else(|| body_object.remove(key));
            for default_key in ["max_tokens", "max_completion_tokens", "max_output_tokens"] {
                if let Some(value) = body_object.remove(default_key) {
                    if requested.is_none() { requested = Some(value); }
                }
            }
            if let Some(requested) = requested { body_object.insert(key.to_owned(), requested); }
            if let Some(ceiling) = ceiling {
                cap_json_number(body_object, key, ceiling);
            }
        }
        ChatProtocol::Anthropic | ChatProtocol::Bedrock | ChatProtocol::Vertex => {
            if let Some(ceiling) = ceiling {
                cap_json_number(body_object, "max_tokens", ceiling);
            }
            let max_tokens = body_object.get("max_tokens").and_then(Value::as_u64)
                .filter(|value| *value > 0)
                .ok_or_else(|| ChatModelError::invalid_request("Messages output ceiling must be positive"))?;
            if let Some(thinking) = body_object.get("thinking") {
                match thinking.get("type").and_then(Value::as_str) {
                    Some("enabled") => {
                        if !thinking.get("budget_tokens").and_then(Value::as_u64)
                            .is_some_and(|budget| budget >= 1024 && budget < max_tokens) {
                            return Err(ChatModelError::invalid_request(
                                "Messages thinking budget must remain below the effective provider output ceiling",
                            ));
                        }
                    }
                    Some("adaptive" | "disabled") if thinking.get("budget_tokens").is_none() => {}
                    _ => return Err(ChatModelError::invalid_request(
                        "Messages thinking must use enabled with a valid budget, or adaptive/disabled without budget_tokens",
                    )),
                }
                if matches!(thinking.get("type").and_then(Value::as_str), Some("enabled" | "adaptive"))
                    && matches!(body_object.get("tool_choice").and_then(|choice| choice.get("type")).and_then(Value::as_str), Some("any" | "tool")) {
                    return Err(ChatModelError::invalid_request(
                        "Messages thinking cannot force tool_choice any/tool; use auto or none",
                    ));
                }
            }
        }
    }
    if let Some(reasoning_effort) = reasoning_effort {
        let supported = match protocol {
            ChatProtocol::OpenaiChat | ChatProtocol::OpenaiResponses => matches!(
                reasoning_effort.as_str(),
                "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "ultra"
            ),
            ChatProtocol::Gemini => {
                matches!(reasoning_effort.as_str(), "minimal" | "low" | "medium" | "high")
            }
            ChatProtocol::Anthropic | ChatProtocol::Bedrock | ChatProtocol::Vertex => false,
        };
        if !supported {
            return Err(ChatModelError::invalid_request(
                "configured reasoning effort is unsupported by this Chat protocol",
            ));
        }
        match protocol {
            ChatProtocol::OpenaiChat => {
                body_object
                    .entry("reasoning_effort")
                    .or_insert_with(|| Value::String(reasoning_effort));
            }
            ChatProtocol::OpenaiResponses => {
                let reasoning = body_object
                    .entry("reasoning".to_owned())
                    .or_insert_with(|| Value::Object(Default::default()));
                let reasoning = reasoning.as_object_mut().ok_or_else(|| {
                    ChatModelError::invalid_request(
                        "OpenAI Responses reasoning configuration must be an object",
                    )
                })?;
                reasoning
                    .entry("effort")
                    .or_insert_with(|| Value::String(reasoning_effort));
            }
            ChatProtocol::Gemini if !typed_gemini_budget => {
                let generation = body_object
                    .entry("generationConfig".to_owned())
                    .or_insert_with(|| Value::Object(Default::default()));
                let generation = generation.as_object_mut().ok_or_else(|| {
                    ChatModelError::invalid_request(
                        "Gemini generationConfig must be an object",
                    )
                })?;
                let thinking = generation
                    .entry("thinkingConfig".to_owned())
                    .or_insert_with(|| Value::Object(Default::default()));
                let thinking = thinking.as_object_mut().ok_or_else(|| {
                    ChatModelError::invalid_request(
                        "Gemini thinkingConfig must be an object",
                    )
                })?;
                thinking
                    .entry("thinkingLevel")
                    .or_insert_with(|| Value::String(reasoning_effort));
            }
            ChatProtocol::Gemini => {},
            ChatProtocol::Anthropic | ChatProtocol::Bedrock | ChatProtocol::Vertex => unreachable!(
                "unsupported reasoning protocol returned before provider default merge"
            ),
        }
    }
    if protocol == ChatProtocol::Gemini && body_object.get("generationConfig")
        .and_then(|value| value.get("thinkingConfig"))
        .is_some_and(|thinking| thinking.get("thinkingLevel").is_some() && thinking.get("thinkingBudget").is_some()) {
        return Err(ChatModelError::invalid_request(
            "Gemini thinkingConfig cannot combine thinkingLevel and thinkingBudget; choose one control",
        ));
    }
    Ok(body)
}

fn merge_missing_json_value(
    target: &mut serde_json::Map<String, Value>,
    key: &str,
    incoming: &Value,
) {
    match target.get_mut(key) {
        Some(Value::Object(existing)) => {
            if let Value::Object(incoming) = incoming {
                for (nested_key, nested_value) in incoming {
                    if let Some(Value::Object(existing_nested)) = existing.get_mut(nested_key)
                        && let Value::Object(incoming_nested) = nested_value
                    {
                        merge_json_object_missing(existing_nested, incoming_nested);
                    } else {
                        existing
                            .entry(nested_key.clone())
                            .or_insert_with(|| nested_value.clone());
                    }
                }
            }
        }
        Some(_) => {}
        None => {
            target.insert(key.to_owned(), incoming.clone());
        }
    }
}

fn merge_json_object_missing(
    target: &mut serde_json::Map<String, Value>,
    incoming: &serde_json::Map<String, Value>,
) {
    for (key, value) in incoming {
        if let Some(Value::Object(existing)) = target.get_mut(key)
            && let Value::Object(incoming) = value
        {
            merge_json_object_missing(existing, incoming);
        } else {
            target.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
}

fn cap_json_number(
    object: &mut serde_json::Map<String, Value>,
    key: &str,
    ceiling: u32,
) {
    let ceiling = u64::from(ceiling);
    match object.get(key).and_then(Value::as_u64) {
        Some(current) if current > ceiling => {
            object.insert(key.to_owned(), Value::from(ceiling));
        }
        None => {
            object.insert(key.to_owned(), Value::from(ceiling));
        }
        _ => {}
    }
}

pub fn configure_standalone_non_streaming(
    protocol: ChatProtocol,
    url: &mut String,
    body: &mut Value,
) -> Result<(), ChatModelError> {
    let object = body.as_object_mut().ok_or_else(|| {
        ChatModelError::new(
            ChatModelErrorCode::ProtocolViolation,
            "provider chat request body is not a JSON object",
            ChatRetryDirective::Never,
        )
    })?;
    match protocol {
        ChatProtocol::OpenaiChat => {
            object.insert("stream".into(), Value::Bool(false));
            object.remove("stream_options");
        }
        ChatProtocol::OpenaiResponses | ChatProtocol::Anthropic | ChatProtocol::Vertex => {
            object.insert("stream".into(), Value::Bool(false));
        }
        ChatProtocol::Gemini => {
            let mut parsed = url::Url::parse(url).map_err(|_| {
                ChatModelError::new(
                    ChatModelErrorCode::AdapterUnavailable,
                    "configured Gemini endpoint is invalid",
                    ChatRetryDirective::Never,
                )
            })?;
            let path = parsed.path().replace(":streamGenerateContent", ":generateContent");
            parsed.set_path(&path);
            let query = parsed
                .query_pairs()
                .filter(|(key, value)| !(key == "alt" && value == "sse"))
                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                .collect::<Vec<_>>();
            parsed.set_query(None);
            if !query.is_empty() {
                parsed.query_pairs_mut().extend_pairs(query);
            }
            *url = parsed.into();
        }
        ChatProtocol::Bedrock => {
            *url = url.replace("/invoke-with-response-stream", "/invoke");
        }
    }
    // The executor's bounded JSON framing turns one response object into one
    // broker frame; no SSE/AWS event-stream parser is used in this mode.
    Ok(())
}

pub fn chat_request_uses_reasoning(protocol: ChatProtocol, body: &Value) -> bool {
    match protocol {
            ChatProtocol::Anthropic | ChatProtocol::Bedrock | ChatProtocol::Vertex => body.get("thinking")
                .and_then(|value| value.get("type")).and_then(Value::as_str)
                .is_some_and(|kind| matches!(kind, "enabled" | "adaptive")),
            ChatProtocol::OpenaiChat => body.get("reasoning_effort").and_then(Value::as_str)
                .is_some_and(|effort| effort != "none"),
            ChatProtocol::OpenaiResponses => body.get("reasoning").and_then(Value::as_object)
                .is_some_and(|reasoning| !reasoning.is_empty() && reasoning.get("effort").and_then(Value::as_str) != Some("none")),
            ChatProtocol::Gemini => body.get("generationConfig").and_then(|value| value.get("thinkingConfig"))
                .is_some_and(|thinking| thinking.get("thinkingLevel").is_some()
                    || thinking.get("thinkingBudget").and_then(Value::as_i64).is_some_and(|budget| budget != 0)
                    || thinking.get("includeThoughts").and_then(Value::as_bool) == Some(true)),
    }
}
