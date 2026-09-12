//! Server-side execution owner for `workshop.template.run`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use nomifun_agent_domain_wave3::Wave3HostPortError;
use nomifun_creation::{
    CreationInput, CreationInputKind, CreationService, CreativeTaskOwner, NewCreationTask,
};
use nomifun_workshop::template::{
    CreativePromptTemplateSegment, CreativeTemplateImageQuality, CreativeTemplateImageTask,
    CreativeTemplateOutputPlan, CreativeTemplatePromptSource, CreativeTemplateStep,
};
use nomifun_workshop::template_run::{
    CreativeTemplateInputValue, CreativeTemplatePromptDraft, CreativeTemplatePromptDraftStatus,
    CreativeTemplateRunAggregateV1, CreativeTemplateRunCreateRequest,
    CreativeTemplateRunFailure, CreativeTemplateRunStatus,
};
use nomifun_workshop::WorkshopService;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub(crate) const HUMAN_REVIEW_REQUIRED: &str = "HUMAN_REVIEW_REQUIRED";
const TEMPLATE_RUN_FAILED: &str = "WAVE3_TEMPLATE_RUN_FAILED";
const TEMPLATE_RUN_CANCELED: &str = "WAVE3_TEMPLATE_RUN_CANCELED";
const TEMPLATE_TASK_TIMEOUT_CANCELED: &str = "WAVE3_TEMPLATE_TASK_TIMEOUT_CANCELED";
const TEMPLATE_TASK_OUTCOME_UNKNOWN: &str = "WAVE3_TEMPLATE_TASK_OUTCOME_UNKNOWN";
const TASK_POLL_INTERVAL: Duration = Duration::from_millis(200);
const TASK_WAIT_LIMIT: Duration = Duration::from_secs(300);

#[derive(Clone)]
pub(crate) struct NomiWave3TemplateRunner {
    workshop: Arc<WorkshopService>,
    creation: Arc<CreationService>,
    run_locks: Arc<tokio::sync::Mutex<BTreeMap<String, Weak<tokio::sync::Mutex<()>>>>>,
}

impl NomiWave3TemplateRunner {
    pub(crate) fn new(
        workshop: Arc<WorkshopService>,
        creation: Arc<CreationService>,
    ) -> Self {
        Self {
            workshop,
            creation,
            run_locks: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
        }
    }

    pub(crate) async fn run(
        &self,
        request: CreativeTemplateRunCreateRequest,
    ) -> Result<CreativeTemplateRunAggregateV1, Wave3HostPortError> {
        let run_lock = {
            let mut locks = self.run_locks.lock().await;
            match locks
                .get(&request.template_run_id)
                .and_then(Weak::upgrade)
            {
                Some(lock) => lock,
                None => {
                    let lock = Arc::new(tokio::sync::Mutex::new(()));
                    locks.insert(request.template_run_id.clone(), Arc::downgrade(&lock));
                    lock
                }
            }
        };
        let _run_guard = run_lock.lock().await;
        let definition = self
            .workshop
            .get_creative_template(&request.template_id)
            .await
            .map_err(map_error)?;
        if definition.revision != request.template_revision {
            return Err(Wave3HostPortError::new(
                "WAVE3_TEMPLATE_REVISION_CONFLICT",
                "selected template revision changed",
            ));
        }
        if matches!(
            definition.output,
            CreativeTemplateOutputPlan::MultiImageSeries {
                review_required: true,
                ..
            }
        ) {
            return Err(Wave3HostPortError::new(
                HUMAN_REVIEW_REQUIRED,
                "this template requires Creative Studio prompt review before Apply; open the template run UI",
            ));
        }

        let run = self
            .workshop
            .create_creative_template_run(request)
            .await
            .map_err(map_error)?;
        if run.record.status.is_terminal() {
            return terminal_run_result(run);
        }
        match self.advance(run).await {
            Ok(run) => Ok(run),
            Err((_run, error)) if error.code == TEMPLATE_TASK_OUTCOME_UNKNOWN => Err(error),
            Err((run, error)) => {
                let code = error.code.clone();
                let message = error.message.clone();
                let failed = self.persist_failure(run, error).await?;
                Err(Wave3HostPortError::new(
                    TEMPLATE_RUN_FAILED,
                    format!(
                        "template run {} failed [{code}]: {message}",
                        failed.request.id
                    ),
                ))
            }
        }
    }

