//! Deterministic protocol fixture for the actual Desktop App, never a native
//! replacement window. Only the separately launched product owns the browser.
use super::{Fixture, browser_tool, computer_result_text, contains_inline_png};
use serde_json::{Value, json};
use base64::Engine as _;

pub(super) const PAGE: &str = include_str!("wk_browser_fixture.html");
const TEXT: &str = "WK 主应用表单 中文 café 🙂";
const DIALOG_PROMPT:&str="Verify WK pending dialog cancellation.";

fn pending_dialog(body:&Value)->bool {
    body["messages"].as_array().into_iter().flatten().rev().find(|message|message["role"]=="user" && {
        let content=message["content"].to_string();
        content.contains(DIALOG_PROMPT)||content.contains("Verify the packaged WK Browser surface.")
    })
        .is_some_and(|message|message["content"].to_string().contains(DIALOG_PROMPT))
}

pub(super) fn call_id(body:&Value,step:usize)->String {
    format!("{}{step}",if pending_dialog(body){"gui-wk-dialog-"}else{"gui-wk-"})
}

pub(super) fn step(body:&Value)->usize {
    let prefix=if pending_dialog(body){"gui-wk-dialog-"}else{"gui-wk-"};
    body["messages"].as_array().into_iter().flatten()
        .filter(|message|message["role"]=="tool")
        .filter_map(|message|message["tool_call_id"].as_str()?.strip_prefix(prefix)?.parse::<usize>().ok())
        .max().map_or(0,|step|step+1)
}

fn result(body: &Value, step: usize) -> anyhow::Result<Value> {
    let text = computer_result_text(body, &call_id(body,step))?;
    let value: Value = serde_json::from_str(text)?;
    anyhow::ensure!(value.get("code").is_none(), "WK action returned an error at step {step}: {value}");
    Ok(value.get("result").cloned().unwrap_or(value))
}

fn element(body: &Value, step: usize, name: &str) -> anyhow::Result<Value> {
    let value = result(body, step)?;
    value["elements"].as_array()
        .and_then(|elements| elements.iter().find(|element| element["name"] == name))
        .cloned().ok_or_else(|| anyhow::anyhow!("WK fresh observed element missing: {name}"))
}

fn semantic(fixture: &Fixture, body: &Value, step: usize, name: &str) -> anyhow::Result<()> {
    let value = result(body, step)?;
    anyhow::ensure!(value["interaction_fidelity"] == "semantic_dom", "WK {name} misreported interaction fidelity");
    anyhow::ensure!(value["status"] == "completed", "WK {name} did not settle");
    fixture.wk_evidence.lock().unwrap().insert(name.to_owned(), json!(true));
    Ok(())
}

fn image_part(value: &Value) -> Option<&str> {
    match value {
        Value::String(value)=>value.strip_prefix("data:image/png;base64,"),
        Value::Array(values)=>values.iter().find_map(image_part),
        Value::Object(values)=>values.values().find_map(image_part),
        _=>None,
    }
}