    async fn advance(
        &self,
        mut run: CreativeTemplateRunAggregateV1,
    ) -> Result<CreativeTemplateRunAggregateV1, (CreativeTemplateRunAggregateV1, Wave3HostPortError)>
    {
        let plan = match task_plan(&run) {
            Ok(plan) => plan,
            Err(error) => return Err((run, error)),
        };
        if run.record.status == CreativeTemplateRunStatus::Requested {
            let mut next = run.clone();
            next.revision += 1;
            next.record.status = CreativeTemplateRunStatus::Queued;
            next.record.task_ids = plan.iter().map(|entry| entry.task_id.clone()).collect();
            next.record.queued_at = Some(nomifun_common::now_ms());
            run = match self.persist(&run, next).await {
                Ok(run) => run,
                Err(error) => return Err((run, error)),
            };
        }
        if run.record.status == CreativeTemplateRunStatus::Queued {
            let mut next = run.clone();
            next.revision += 1;
            next.record.status = CreativeTemplateRunStatus::Running;
            next.record.started_at = Some(nomifun_common::now_ms());
            run = match self.persist(&run, next).await {
                Ok(run) => run,
                Err(error) => return Err((run, error)),
            };
        }
        if run.record.status != CreativeTemplateRunStatus::Running {
            return Err((
                run,
                Wave3HostPortError::new(TEMPLATE_RUN_FAILED, "template run is not executable"),
            ));
        }

        if run.prompt_drafts.is_empty()
            && let Some(planner) = plan.iter().find(|entry| entry.kind == PlanKind::Planner)
        {
            let task = match self.create_planner_task(&run, planner).await {
                Ok(task) => task,
                Err(error) => return Err((run, error)),
            };
            let Some(asset_id) = task.result_asset_ids.first() else {
                return Err((
                    run,
                    Wave3HostPortError::new(TEMPLATE_RUN_FAILED, "planner produced no text asset"),
                ));
            };
            let asset = match self.workshop.get_asset(asset_id).await.map_err(map_error) {
                Ok(asset) => asset,
                Err(error) => return Err((run, error)),
            };
            let Some(text) = asset.text_content else {
                return Err((
                    run,
                    Wave3HostPortError::new(TEMPLATE_RUN_FAILED, "planner result is not text"),
                ));
            };
            let drafts = match planner_drafts(&run, &text) {
                Ok(drafts) => drafts,
                Err(error) => return Err((run, error)),
            };
            let mut next = run.clone();
            next.revision += 1;
            next.prompt_drafts = drafts;
            next.record.prompt_draft_ids = next
                .prompt_drafts
                .iter()
                .map(|draft| draft.id.clone())
                .collect();
            run = match self.persist(&run, next).await {
                Ok(run) => run,
                Err(error) => return Err((run, error)),
            };
        }

        let mut results = Vec::new();
        for entry in plan.iter().filter(|entry| entry.kind == PlanKind::Image) {
            let task = match self.create_image_task(&run, entry).await {
                Ok(task) => task,
                Err(error) => return Err((run, error)),
            };
            results.extend(task.result_asset_ids);
        }
        let mut next = run.clone();
        next.revision += 1;
        next.record.status = CreativeTemplateRunStatus::Succeeded;
        next.record.result_asset_ids = results;
        next.record.completed_at = Some(nomifun_common::now_ms());
        match self.persist(&run, next).await {
            Ok(run) => Ok(run),
            Err(error) => Err((run, error)),
        }
    }

    async fn create_planner_task(
        &self,
        run: &CreativeTemplateRunAggregateV1,
        entry: &PlanEntry,
    ) -> Result<nomifun_creation::CreationTask, Wave3HostPortError> {
        let CreativeTemplateStep::DraftPrompts {
            id,
            template_id,
            planning,
            ..
        } = &run.template_snapshot.steps[entry.step_index]
        else {
            return Err(Wave3HostPortError::new(TEMPLATE_RUN_FAILED, "invalid planner plan"));
        };
        let model = planning.model.as_ref().ok_or_else(|| {
            Wave3HostPortError::new(TEMPLATE_RUN_FAILED, "planner step has no Chat model")
        })?;
        let count = match run.template_snapshot.output {
            CreativeTemplateOutputPlan::MultiImageSeries { target_count, .. } => target_count,
            CreativeTemplateOutputPlan::SingleImage => {
                return Err(Wave3HostPortError::new(
                    TEMPLATE_RUN_FAILED,
                    "single-image template cannot run a planner",
                ));
            }
        };
        let brief = render_prompt(run, template_id)?;
        let params = json!({
            "system": format!(
                "{}\nReturn only one JSON object with exactly {count} prompts. The exact schema is {{\"prompts\":[{{\"title\":\"...\",\"prompt\":\"...\"}}]}}. Do not add markdown fences, commentary, extra fields, or trailing text.",
                planning.instruction.trim()
            ),
            "prompt": format!("Create {count} production-ready image prompts from the following brief.\n<brief>\n{brief}\n</brief>"),
            "max_tokens": planning.max_tokens,
        });
        self.create_and_wait(
            run,
            entry,
            id,
            &model.provider_id,
            &model.model,
            "text",
            params,
            Vec::new(),
        )
        .await
    }

    async fn create_image_task(
        &self,
        run: &CreativeTemplateRunAggregateV1,
        entry: &PlanEntry,
    ) -> Result<nomifun_creation::CreationTask, Wave3HostPortError> {
        let CreativeTemplateStep::GenerateImages {
            id,
            prompt_source,
            reference_variable_ids,
            generation,
            ..
        } = &run.template_snapshot.steps[entry.step_index]
        else {
            return Err(Wave3HostPortError::new(TEMPLATE_RUN_FAILED, "invalid image plan"));
        };
        let model = generation.model.as_ref().ok_or_else(|| {
            Wave3HostPortError::new(TEMPLATE_RUN_FAILED, "image step has no model")
        })?;
        let prompt = match prompt_source {
            CreativeTemplatePromptSource::Template { template_id } => {
                render_prompt(run, template_id)?
            }
            CreativeTemplatePromptSource::PromptDrafts { .. } => run
                .prompt_drafts
                .iter()
                .find(|draft| Some(draft.series_index) == entry.series_index)
                .filter(|draft| draft.status == CreativeTemplatePromptDraftStatus::Approved)
                .map(|draft| draft.prompt.clone())
                .ok_or_else(|| {
                    Wave3HostPortError::new(TEMPLATE_RUN_FAILED, "image step has no approved prompt")
                })?,
        };
        let mut asset_ids = run.request.reference_asset_ids.clone();
        for input in &run.request.inputs {
            if reference_variable_ids.contains(&input_variable_id(input).to_owned()) {
                append_input_assets(input, &mut asset_ids);
            }
        }
        let mut seen = BTreeSet::new();
        asset_ids.retain(|id| seen.insert(id.clone()));
        let capability = match model.task {
            CreativeTemplateImageTask::ImageGeneration if asset_ids.is_empty() => "t2i",
            CreativeTemplateImageTask::ImageEdit if !asset_ids.is_empty() => "i2i",
            CreativeTemplateImageTask::ImageGeneration => {
                return Err(Wave3HostPortError::new(
                    TEMPLATE_RUN_FAILED,
                    "image-generation step cannot receive reference assets",
                ));
            }
            CreativeTemplateImageTask::ImageEdit => {
                return Err(Wave3HostPortError::new(
                    TEMPLATE_RUN_FAILED,
                    "image-edit step requires reference assets",
                ));
            }
        };
        let quality = match generation.quality {
            CreativeTemplateImageQuality::Auto => "auto",
            CreativeTemplateImageQuality::High => "high",
            CreativeTemplateImageQuality::Medium => "medium",
            CreativeTemplateImageQuality::Low => "low",
        };
        let params = json!({
            "prompt": prompt,
            "interface_mode": "images",
            "quality": quality,
            "count": generation.images_per_prompt,
            "width": generation.width,
            "height": generation.height,
        });
        let inputs = asset_ids
            .into_iter()
            .map(|asset_id| CreationInput {
                asset_id,
                kind: CreationInputKind::Image,
                role: "reference".into(),
            })
            .collect();
        self.create_and_wait(
            run,
            entry,
            id,
            &model.provider_id,
            &model.model,
            capability,
            params,
            inputs,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn create_and_wait(
        &self,
        run: &CreativeTemplateRunAggregateV1,
        entry: &PlanEntry,
        step_id: &str,
        provider_id: &str,
        model: &str,
        capability: &str,
        params: Value,
        inputs: Vec<CreationInput>,
    ) -> Result<nomifun_creation::CreationTask, Wave3HostPortError> {
        let task = self
            .creation
            .create_creative_task(
                CreativeTaskOwner::TemplateStep {
                    template_id: run.request.template_id.clone(),
                    template_run_id: run.request.id.clone(),
                    template_step_id: step_id.to_owned(),
                },
                entry.task_id.clone(),
                NewCreationTask {
                    provider_id: provider_id.to_owned(),
                    model: model.to_owned(),
                    capability: capability.to_owned(),
                    params,
                    inputs,
                },
            )
            .await
            .map_err(map_error)?;
        if matches!(task.status.as_str(), "succeeded" | "failed" | "canceled") {
            return terminal_task(task);
        }
        let started = Instant::now();
        loop {
            if started.elapsed() >= TASK_WAIT_LIMIT {
                return match self.creation.cancel_task(&entry.task_id).await {
                    Ok(canceled) if canceled.status == "canceled" => Err(
                        Wave3HostPortError::new(
                            TEMPLATE_TASK_TIMEOUT_CANCELED,
                            format!(
                                "template task {} timed out and was durably canceled",
                                entry.task_id
                            ),
                        ),
                    ),
                    Ok(completed) if completed.status == "succeeded" => Ok(completed),
                    Ok(completed) if completed.status == "failed" => terminal_task(completed),
                    Ok(observed) => Err(Wave3HostPortError::new(
                        TEMPLATE_TASK_OUTCOME_UNKNOWN,
                        format!(
                            "template task {} timed out; cancellation returned {}. Observe the creation task before retrying",
                            observed.creation_task_id, observed.status
                        ),
                    )),
                    Err(error) => Err(Wave3HostPortError::new(
                        TEMPLATE_TASK_OUTCOME_UNKNOWN,
                        format!(
                            "template task {} timed out and cancellation could not be confirmed: {error}. Observe the creation task before retrying",
                            entry.task_id
                        ),
                    )),
                };
            }
            tokio::time::sleep(TASK_POLL_INTERVAL).await;
            let task = self
                .creation
                .get_task(&entry.task_id)
                .await
                .map_err(map_error)?;
            if matches!(task.status.as_str(), "succeeded" | "failed" | "canceled") {
                return terminal_task(task);
            }
        }
    }

    async fn persist(
        &self,
        current: &CreativeTemplateRunAggregateV1,
        next: CreativeTemplateRunAggregateV1,
    ) -> Result<CreativeTemplateRunAggregateV1, Wave3HostPortError> {
        self.workshop
            .save_creative_template_run(&current.request.id, &current.revision.to_string(), next)
            .await
            .map_err(map_error)
    }

    async fn persist_failure(
        &self,
        run: CreativeTemplateRunAggregateV1,
        error: Wave3HostPortError,
    ) -> Result<CreativeTemplateRunAggregateV1, Wave3HostPortError> {
        if run.record.status.is_terminal() {
            return Ok(run);
        }
        let mut failed = run.clone();
        failed.revision += 1;
        failed.record.status = CreativeTemplateRunStatus::Failed;
        failed.record.completed_at = Some(nomifun_common::now_ms());
        failed.record.failure = Some(CreativeTemplateRunFailure {
            code: failure_code(&error.code),
            message: error.message.chars().take(2_000).collect(),
        });
        self.persist(&run, failed).await
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PlanKind {
    Planner,
    Image,
}

struct PlanEntry {
    kind: PlanKind,
    step_index: usize,
    series_index: Option<usize>,
    task_id: String,
}

fn task_plan(run: &CreativeTemplateRunAggregateV1) -> Result<Vec<PlanEntry>, Wave3HostPortError> {
    let order = topological_order(&run.template_snapshot.steps)?;
    let mut descriptors = Vec::new();
    for index in order {
        match &run.template_snapshot.steps[index] {
            CreativeTemplateStep::DraftPrompts { enabled: true, .. } => {
                descriptors.push((PlanKind::Planner, index, None));
            }
            CreativeTemplateStep::GenerateImages {
                enabled: true,
                prompt_source: CreativeTemplatePromptSource::PromptDrafts { .. },
                ..
            } => {
                let count = match run.template_snapshot.output {
                    CreativeTemplateOutputPlan::MultiImageSeries { target_count, .. } => target_count,
                    CreativeTemplateOutputPlan::SingleImage => 1,
                };
                descriptors.extend((0..count).map(|series| (PlanKind::Image, index, Some(series))));
            }
            CreativeTemplateStep::GenerateImages { enabled: true, .. } => {
                descriptors.push((PlanKind::Image, index, None));
            }
            _ => {}
        }
    }
    if descriptors.is_empty() {
        return Err(Wave3HostPortError::new(TEMPLATE_RUN_FAILED, "template has no tasks"));
    }
    if descriptors.iter().filter(|(kind, _, _)| *kind == PlanKind::Planner).count() > 1 {
        return Err(Wave3HostPortError::new(
            TEMPLATE_RUN_FAILED,
            "template has more than one prompt planner",
        ));
    }
    let existing = &run.record.task_ids;
    if !existing.is_empty() && existing.len() != descriptors.len() {
        return Err(Wave3HostPortError::new(
            TEMPLATE_RUN_FAILED,
            "persisted task plan length differs from template",
        ));
    }
    Ok(descriptors
        .into_iter()
        .enumerate()
        .map(|(position, (kind, step_index, series_index))| PlanEntry {
            kind,
            step_index,
            series_index,
            task_id: existing
                .get(position)
                .cloned()
                .unwrap_or_else(|| stable_uuid(&run.request.id, "task", position)),
        })
        .collect())
}

fn topological_order(steps: &[CreativeTemplateStep]) -> Result<Vec<usize>, Wave3HostPortError> {
    let ids = steps
        .iter()
        .enumerate()
        .map(|(index, step)| (step_common(step).0.to_owned(), index))
        .collect::<BTreeMap<_, _>>();
    let mut remaining = (0..steps.len()).collect::<BTreeSet<_>>();
    let mut emitted = BTreeSet::new();
    let mut order = Vec::new();
    while !remaining.is_empty() {
        let ready = remaining
            .iter()
            .copied()
            .find(|index| step_common(&steps[*index]).1.iter().all(|id| emitted.contains(id)));
        let Some(index) = ready else {
            return Err(Wave3HostPortError::new(TEMPLATE_RUN_FAILED, "template task graph is cyclic"));
        };
        if step_common(&steps[index]).1.iter().any(|id| !ids.contains_key(id)) {
            return Err(Wave3HostPortError::new(TEMPLATE_RUN_FAILED, "template task dependency is missing"));
        }
        remaining.remove(&index);
        emitted.insert(step_common(&steps[index]).0.to_owned());
        order.push(index);
    }
    Ok(order)
}

fn step_common(step: &CreativeTemplateStep) -> (&str, &[String]) {
    match step {
        CreativeTemplateStep::RenderTemplate { id, depends_on, .. }
        | CreativeTemplateStep::DraftPrompts { id, depends_on, .. }
        | CreativeTemplateStep::GenerateImages { id, depends_on, .. }
        | CreativeTemplateStep::RecordHistory { id, depends_on, .. } => (id, depends_on),
    }
}

fn render_prompt(
    run: &CreativeTemplateRunAggregateV1,
    template_id: &str,
) -> Result<String, Wave3HostPortError> {
    let template = run
        .template_snapshot
        .templates
        .iter()
        .find(|template| template.id == template_id)
        .ok_or_else(|| Wave3HostPortError::new(TEMPLATE_RUN_FAILED, "prompt template is missing"))?;
    let inputs = run
        .request
        .inputs
        .iter()
        .map(|input| (input_variable_id(input), input))
        .collect::<BTreeMap<_, _>>();
    let mut rendered = String::new();
    for segment in &template.segments {
        match segment {
            CreativePromptTemplateSegment::Text { text } => rendered.push_str(text),
            CreativePromptTemplateSegment::Variable { variable_id } => {
                let input = inputs.get(variable_id.as_str()).ok_or_else(|| {
                    Wave3HostPortError::new(TEMPLATE_RUN_FAILED, "prompt variable input is missing")
                })?;
                rendered.push_str(&input_text(input)?);
            }
        }
    }
    Ok(rendered)
}

fn input_variable_id(input: &CreativeTemplateInputValue) -> &str {
    match input {
        CreativeTemplateInputValue::Text { variable_id, .. }
        | CreativeTemplateInputValue::MultilineText { variable_id, .. }
        | CreativeTemplateInputValue::Number { variable_id, .. }
        | CreativeTemplateInputValue::Boolean { variable_id, .. }
        | CreativeTemplateInputValue::Choice { variable_id, .. }
        | CreativeTemplateInputValue::Image { variable_id, .. }
        | CreativeTemplateInputValue::ImageSeries { variable_id, .. } => variable_id,
    }
}

fn input_text(input: &CreativeTemplateInputValue) -> Result<String, Wave3HostPortError> {
    match input {
        CreativeTemplateInputValue::Text { value, .. }
        | CreativeTemplateInputValue::MultilineText { value, .. }
        | CreativeTemplateInputValue::Choice { value, .. } => Ok(value.clone()),
        CreativeTemplateInputValue::Number { value, .. } => Ok(value.to_string()),
        CreativeTemplateInputValue::Boolean { value, .. } => Ok(value.to_string()),
        CreativeTemplateInputValue::Image { .. }
        | CreativeTemplateInputValue::ImageSeries { .. } => Err(Wave3HostPortError::new(
            TEMPLATE_RUN_FAILED,
            "image inputs cannot be interpolated into prompt text",
        )),
    }
}

fn append_input_assets(input: &CreativeTemplateInputValue, target: &mut Vec<String>) {
    match input {
        CreativeTemplateInputValue::Image { asset_id: Some(id), .. } => target.push(id.clone()),
        CreativeTemplateInputValue::ImageSeries { asset_ids, .. } => target.extend(asset_ids.clone()),
        _ => {}
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlannerOutput {
    prompts: Vec<PlannerPrompt>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlannerPrompt {
    title: String,
    prompt: String,
}

fn planner_drafts(
    run: &CreativeTemplateRunAggregateV1,
    text: &str,
) -> Result<Vec<CreativeTemplatePromptDraft>, Wave3HostPortError> {
    let output: PlannerOutput = serde_json::from_str(text).map_err(|error| {
        Wave3HostPortError::new(TEMPLATE_RUN_FAILED, format!("planner output is invalid: {error}"))
    })?;
    let expected = match run.template_snapshot.output {
        CreativeTemplateOutputPlan::MultiImageSeries { target_count, .. } => target_count,
        CreativeTemplateOutputPlan::SingleImage => 0,
    };
    if output.prompts.len() != expected {
        return Err(Wave3HostPortError::new(
            TEMPLATE_RUN_FAILED,
            "planner output count differs from template",
        ));
    }
    let now = nomifun_common::now_ms();
    output
        .prompts
        .into_iter()
        .enumerate()
        .map(|(index, prompt)| {
            if prompt.title.trim().is_empty()
                || prompt.title.chars().count() > 120
                || prompt.prompt.trim().is_empty()
                || prompt.prompt.chars().count() > 200_000
            {
                return Err(Wave3HostPortError::new(
                    TEMPLATE_RUN_FAILED,
                    "planner prompt is empty or exceeds product bounds",
                ));
            }
            Ok(CreativeTemplatePromptDraft {
                id: stable_uuid(&run.request.id, "draft", index),
                template_id: run.request.template_id.clone(),
                run_request_id: run.request.id.clone(),
                series_index: index,
                title: prompt.title,
                prompt: prompt.prompt,
                status: CreativeTemplatePromptDraftStatus::Approved,
                created_at: now,
                reviewed_at: Some(now),
                review_note: None,
            })
        })
        .collect()
}

fn stable_uuid(run_id: &str, kind: &str, index: usize) -> String {
    let mut digest = Sha256::new();
    digest.update(run_id.as_bytes());
    digest.update([0]);
    digest.update(kind.as_bytes());
    digest.update((index as u64).to_be_bytes());
    let digest = digest.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x70;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes).to_string()
}

fn failure_code(value: &str) -> String {
    let normalized = value
        .to_ascii_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '.' | '_' | '-')
            {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    let trimmed = normalized.trim_matches(|character: char| !character.is_ascii_lowercase());
    if trimmed.is_empty() {
        "template_run_failed".into()
    } else {
        trimmed.chars().take(80).collect()
    }
}

fn terminal_task(
    task: nomifun_creation::CreationTask,
) -> Result<nomifun_creation::CreationTask, Wave3HostPortError> {
    match task.status.as_str() {
        "succeeded" => Ok(task),
        "failed" | "canceled" => Err(Wave3HostPortError::new(
            TEMPLATE_RUN_FAILED,
            task.error
                .as_ref()
                .map(Value::to_string)
                .unwrap_or_else(|| format!("template task ended as {}", task.status)),
        )),
        _ => Err(Wave3HostPortError::new(
            TEMPLATE_RUN_FAILED,
            "template task is not terminal",
        )),
    }
}

fn terminal_run_result(
    run: CreativeTemplateRunAggregateV1,
) -> Result<CreativeTemplateRunAggregateV1, Wave3HostPortError> {
    match run.record.status {
        CreativeTemplateRunStatus::Succeeded => Ok(run),
        CreativeTemplateRunStatus::Failed => {
            let failure = run.record.failure.as_ref();
            Err(Wave3HostPortError::new(
                TEMPLATE_RUN_FAILED,
                failure
                    .map(|failure| format!("[{}] {}", failure.code, failure.message))
                    .unwrap_or_else(|| format!("template run {} failed", run.request.id)),
            ))
        }
        CreativeTemplateRunStatus::Cancelled => Err(Wave3HostPortError::new(
            TEMPLATE_RUN_CANCELED,
            format!("template run {} was canceled", run.request.id),
        )),
        _ => Err(Wave3HostPortError::new(
            TEMPLATE_TASK_OUTCOME_UNKNOWN,
            format!("template run {} is not terminal", run.request.id),
        )),
    }
}

fn map_error(error: nomifun_common::AppError) -> Wave3HostPortError {
    Wave3HostPortError::new(error.error_code(), error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_common::{ProviderId, UserId, generate_id};
    use nomifun_db::{
        SqliteCreationTaskRepository, SqliteWorkshopRepository,
        init_database_memory_with_owner,
    };
    use nomifun_workshop::template::{
        CreativePromptTemplate, CreativeTemplateDefinitionV1,
        CreativeTemplateImageGenerationSettings, CreativeTemplateImageModelBinding,
        CreativeTemplateMetadata, CreativeTemplatePromptPlanningSettings,
        CreativeTemplateTextModelBinding, CreativeTemplateTextTask, CreativeTemplateVariable,
        CreativeTemplateVisibility,
    };

    async fn seed_provider(database: &nomifun_db::Database) -> String {
        let provider_id = ProviderId::new().into_string();
        sqlx::query(
            "INSERT INTO providers \
             (provider_id, platform, name, base_url, auth_scheme, credentials_encrypted, enabled, created_at, updated_at) \
             VALUES (?, 'openai', 'Template Runner', 'https://example.invalid', 'bearer', '', 1, 0, 0)",
        )
        .bind(&provider_id)
        .execute(database.pool())
        .await
        .expect("provider");
        sqlx::query(
            "INSERT INTO provider_models \
             (provider_id, model, enabled, sort_order, description, created_at, updated_at) \
             VALUES (?, 'image-model', 1, 0, NULL, 0, 0)",
        )
        .bind(&provider_id)
        .execute(database.pool())
        .await
        .expect("model");
        sqlx::query(
            "INSERT INTO provider_model_capabilities \
             (provider_id, model, task, traits, protocol, connection_role, provider_params, created_at, updated_at) \
             VALUES (?, 'image-model', 'image_generation', '[]', 'openai.images', 'default', '{}', 0, 0)",
        )
        .bind(&provider_id)
        .execute(database.pool())
        .await
        .expect("capability");
        sqlx::query(
            "INSERT INTO provider_model_capabilities \
             (provider_id, model, task, traits, protocol, connection_role, provider_params, created_at, updated_at) \
             VALUES (?, 'image-model', 'chat', '[]', 'openai.chat_text', 'default', '{}', 0, 0)",
        )
        .bind(&provider_id)
        .execute(database.pool())
        .await
        .expect("chat capability");
        provider_id
    }

    fn definition(provider_id: &str, review_required: bool) -> CreativeTemplateDefinitionV1 {
        let template_id = generate_id();
        let variable_id = generate_id();
        let prompt_id = generate_id();
        let render_id = generate_id();
        let draft_id = generate_id();
        let generate_step_id = generate_id();
        CreativeTemplateDefinitionV1 {
            id: template_id,
            revision: 1,
            metadata: CreativeTemplateMetadata {
                name: "Wave3 Template".into(),
                description: String::new(),
                category: String::new(),
                visibility: CreativeTemplateVisibility::Private,
                tags: Vec::new(),
                created_at: 0,
                updated_at: 0,
            },
            output: if review_required {
                CreativeTemplateOutputPlan::MultiImageSeries {
                    target_count: 2,
                    concurrency: 1,
                    review_required: true,
                }
            } else {
                CreativeTemplateOutputPlan::SingleImage
            },
            variables: vec![CreativeTemplateVariable::Text {
                id: variable_id.clone(),
                key: "subject".into(),
                label: "Subject".into(),
                description: String::new(),
                required: true,
                default_value: None,
                placeholder: String::new(),
                min_length: 1,
                max_length: 100,
            }],
            templates: vec![CreativePromptTemplate {
                id: prompt_id.clone(),
                name: "Prompt".into(),
                segments: vec![CreativePromptTemplateSegment::Variable {
                    variable_id: variable_id.clone(),
                }],
            }],
            steps: if review_required {
                vec![
                    CreativeTemplateStep::DraftPrompts {
                        id: draft_id.clone(),
                        name: "Plan".into(),
                        depends_on: Vec::new(),
                        enabled: true,
                        template_id: prompt_id,
                        planning: CreativeTemplatePromptPlanningSettings {
                            model: Some(CreativeTemplateTextModelBinding {
                                provider_id: provider_id.into(),
                                model: "image-model".into(),
                                task: CreativeTemplateTextTask::Chat,
                            }),
                            instruction: "Keep the series coherent".into(),
                            max_tokens: 4096,
                        },
                    },
                    CreativeTemplateStep::GenerateImages {
                        id: generate_step_id,
                        name: "Generate".into(),
                        depends_on: vec![draft_id.clone()],
                        enabled: true,
                        prompt_source: CreativeTemplatePromptSource::PromptDrafts {
                            step_id: draft_id,
                        },
                        reference_variable_ids: Vec::new(),
                        generation: CreativeTemplateImageGenerationSettings {
                            model: Some(CreativeTemplateImageModelBinding {
                                provider_id: provider_id.into(),
                                model: "image-model".into(),
                                task: CreativeTemplateImageTask::ImageGeneration,
                            }),
                            quality: CreativeTemplateImageQuality::Auto,
                            width: 1024,
                            height: 1024,
                            images_per_prompt: 1,
                        },
                    },
                ]
            } else {
                vec![
                    CreativeTemplateStep::RenderTemplate {
                        id: render_id.clone(),
                        name: "Render".into(),
                        depends_on: Vec::new(),
                        enabled: true,
                        template_id: prompt_id.clone(),
                    },
                    CreativeTemplateStep::GenerateImages {
                        id: generate_step_id,
                        name: "Generate".into(),
                        depends_on: vec![render_id],
                        enabled: true,
                        prompt_source: CreativeTemplatePromptSource::Template {
                            template_id: prompt_id,
                        },
                        reference_variable_ids: Vec::new(),
                        generation: CreativeTemplateImageGenerationSettings {
                            model: Some(CreativeTemplateImageModelBinding {
                                provider_id: provider_id.into(),
                                model: "image-model".into(),
                                task: CreativeTemplateImageTask::ImageGeneration,
                            }),
                            quality: CreativeTemplateImageQuality::Auto,
                            width: 1024,
                            height: 1024,
                            images_per_prompt: 1,
                        },
                    },
                ]
            },
        }
    }

    fn run_request(definition: &CreativeTemplateDefinitionV1) -> CreativeTemplateRunCreateRequest {
        CreativeTemplateRunCreateRequest {
            template_run_id: generate_id(),
            template_id: definition.id.clone(),
            template_revision: definition.revision,
            inputs: vec![CreativeTemplateInputValue::Text {
                variable_id: match &definition.variables[0] {
                    CreativeTemplateVariable::Text { id, .. } => id.clone(),
                    _ => unreachable!(),
                },
                value: "durable subject".into(),
            }],
            reference_asset_ids: Vec::new(),
        }
    }

    async fn runner(
        owner_id: &UserId,
    ) -> (
        nomifun_db::Database,
        Arc<WorkshopService>,
        NomiWave3TemplateRunner,
        tempfile::TempDir,
    ) {
        let database = init_database_memory_with_owner(owner_id.clone())
            .await
            .expect("database");
        let root = tempfile::tempdir().expect("root");
        let workshop = WorkshopService::start(
            root.path(),
            Arc::new(SqliteWorkshopRepository::new(database.pool().clone())),
        );
        let creation = CreationService::new(Arc::new(SqliteCreationTaskRepository::new(
            database.pool().clone(),
        )));
        let runner = NomiWave3TemplateRunner::new(Arc::clone(&workshop), creation);
        (database, workshop, runner, root)
    }

    #[tokio::test]
    async fn review_required_template_is_rejected_before_a_run_is_written() {
        let owner_id = UserId::new();
        let (database, workshop, runner, _root) = runner(&owner_id).await;
        let provider_id = seed_provider(&database).await;
        let definition = workshop
            .create_creative_template(definition(&provider_id, true))
            .await
            .expect("template");
        let error = runner
            .run(run_request(&definition))
            .await
            .expect_err("review required");
        assert_eq!(error.code, HUMAN_REVIEW_REQUIRED);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM creative_studio_template_runs")
            .fetch_one(database.pool())
            .await
            .expect("run count");
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn execution_failure_is_terminal_and_idempotently_replayed() {
        let owner_id = UserId::new();
        let (database, workshop, runner, _root) = runner(&owner_id).await;
        let provider_id = seed_provider(&database).await;
        let definition = workshop
            .create_creative_template(definition(&provider_id, false))
            .await
            .expect("template");
        let request = run_request(&definition);
        let run_id = request.template_run_id.clone();
        let (first, replay) = tokio::join!(runner.run(request.clone()), runner.run(request));
        let first = first.expect_err("terminal failure must be a non-success Tool result");
        let replay = replay.expect_err("concurrent failed replay must remain non-success");
        assert_eq!(first.code, TEMPLATE_RUN_FAILED);
        assert_eq!(replay.code, first.code);
        let persisted = workshop
            .get_creative_template_run(&run_id)
            .await
            .expect("persisted terminal run");
        assert_eq!(persisted.record.status, CreativeTemplateRunStatus::Failed);
        assert!(persisted.record.failure.is_some());
        let run_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM creative_studio_template_runs")
                .fetch_one(database.pool())
                .await
                .expect("run count");
        let task_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM creation_tasks")
            .fetch_one(database.pool())
            .await
            .expect("task count");
        assert_eq!((run_count, task_count), (1, 1));
    }
}