pub(super) fn operation(fixture: &Fixture, body: &Value, step: usize) -> anyhow::Result<Option<(String, Value)>> {
    if pending_dialog(body) {return dialog_operation(fixture,body,step);}
    let url = fixture.native_url.as_deref().ok_or_else(|| anyhow::anyhow!("WK fixture URL missing"))?;
    let (action, input) = match step {
        0 => ("browser/navigate", json!({"url":url})),
        1 => { result(body, 0)?; ("browser/observe", json!({})) }
        2 => {
            let target=element(body,1,"提交验收")?;
            ("browser/act",json!({"action":"drag","from":target,"to":target}))
        }
        3 => {
            let error=computer_result_text(body,"gui-wk-2")?;
            anyhow::ensure!(error.contains("BROWSER_UNSUPPORTED_ACTION"),"WK drag did not return explicit unsupported");
            fixture.wk_evidence.lock().unwrap().insert("drag_explicitly_unsupported".into(),json!(true));
            ("browser/observe",json!({}))
        }
        4 => {
            // Store the observation in the model request's canonical history,
            // not a second identity registry. Reuse it only to prove rejection.
            element(body,3,"提交验收")?;
            ("browser/navigate",json!({"url":format!("{url}?after-navigation=1")}))
        }
        5 => { result(body,4)?; ("browser/act",json!({"action":"click","element":element(body,3,"提交验收")?})) }
        6 => {
            let error=computer_result_text(body,"gui-wk-5")?;
            anyhow::ensure!(error.contains("BROWSER_STALE_TARGET") || error.contains("BROWSER_STALE_OBSERVATION"),"WK old document reference was not rejected");
            fixture.wk_evidence.lock().unwrap().insert("old_document_reference_rejected".into(),json!(true));
            ("browser/observe",json!({}))
        }
        7 => ("browser/act", json!({"action":"type","element":element(body,6,"验收姓名")?,"text":TEXT})),
        8 => { semantic(fixture,body,7,"type_semantic_dom")?; ("browser/observe", json!({})) }
        9 => ("browser/act", json!({"action":"select","element":element(body,8,"验收选项")?,"labels":["Café 中文"]})),
        10 => { semantic(fixture,body,9,"select_semantic_dom")?; ("browser/observe",json!({})) }
        11 => ("browser/act", json!({"action":"scroll","element":element(body,10,"验收滚动区域")?,"delta_x":0,"delta_y":320})),
        12 => { semantic(fixture,body,11,"scroll_semantic_dom")?; ("browser/observe",json!({})) }
        13 => ("browser/act",json!({"action":"click","element":element(body,12,"提交验收")?})),
        14 => { semantic(fixture,body,13,"click_semantic_dom")?; ("browser/observe",json!({"screenshot":true})) }
        15 => {
            let screenshot_result=super::computer_tool_result(body,"gui-wk-14").ok_or_else(||anyhow::anyhow!("Exact first WK screenshot result missing"))?;
            anyhow::ensure!(contains_inline_png(screenshot_result),"First WK screenshot missing typed image");
            ("browser/act",json!({"action":"click","element":element(body,14,"用户计数")?}))
        }
        16 => { semantic(fixture,body,15,"post_screenshot_reference_usable")?; ("browser/observe",json!({"screenshot":true})) }
        17 => {
            let observed = result(body,16)?;
            let content = observed["content"].as_str().unwrap_or_default();
            anyhow::ensure!(content.contains(TEXT) && content.contains("Café 中文") && content.contains("滚动完成") && content.contains("isTrusted=false"), "WK result missing exact form, select, scroll or semantic event evidence");
            let screenshot_result=super::computer_tool_result(body,"gui-wk-16").ok_or_else(||anyhow::anyhow!("Exact WK screenshot tool result missing"))?;
            anyhow::ensure!(contains_inline_png(screenshot_result),"WK screenshot did not reach model as an image part");
            let encoded=image_part(screenshot_result).ok_or_else(||anyhow::anyhow!("WK screenshot image missing"))?;
            let bytes=base64::engine::general_purpose::STANDARD.decode(encoded)?;
            anyhow::ensure!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"),"WK screenshot is not PNG");
            let path=fixture.wk_screenshot.as_ref().ok_or_else(||anyhow::anyhow!("WK screenshot output missing"))?;
            use std::io::Write;
            let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
            file.write_all(&bytes)?;
            let mut evidence=fixture.wk_evidence.lock().unwrap();
            evidence.insert("screenshot_path".into(),json!(path));
            evidence.insert("screenshot_image_part".into(),json!(true));
            evidence.insert("form_result_observed".into(),json!(true));
            evidence.insert("ordinary_browser_survived_unsupported".into(),json!(true));
            // Browser is an atomic product capability, not workspace ledger
            // work. Obey only controls actually offered by this Runtime turn.
            if !body["tools"].as_array().is_some_and(|tools|tools.iter().any(|tool|tool["function"]["name"]=="report_completion")) {
                evidence.insert("completion_control".into(),json!("not_exposed"));
                evidence.insert("awaiting_terminal_release".into(),json!(true));
                return Ok(None);
            }
            return Ok(Some(("report_completion".into(),json!({
                "summary":"Actual product WK Browser navigation, semantic form actions, scrolling and screenshot completed. Unsupported drag and stale reference remain recorded as two expected tool errors.",
                "observed_tool_error_count":2,
                "criteria":[{"step":"Verify WK form, selection, scrolling and result","disposition":"supported","evidence_call_ids":["gui-wk-16"],"rationale":"The final current observation includes exact entered values, selected option, scroll evidence and isTrusted=false."},{"step":"Verify WK screenshot and capability boundaries","disposition":"supported","evidence_call_ids":["gui-wk-16"],"rationale":"The final observation includes the visible native screenshot and platform capability declarations, after successfully using an element reference returned alongside the previous screenshot. Earlier unsupported and stale-target results remain historical evidence in the summary."}]
            }))));
        }
        18 => {
            let completion=computer_result_text(body,"gui-wk-17")?;
            anyhow::ensure!(completion.contains("Completion account recorded"),"WK completion report was not accepted");
            fixture.wk_evidence.lock().unwrap().insert("awaiting_terminal_release".into(),json!(true));
            return Ok(None);
        }
        _ => anyhow::bail!("Unexpected WK model step {step}"),
    };
    Ok(Some((browser_tool(body,action)?,input)))
}

fn dialog_operation(fixture:&Fixture,body:&Value,step:usize)->anyhow::Result<Option<(String,Value)>> {
    let url=fixture.native_url.as_deref().ok_or_else(||anyhow::anyhow!("WK fixture URL missing"))?;
    let (action,input)=match step {
        0=>("browser/navigate",json!({"url":format!("{url}?case=pending-dialog#dialog")})),
        1=>{result(body,0)?;("browser/observe",json!({}))}
        2=>("browser/act",json!({"action":"click","element":element(body,1,"显示 Confirm")?})),
        3=>{
            let value=result(body,2)?;
            anyhow::ensure!(value["status"]=="awaiting_dialog","Confirm click did not retain its native callback");
            let mut evidence=fixture.wk_evidence.lock().unwrap();
            evidence.insert("pending_dialog_action_awaiting".into(),json!(true));
            evidence.insert("pending_dialog".into(),value["dialog"].clone());
            return Ok(None);
        }
        _=>anyhow::bail!("Unexpected pending-dialog model step {step}"),
    };
    Ok(Some((browser_tool(body,action)?,input)))
}
