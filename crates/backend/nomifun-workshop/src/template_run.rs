//! Durable Creative Studio Template run aggregate v1.
//!
//! Definitions remain editable catalog objects. A run pins one exact validated
//! definition, input set, task plan, review projection, and terminal result so
//! refresh/restart recovery never has to reinterpret a newer template revision.

use std::collections::BTreeSet;

use nomifun_common::validate_uuidv7;
use nomifun_db::CreativeStudioTemplateRunRow;
use serde::{Deserialize, Serialize};

use crate::template::{
    CreativeTemplateDefinitionV1, CreativeTemplateOutputPlan, CreativeTemplateStep,
    CreativeTemplateVariable,
};

pub const TEMPLATE_RUN_KIND: &str = "nomifun.creative-studio.template-run";
pub const MAX_TEMPLATE_RUN_AGGREGATE_BYTES: usize = 16 * 1024 * 1024;
const MAX_RUN_INPUTS: usize = 100;
const MAX_RUN_REFERENCES: usize = 100;
const MAX_RUN_STEPS: usize = 128;
const MAX_RUN_TASKS: usize = 1_000;
const MAX_RUN_TEXT: usize = 20_000;
const MAX_RUN_PROMPT: usize = 200_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CreativeTemplateInputValue {
    Text { variable_id: String, value: String },
    MultilineText { variable_id: String, value: String },
    Number { variable_id: String, value: f64 },
    Boolean { variable_id: String, value: bool },
    Choice { variable_id: String, value: String },
    Image { variable_id: String, asset_id: Option<String> },
    ImageSeries { variable_id: String, asset_ids: Vec<String> },
}

/// Client-owned immutable identity and inputs for idempotently creating a run.
/// Server time and the pinned template snapshot are deliberately excluded.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeTemplateRunCreateRequest {
    pub template_run_id: String,
    pub template_id: String,
    pub template_revision: i64,
    pub inputs: Vec<CreativeTemplateInputValue>,
    pub reference_asset_ids: Vec<String>,
}

impl CreativeTemplateInputValue {
    fn variable_id(&self) -> &str {
        match self {
            Self::Text { variable_id, .. }
            | Self::MultilineText { variable_id, .. }
            | Self::Number { variable_id, .. }
            | Self::Boolean { variable_id, .. }
            | Self::Choice { variable_id, .. }
            | Self::Image { variable_id, .. }
            | Self::ImageSeries { variable_id, .. } => variable_id,
        }
    }

    fn append_asset_ids<'a>(&'a self, target: &mut Vec<&'a str>) {
        match self {
            Self::Image { asset_id: Some(asset_id), .. } => target.push(asset_id),
            Self::ImageSeries { asset_ids, .. } => {
                target.extend(asset_ids.iter().map(String::as_str));
            }
            _ => {}
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeTemplateRunRequest {
    pub id: String,
    pub idempotency_key: String,
    pub template_id: String,
    pub template_revision: i64,
    pub requested_at: i64,
    pub output: CreativeTemplateOutputPlan,
    pub inputs: Vec<CreativeTemplateInputValue>,
    pub reference_asset_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CreativeTemplatePromptDraftStatus {
    PendingReview,
    Approved,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeTemplatePromptDraft {
    pub id: String,
    pub template_id: String,
    pub run_request_id: String,
    pub series_index: usize,
    pub title: String,
    pub prompt: String,
    pub status: CreativeTemplatePromptDraftStatus,
    pub created_at: i64,
    pub reviewed_at: Option<i64>,
    pub review_note: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CreativeTemplateRunStatus {
    Requested,
    AwaitingReview,
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl CreativeTemplateRunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::AwaitingReview => "awaiting-review",
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeTemplateRunFailure {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeTemplateRunRecord {
    pub request_id: String,
    pub template_id: String,
    pub status: CreativeTemplateRunStatus,
    pub prompt_draft_ids: Vec<String>,
    pub task_ids: Vec<String>,
    pub result_asset_ids: Vec<String>,
    pub history_reference_ids: Vec<String>,
    pub queued_at: Option<i64>,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub failure: Option<CreativeTemplateRunFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeTemplateRunAggregateV1 {
    pub kind: String,
    pub version: i64,
    pub revision: i64,
    pub template_snapshot: CreativeTemplateDefinitionV1,
    pub request: CreativeTemplateRunRequest,
    pub prompt_drafts: Vec<CreativeTemplatePromptDraft>,
    pub record: CreativeTemplateRunRecord,
}

impl CreativeTemplateRunAggregateV1 {
    pub fn requested(
        template_snapshot: CreativeTemplateDefinitionV1,
        template_run_id: String,
        inputs: Vec<CreativeTemplateInputValue>,
        reference_asset_ids: Vec<String>,
        requested_at: i64,
    ) -> Result<Self, String> {
        let aggregate = Self {
            kind: TEMPLATE_RUN_KIND.into(),
            version: 1,
            revision: 1,
            request: CreativeTemplateRunRequest {
                id: template_run_id.clone(),
                idempotency_key: template_run_id.clone(),
                template_id: template_snapshot.id.clone(),
                template_revision: template_snapshot.revision,
                requested_at,
                output: template_snapshot.output.clone(),
                inputs,
                reference_asset_ids,
            },
            prompt_drafts: Vec::new(),
            record: CreativeTemplateRunRecord {
                request_id: template_run_id,
                template_id: template_snapshot.id.clone(),
                status: CreativeTemplateRunStatus::Requested,
                prompt_draft_ids: Vec::new(),
                task_ids: Vec::new(),
                result_asset_ids: Vec::new(),
                history_reference_ids: Vec::new(),
                queued_at: None,
                started_at: None,
                completed_at: None,
                failure: None,
            },
            template_snapshot,
        };
        aggregate.validate()?;
        Ok(aggregate)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.kind != TEMPLATE_RUN_KIND || self.version != 1 {
            return Err("template run envelope kind/version is unsupported".into());
        }
        if self.revision < 1 {
            return Err("template run revision must be positive".into());
        }
        self.template_snapshot.validate()?;
        validate_id("request.id", &self.request.id)?;
        validate_id("request.idempotencyKey", &self.request.idempotency_key)?;
        if self.request.idempotency_key != self.request.id {
            return Err("template run idempotencyKey must equal its durable run id".into());
        }
        if self.request.template_id != self.template_snapshot.id
            || self.request.template_revision != self.template_snapshot.revision
            || self.request.output != self.template_snapshot.output
        {
            return Err("template run request does not match its pinned definition".into());
        }
        if self.request.requested_at < 0 {
            return Err("template run requestedAt must be non-negative".into());
        }
        validate_inputs(&self.template_snapshot, &self.request.inputs)?;
        validate_unique_ids(
            "request.referenceAssetIds",
            &self.request.reference_asset_ids,
            MAX_RUN_REFERENCES,
        )?;
        validate_run_models(&self.template_snapshot)?;
        let executable_steps = self.executable_task_step_ids();
        if executable_steps.is_empty() || executable_steps.len() > MAX_RUN_STEPS {
            return Err("template run executable step count is outside 1..=128".into());
        }

        let expected_draft_count = match self.template_snapshot.output {
            CreativeTemplateOutputPlan::SingleImage => 0,
            CreativeTemplateOutputPlan::MultiImageSeries { target_count, .. } => target_count,
        };
        validate_prompt_drafts(self, expected_draft_count)?;
        validate_record(self, expected_draft_count)?;
        Ok(())
    }

    pub fn executable_task_step_ids(&self) -> Vec<String> {
        self.template_snapshot
            .steps
            .iter()
            .filter_map(|step| match step {
                CreativeTemplateStep::DraftPrompts { id, enabled: true, .. }
                | CreativeTemplateStep::GenerateImages { id, enabled: true, .. } => {
                    Some(id.clone())
                }
                _ => None,
            })
            .collect()
    }

    pub fn expected_task_count(&self) -> usize {
        expected_task_count(&self.template_snapshot)
    }

    pub fn expected_result_asset_count(&self) -> usize {
        expected_result_asset_count(&self.template_snapshot)
    }

    pub fn referenced_input_asset_ids(&self) -> Vec<&str> {
        let mut ids = Vec::new();
        for input in &self.request.inputs {
            input.append_asset_ids(&mut ids);
        }
        ids.extend(self.request.reference_asset_ids.iter().map(String::as_str));
        ids
    }

    pub fn matches_create_request(&self, request: &CreativeTemplateRunCreateRequest) -> bool {
        self.request.id == request.template_run_id
            && self.request.idempotency_key == request.template_run_id
            && self.request.template_id == request.template_id
            && self.request.template_revision == request.template_revision
            && self.request.inputs == request.inputs
            && self.request.reference_asset_ids == request.reference_asset_ids
    }

    pub fn validate_transition(&self, next: &Self) -> Result<(), String> {
        self.validate()?;
        next.validate()?;
        if self.record.status.is_terminal() {
            return Err("terminal template runs are immutable".into());
        }
        if next.revision != self.revision + 1 {
            return Err("template run revision must increment exactly once".into());
        }
        if next.template_snapshot != self.template_snapshot || next.request != self.request {
            return Err("template run definition and request snapshots are immutable".into());
        }
        let allowed = matches!(
            (self.record.status, next.record.status),
            (CreativeTemplateRunStatus::Requested, CreativeTemplateRunStatus::Queued)
                | (CreativeTemplateRunStatus::Requested, CreativeTemplateRunStatus::Failed)
                | (CreativeTemplateRunStatus::Requested, CreativeTemplateRunStatus::Cancelled)
                | (CreativeTemplateRunStatus::Queued, CreativeTemplateRunStatus::Queued)
                | (CreativeTemplateRunStatus::Queued, CreativeTemplateRunStatus::Running)
                | (CreativeTemplateRunStatus::Queued, CreativeTemplateRunStatus::Failed)
                | (CreativeTemplateRunStatus::Queued, CreativeTemplateRunStatus::Cancelled)
                | (CreativeTemplateRunStatus::Running, CreativeTemplateRunStatus::Running)
                | (CreativeTemplateRunStatus::Running, CreativeTemplateRunStatus::AwaitingReview)
                | (CreativeTemplateRunStatus::Running, CreativeTemplateRunStatus::Succeeded)
                | (CreativeTemplateRunStatus::Running, CreativeTemplateRunStatus::Failed)
                | (CreativeTemplateRunStatus::Running, CreativeTemplateRunStatus::Cancelled)
                | (CreativeTemplateRunStatus::AwaitingReview, CreativeTemplateRunStatus::AwaitingReview)
                | (CreativeTemplateRunStatus::AwaitingReview, CreativeTemplateRunStatus::Running)
                | (CreativeTemplateRunStatus::AwaitingReview, CreativeTemplateRunStatus::Failed)
                | (CreativeTemplateRunStatus::AwaitingReview, CreativeTemplateRunStatus::Cancelled)
        );
        if !allowed {
            return Err(format!(
                "invalid template run transition {} -> {}",
                self.record.status.as_str(),
                next.record.status.as_str()
            ));
        }
        validate_prefix("record.taskIds", &self.record.task_ids, &next.record.task_ids)?;
        validate_prefix(
            "record.resultAssetIds",
            &self.record.result_asset_ids,
            &next.record.result_asset_ids,
        )?;
        validate_prefix(
            "record.historyReferenceIds",
            &self.record.history_reference_ids,
            &next.record.history_reference_ids,
        )?;
        for (field, current, replacement) in [
            ("queuedAt", self.record.queued_at, next.record.queued_at),
            ("startedAt", self.record.started_at, next.record.started_at),
            ("completedAt", self.record.completed_at, next.record.completed_at),
        ] {
            if current.is_some() && current != replacement {
                return Err(format!("template run {field} is immutable once set"));
            }
        }
        validate_draft_transition(self, next)?;
        Ok(())
    }

    pub fn to_row(&self, created_at: i64, updated_at: i64) -> Result<CreativeStudioTemplateRunRow, String> {
        self.validate()?;
        if created_at != self.request.requested_at || updated_at < created_at {
            return Err("template run row timestamps do not match its request".into());
        }
        let aggregate_json = serde_json::to_string(self)
            .map_err(|error| format!("failed to serialize template run: {error}"))?;
        if aggregate_json.len() > MAX_TEMPLATE_RUN_AGGREGATE_BYTES {
            return Err("template run aggregate exceeds the 16 MiB limit".into());
        }
        let step_ids_json = serde_json::to_string(&self.executable_task_step_ids())
            .map_err(|error| format!("failed to serialize template run step ids: {error}"))?;
        Ok(CreativeStudioTemplateRunRow {
            id: 0,
            template_run_id: self.request.id.clone(),
            template_id: self.request.template_id.clone(),
            template_revision: self.request.template_revision,
            revision: self.revision,
            status: self.record.status.as_str().into(),
            step_ids_json,
            aggregate_json,
            created_at,
            updated_at,
        })
    }
}

pub fn parse_template_run_row(
    row: &CreativeStudioTemplateRunRow,
) -> Result<CreativeTemplateRunAggregateV1, String> {
    if row.aggregate_json.len() > MAX_TEMPLATE_RUN_AGGREGATE_BYTES {
        return Err("stored template run aggregate exceeds the 16 MiB limit".into());
    }
    let aggregate: CreativeTemplateRunAggregateV1 = serde_json::from_str(&row.aggregate_json)
        .map_err(|error| format!("stored template run JSON is invalid: {error}"))?;
    aggregate.validate()?;
    let step_ids: Vec<String> = serde_json::from_str(&row.step_ids_json)
        .map_err(|error| format!("stored template run step IDs are invalid: {error}"))?;
    if aggregate.request.id != row.template_run_id
        || aggregate.request.template_id != row.template_id
        || aggregate.request.template_revision != row.template_revision
        || aggregate.revision != row.revision
        || aggregate.record.status.as_str() != row.status
        || aggregate.executable_task_step_ids() != step_ids
        || aggregate.request.requested_at != row.created_at
        || row.updated_at < row.created_at
    {
        return Err("stored template run row metadata does not match its aggregate".into());
    }
    Ok(aggregate)
}

fn validate_inputs(
    template: &CreativeTemplateDefinitionV1,
    inputs: &[CreativeTemplateInputValue],
) -> Result<(), String> {
    if inputs.len() > MAX_RUN_INPUTS || inputs.len() > template.variables.len() {
        return Err("template run input count exceeds its definition".into());
    }
    let mut seen = BTreeSet::new();
    for input in inputs {
        validate_id("request.inputs[].variableId", input.variable_id())?;
        if !seen.insert(input.variable_id()) {
            return Err("template run input variable IDs must be unique".into());
        }
        let variable = template
            .variables
            .iter()
            .find(|variable| variable_id(variable) == input.variable_id())
            .ok_or_else(|| "template run input references a missing variable".to_string())?;
        validate_input(variable, input)?;
    }
    for variable in &template.variables {
        if variable_required(variable)
            && inputs
                .iter()
                .find(|input| input.variable_id() == variable_id(variable))
                .is_none_or(input_is_absent)
        {
            return Err(format!(
                "required template input {} is missing",
                variable_id(variable)
            ));
        }
    }
    Ok(())
}

fn validate_input(
    variable: &CreativeTemplateVariable,
    input: &CreativeTemplateInputValue,
) -> Result<(), String> {
    match (variable, input) {
        (
            CreativeTemplateVariable::Text { min_length, max_length, .. },
            CreativeTemplateInputValue::Text { value, .. },
        )
        | (
            CreativeTemplateVariable::MultilineText { min_length, max_length, .. },
            CreativeTemplateInputValue::MultilineText { value, .. },
        ) => {
            validate_text("request.inputs[].value", value, MAX_RUN_TEXT, true)?;
            let length = value.encode_utf16().count();
            if length < *min_length || length > *max_length {
                return Err("template run text input is outside its bounds".into());
            }
        }
        (
            CreativeTemplateVariable::Number { minimum, maximum, .. },
            CreativeTemplateInputValue::Number { value, .. },
        ) => {
            if !value.is_finite()
                || minimum.is_some_and(|minimum| *value < minimum)
                || maximum.is_some_and(|maximum| *value > maximum)
            {
                return Err("template run number input is outside its bounds".into());
            }
        }
        (
            CreativeTemplateVariable::Boolean { .. },
            CreativeTemplateInputValue::Boolean { .. },
        ) => {}
        (
            CreativeTemplateVariable::Choice { options, .. },
            CreativeTemplateInputValue::Choice { value, .. },
        ) if options.contains(value) => {}
        (
            CreativeTemplateVariable::Image { .. },
            CreativeTemplateInputValue::Image { asset_id, .. },
        ) => {
            if let Some(asset_id) = asset_id {
                validate_id("request.inputs[].assetId", asset_id)?;
            }
        }
        (
            CreativeTemplateVariable::ImageSeries { min_items, max_items, .. },
            CreativeTemplateInputValue::ImageSeries { asset_ids, .. },
        ) => {
            validate_unique_ids("request.inputs[].assetIds", asset_ids, MAX_RUN_REFERENCES)?;
            if asset_ids.len() < *min_items || asset_ids.len() > *max_items {
                return Err("template run image-series input is outside its bounds".into());
            }
        }
        _ => return Err("template run input type does not match its variable".into()),
    }
    Ok(())
}

fn validate_run_models(template: &CreativeTemplateDefinitionV1) -> Result<(), String> {
    let enabled_planners = template
        .steps
        .iter()
        .filter(|step| matches!(step, CreativeTemplateStep::DraftPrompts { enabled: true, .. }))
        .collect::<Vec<_>>();
    match template.output {
        CreativeTemplateOutputPlan::SingleImage if !enabled_planners.is_empty() => {
            return Err("single-image template cannot execute a prompt planner".into());
        }
        CreativeTemplateOutputPlan::MultiImageSeries { .. } if enabled_planners.len() != 1 => {
            return Err("multi-image template requires exactly one enabled prompt planner".into());
        }
        _ => {}
    }
    for step in &template.steps {
        match step {
            CreativeTemplateStep::DraftPrompts {
                enabled: true,
                planning,
                ..
            } if planning.model.is_none() => {
                return Err("enabled prompt-planning step has no Chat model binding".into());
            }
            CreativeTemplateStep::GenerateImages {
                enabled: true,
                generation,
                ..
            } if generation.model.is_none() => {
                return Err("enabled image-generation step has no model binding".into());
            }
            _ => {}
        }
    }
    if !template
        .steps
        .iter()
        .any(|step| matches!(step, CreativeTemplateStep::GenerateImages { enabled: true, .. }))
    {
        return Err("template run requires an enabled image-generation step".into());
    }
    Ok(())
}

fn validate_prompt_drafts(
    aggregate: &CreativeTemplateRunAggregateV1,
    expected_count: usize,
) -> Result<(), String> {
    if aggregate.prompt_drafts.len() > expected_count {
        return Err("template run contains too many prompt drafts".into());
    }
    if expected_count == 0 && !aggregate.prompt_drafts.is_empty() {
        return Err("single-image template run cannot contain prompt drafts".into());
    }
    let mut ids = BTreeSet::new();
    let mut indexes = BTreeSet::new();
    for draft in &aggregate.prompt_drafts {
        validate_id("promptDrafts[].id", &draft.id)?;
        if !ids.insert(draft.id.as_str()) || !indexes.insert(draft.series_index) {
            return Err("template run prompt draft IDs and indexes must be unique".into());
        }
        if draft.template_id != aggregate.request.template_id
            || draft.run_request_id != aggregate.request.id
            || draft.series_index >= expected_count
            || draft.created_at < aggregate.request.requested_at
        {
            return Err("template run prompt draft ownership/timestamps are invalid".into());
        }
        validate_text("promptDrafts[].title", &draft.title, 120, false)?;
        validate_text("promptDrafts[].prompt", &draft.prompt, MAX_RUN_PROMPT, false)?;
        if let Some(note) = draft.review_note.as_deref() {
            validate_text("promptDrafts[].reviewNote", note, 2_000, true)?;
        }
        match draft.status {
            CreativeTemplatePromptDraftStatus::PendingReview
                if draft.reviewed_at.is_some() || draft.review_note.is_some() =>
            {
                return Err("pending prompt draft cannot contain review data".into());
            }
            CreativeTemplatePromptDraftStatus::Approved
            | CreativeTemplatePromptDraftStatus::Rejected => {
                let reviewed_at = draft
                    .reviewed_at
                    .ok_or_else(|| "reviewed prompt draft requires reviewedAt".to_string())?;
                if reviewed_at < draft.created_at {
                    return Err("prompt draft review cannot predate creation".into());
                }
            }
            CreativeTemplatePromptDraftStatus::PendingReview => {}
        }
    }
    Ok(())
}

fn validate_record(
    aggregate: &CreativeTemplateRunAggregateV1,
    expected_draft_count: usize,
) -> Result<(), String> {
    let record = &aggregate.record;
    if record.request_id != aggregate.request.id || record.template_id != aggregate.request.template_id {
        return Err("template run record ownership does not match its request".into());
    }
    let draft_ids = aggregate
        .prompt_drafts
        .iter()
        .map(|draft| draft.id.clone())
        .collect::<Vec<_>>();
    if record.prompt_draft_ids != draft_ids {
        return Err("template run prompt draft projection is inconsistent".into());
    }
    validate_unique_ids("record.taskIds", &record.task_ids, MAX_RUN_TASKS)?;
    validate_unique_ids("record.resultAssetIds", &record.result_asset_ids, MAX_RUN_TASKS * 6)?;
    validate_unique_ids(
        "record.historyReferenceIds",
        &record.history_reference_ids,
        MAX_RUN_TASKS * 6,
    )?;
    let expected_tasks = aggregate.expected_task_count();
    let expected_results = aggregate.expected_result_asset_count();
    if record.task_ids.len() > expected_tasks || record.result_asset_ids.len() > expected_results {
        return Err("template run task/result projection exceeds its pinned plan".into());
    }
    for timestamp in [record.queued_at, record.started_at, record.completed_at]
        .into_iter()
        .flatten()
    {
        if timestamp < aggregate.request.requested_at {
            return Err("template run timestamp predates its request".into());
        }
    }
    if record
        .queued_at
        .zip(record.started_at)
        .is_some_and(|(queued, started)| started < queued)
        || record
            .started_at
            .zip(record.completed_at)
            .is_some_and(|(started, completed)| completed < started)
    {
        return Err("template run timestamps are not monotonic".into());
    }
    let drafts_complete = aggregate.prompt_drafts.len() == expected_draft_count;
    let drafts_approved = aggregate
        .prompt_drafts
        .iter()
        .all(|draft| draft.status == CreativeTemplatePromptDraftStatus::Approved);
    match record.status {
        CreativeTemplateRunStatus::Requested => {
            if record.queued_at.is_some()
                || record.started_at.is_some()
                || record.completed_at.is_some()
                || !record.task_ids.is_empty()
                || !record.result_asset_ids.is_empty()
                || !aggregate.prompt_drafts.is_empty()
                || record.failure.is_some()
            {
                return Err("requested template run contains execution data".into());
            }
        }
        CreativeTemplateRunStatus::Queued => {
            if record.queued_at.is_none()
                || record.started_at.is_some()
                || record.completed_at.is_some()
                || record.task_ids.len() != expected_tasks
                || record.failure.is_some()
            {
                return Err("queued template run projection is incomplete".into());
            }
        }
        CreativeTemplateRunStatus::Running => {
            if record.queued_at.is_none()
                || record.started_at.is_none()
                || record.completed_at.is_some()
                || record.task_ids.len() != expected_tasks
                || record.failure.is_some()
            {
                return Err("running template run projection is incomplete".into());
            }
            if !aggregate.prompt_drafts.is_empty() && (!drafts_complete || !drafts_approved) {
                return Err("running image phase requires a complete approved prompt set".into());
            }
        }
        CreativeTemplateRunStatus::AwaitingReview => {
            if expected_draft_count == 0
                || record.queued_at.is_none()
                || record.started_at.is_none()
                || record.completed_at.is_some()
                || record.task_ids.len() != expected_tasks
                || !drafts_complete
                || record.failure.is_some()
            {
                return Err("awaiting-review template run projection is incomplete".into());
            }
        }
        CreativeTemplateRunStatus::Succeeded => {
            if record.queued_at.is_none()
                || record.started_at.is_none()
                || record.completed_at.is_none()
                || record.task_ids.len() != expected_tasks
                || record.result_asset_ids.len() != expected_results
                || (expected_draft_count > 0 && (!drafts_complete || !drafts_approved))
                || record.failure.is_some()
            {
                return Err("succeeded template run projection is incomplete".into());
            }
        }
        CreativeTemplateRunStatus::Failed => {
            if record.completed_at.is_none() || record.failure.is_none() {
                return Err("failed template run requires terminal failure data".into());
            }
        }
        CreativeTemplateRunStatus::Cancelled => {
            if record.completed_at.is_none() || record.failure.is_some() {
                return Err("cancelled template run projection is invalid".into());
            }
        }
    }
    if record.status != CreativeTemplateRunStatus::Failed && record.failure.is_some() {
        return Err("only failed template runs may carry failure data".into());
    }
    if let Some(failure) = record.failure.as_ref() {
        validate_code("record.failure.code", &failure.code)?;
        validate_text("record.failure.message", &failure.message, 2_000, false)?;
    }
    Ok(())
}

fn validate_draft_transition(
    current: &CreativeTemplateRunAggregateV1,
    next: &CreativeTemplateRunAggregateV1,
) -> Result<(), String> {
    if current.prompt_drafts.is_empty() {
        return Ok(());
    }
    if current.prompt_drafts.len() != next.prompt_drafts.len() {
        return Err("persisted prompt drafts cannot be removed or replaced".into());
    }
    for (before, after) in current.prompt_drafts.iter().zip(&next.prompt_drafts) {
        if before.id != after.id
            || before.template_id != after.template_id
            || before.run_request_id != after.run_request_id
            || before.series_index != after.series_index
            || before.created_at != after.created_at
        {
            return Err("persisted prompt draft identity is immutable".into());
        }
        if current.record.status != CreativeTemplateRunStatus::AwaitingReview && before != after {
            return Err("prompt drafts can only be edited while awaiting review".into());
        }
    }
    Ok(())
}

fn expected_task_count(template: &CreativeTemplateDefinitionV1) -> usize {
    let series_count = match template.output {
        CreativeTemplateOutputPlan::SingleImage => 1,
        CreativeTemplateOutputPlan::MultiImageSeries { target_count, .. } => target_count,
    };
    template
        .steps
        .iter()
        .map(|step| match step {
            CreativeTemplateStep::DraftPrompts { enabled: true, .. } => 1,
            CreativeTemplateStep::GenerateImages {
                enabled: true,
                prompt_source: crate::template::CreativeTemplatePromptSource::PromptDrafts { .. },
                ..
            } => series_count,
            CreativeTemplateStep::GenerateImages { enabled: true, .. } => 1,
            _ => 0,
        })
        .sum()
}

fn expected_result_asset_count(template: &CreativeTemplateDefinitionV1) -> usize {
    let series_count = match template.output {
        CreativeTemplateOutputPlan::SingleImage => 1,
        CreativeTemplateOutputPlan::MultiImageSeries { target_count, .. } => target_count,
    };
    template
        .steps
        .iter()
        .map(|step| match step {
            CreativeTemplateStep::GenerateImages {
                enabled: true,
                prompt_source: crate::template::CreativeTemplatePromptSource::PromptDrafts { .. },
                generation,
                ..
            } => series_count * generation.images_per_prompt,
            CreativeTemplateStep::GenerateImages {
                enabled: true,
                generation,
                ..
            } => generation.images_per_prompt,
            _ => 0,
        })
        .sum()
}

fn variable_id(variable: &CreativeTemplateVariable) -> &str {
    match variable {
        CreativeTemplateVariable::Text { id, .. }
        | CreativeTemplateVariable::MultilineText { id, .. }
        | CreativeTemplateVariable::Number { id, .. }
        | CreativeTemplateVariable::Boolean { id, .. }
        | CreativeTemplateVariable::Choice { id, .. }
        | CreativeTemplateVariable::Image { id, .. }
        | CreativeTemplateVariable::ImageSeries { id, .. } => id,
    }
}

fn variable_required(variable: &CreativeTemplateVariable) -> bool {
    match variable {
        CreativeTemplateVariable::Text { required, .. }
        | CreativeTemplateVariable::MultilineText { required, .. }
        | CreativeTemplateVariable::Number { required, .. }
        | CreativeTemplateVariable::Boolean { required, .. }
        | CreativeTemplateVariable::Choice { required, .. }
        | CreativeTemplateVariable::Image { required, .. }
        | CreativeTemplateVariable::ImageSeries { required, .. } => *required,
    }
}

fn input_is_absent(input: &CreativeTemplateInputValue) -> bool {
    match input {
        CreativeTemplateInputValue::Text { value, .. }
        | CreativeTemplateInputValue::MultilineText { value, .. }
        | CreativeTemplateInputValue::Choice { value, .. } => value.trim().is_empty(),
        CreativeTemplateInputValue::Image { asset_id, .. } => asset_id.is_none(),
        CreativeTemplateInputValue::ImageSeries { asset_ids, .. } => asset_ids.is_empty(),
        CreativeTemplateInputValue::Number { .. } | CreativeTemplateInputValue::Boolean { .. } => {
            false
        }
    }
}

fn validate_prefix(path: &str, current: &[String], next: &[String]) -> Result<(), String> {
    if next.len() < current.len() || next[..current.len()] != *current {
        return Err(format!("template run {path} may only append canonical IDs"));
    }
    Ok(())
}

fn validate_unique_ids(path: &str, values: &[String], maximum: usize) -> Result<(), String> {
    if values.len() > maximum {
        return Err(format!("{path} exceeds its item limit"));
    }
    let mut unique = BTreeSet::new();
    for value in values {
        validate_id(path, value)?;
        if !unique.insert(value.as_str()) {
            return Err(format!("{path} contains duplicate IDs"));
        }
    }
    Ok(())
}

fn validate_id(path: &str, value: &str) -> Result<(), String> {
    validate_uuidv7(value)
        .map(|_| ())
        .map_err(|error| format!("{path} must be a canonical UUIDv7: {error}"))
}

fn validate_text(
    path: &str,
    value: &str,
    maximum: usize,
    allow_empty: bool,
) -> Result<(), String> {
    let has_control = value.chars().any(|value| {
        matches!(value as u32, 0x00..=0x08 | 0x0b | 0x0c | 0x0e..=0x1f | 0x7f)
    });
    if value.encode_utf16().count() > maximum
        || has_control
        || (!allow_empty && value.trim().is_empty())
    {
        return Err(format!("{path} must contain bounded text"));
    }
    Ok(())
}

fn validate_code(path: &str, value: &str) -> Result<(), String> {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return Err(format!("{path} is invalid"));
    };
    if !first.is_ascii_lowercase()
        || value.len() > 80
        || !characters.all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '.' | '_' | '-')
        })
    {
        return Err(format!("{path} is invalid"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::{
        CreativeTemplateImageGenerationSettings, CreativeTemplateImageModelBinding,
        CreativeTemplateImageQuality, CreativeTemplateImageTask, CreativeTemplateMetadata,
        CreativeTemplatePromptPlanningSettings, CreativeTemplatePromptSource,
        CreativePromptTemplate, CreativePromptTemplateSegment,
        CreativeTemplateTextModelBinding, CreativeTemplateTextTask, CreativeTemplateVisibility,
    };

    const DEFINITION_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000301";
    const VARIABLE: &str = "0190f5fe-7c00-7a00-8abc-000000000302";
    const PROMPT_TEMPLATE_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000303";
    const GENERATE: &str = "0190f5fe-7c00-7a00-8abc-000000000304";
    const PROVIDER: &str = "0190f5fe-7c00-7a00-8abc-000000000305";
    const RUN: &str = "0190f5fe-7c00-7a00-8abc-000000000306";
    const TASK: &str = "0190f5fe-7c00-7a00-8abc-000000000307";
    const ASSET: &str = "0190f5fe-7c00-7a00-8abc-000000000308";
    const DRAFT_STEP: &str = "0190f5fe-7c00-7a00-8abc-000000000309";
    const TASK_2: &str = "0190f5fe-7c00-7a00-8abc-000000000310";
    const TASK_3: &str = "0190f5fe-7c00-7a00-8abc-000000000311";
    const DRAFT_1: &str = "0190f5fe-7c00-7a00-8abc-000000000312";
    const DRAFT_2: &str = "0190f5fe-7c00-7a00-8abc-000000000313";

    fn definition() -> CreativeTemplateDefinitionV1 {
        CreativeTemplateDefinitionV1 {
            id: DEFINITION_ID.into(),
            revision: 1,
            metadata: CreativeTemplateMetadata {
                name: "海报".into(),
                description: String::new(),
                category: String::new(),
                visibility: CreativeTemplateVisibility::Private,
                tags: Vec::new(),
                created_at: 100,
                updated_at: 100,
            },
            output: CreativeTemplateOutputPlan::SingleImage,
            variables: vec![CreativeTemplateVariable::Text {
                id: VARIABLE.into(),
                key: "subject".into(),
                label: "主题".into(),
                description: String::new(),
                required: true,
                default_value: None,
                placeholder: String::new(),
                min_length: 1,
                max_length: 80,
            }],
            templates: vec![CreativePromptTemplate {
                id: PROMPT_TEMPLATE_ID.into(),
                name: "提示词".into(),
                segments: vec![CreativePromptTemplateSegment::Variable {
                    variable_id: VARIABLE.into(),
                }],
            }],
            steps: vec![CreativeTemplateStep::GenerateImages {
                id: GENERATE.into(),
                name: "生成".into(),
                depends_on: Vec::new(),
                enabled: true,
                prompt_source: CreativeTemplatePromptSource::Template {
                    template_id: PROMPT_TEMPLATE_ID.into(),
                },
                reference_variable_ids: Vec::new(),
                generation: CreativeTemplateImageGenerationSettings {
                    model: Some(CreativeTemplateImageModelBinding {
                        provider_id: PROVIDER.into(),
                        model: "image-model".into(),
                        task: CreativeTemplateImageTask::ImageGeneration,
                    }),
                    quality: CreativeTemplateImageQuality::Auto,
                    width: 1024,
                    height: 1024,
                    images_per_prompt: 1,
                },
            }],
        }
    }

    fn requested() -> CreativeTemplateRunAggregateV1 {
        CreativeTemplateRunAggregateV1::requested(
            definition(),
            RUN.into(),
            vec![CreativeTemplateInputValue::Text {
                variable_id: VARIABLE.into(),
                value: "NomiFun".into(),
            }],
            Vec::new(),
            1_000,
        )
        .unwrap()
    }

    fn requested_series() -> CreativeTemplateRunAggregateV1 {
        let mut definition = definition();
        definition.output = CreativeTemplateOutputPlan::MultiImageSeries {
            target_count: 2,
            concurrency: 2,
            review_required: true,
        };
        definition.steps = vec![
            CreativeTemplateStep::DraftPrompts {
                id: DRAFT_STEP.into(),
                name: "规划".into(),
                depends_on: Vec::new(),
                enabled: true,
                template_id: PROMPT_TEMPLATE_ID.into(),
                planning: CreativeTemplatePromptPlanningSettings {
                    model: Some(CreativeTemplateTextModelBinding {
                        provider_id: PROVIDER.into(),
                        model: "chat-model".into(),
                        task: CreativeTemplateTextTask::Chat,
                    }),
                    instruction: "保持系列一致".into(),
                    max_tokens: 4096,
                },
            },
            CreativeTemplateStep::GenerateImages {
                id: GENERATE.into(),
                name: "生成".into(),
                depends_on: vec![DRAFT_STEP.into()],
                enabled: true,
                prompt_source: CreativeTemplatePromptSource::PromptDrafts {
                    step_id: DRAFT_STEP.into(),
                },
                reference_variable_ids: Vec::new(),
                generation: CreativeTemplateImageGenerationSettings {
                    model: Some(CreativeTemplateImageModelBinding {
                        provider_id: PROVIDER.into(),
                        model: "image-model".into(),
                        task: CreativeTemplateImageTask::ImageGeneration,
                    }),
                    quality: CreativeTemplateImageQuality::Auto,
                    width: 1024,
                    height: 1024,
                    images_per_prompt: 1,
                },
            },
        ];
        CreativeTemplateRunAggregateV1::requested(
            definition,
            RUN.into(),
            vec![CreativeTemplateInputValue::Text {
                variable_id: VARIABLE.into(),
                value: "NomiFun".into(),
            }],
            Vec::new(),
            1_000,
        )
        .unwrap()
    }

    #[test]
    fn requested_run_round_trips_with_exact_row_metadata() {
        let aggregate = requested();
        assert_eq!(aggregate.expected_task_count(), 1);
        assert_eq!(aggregate.expected_result_asset_count(), 1);
        let row = aggregate.to_row(1_000, 1_000).unwrap();
        assert_eq!(parse_template_run_row(&row).unwrap(), aggregate);
    }

    #[test]
    fn transition_is_cas_monotonic_and_terminal() {
        let requested = requested();
        let mut queued = requested.clone();
        queued.revision = 2;
        queued.record.status = CreativeTemplateRunStatus::Queued;
        queued.record.task_ids = vec![TASK.into()];
        queued.record.queued_at = Some(1_001);
        requested.validate_transition(&queued).unwrap();

        let mut running = queued.clone();
        running.revision = 3;
        running.record.status = CreativeTemplateRunStatus::Running;
        running.record.started_at = Some(1_002);
        queued.validate_transition(&running).unwrap();

        let mut succeeded = running.clone();
        succeeded.revision = 4;
        succeeded.record.status = CreativeTemplateRunStatus::Succeeded;
        succeeded.record.result_asset_ids = vec![ASSET.into()];
        succeeded.record.completed_at = Some(1_003);
        running.validate_transition(&succeeded).unwrap();
        assert!(succeeded.validate_transition(&succeeded).is_err());
    }

    #[test]
    fn rejects_snapshot_input_and_status_drift() {
        let mut aggregate = requested();
        aggregate.request.template_revision = 2;
        assert!(aggregate.validate().unwrap_err().contains("pinned definition"));

        let mut aggregate = requested();
        aggregate.request.inputs.clear();
        assert!(aggregate.validate().unwrap_err().contains("required template input"));

        let mut aggregate = requested();
        aggregate.record.status = CreativeTemplateRunStatus::Succeeded;
        assert!(aggregate.validate().unwrap_err().contains("incomplete"));
    }

    #[test]
    fn series_run_requires_planning_review_before_image_execution() {
        let requested = requested_series();
        assert_eq!(requested.expected_task_count(), 3);
        assert_eq!(requested.expected_result_asset_count(), 2);

        let mut queued = requested.clone();
        queued.revision = 2;
        queued.record.status = CreativeTemplateRunStatus::Queued;
        queued.record.task_ids = vec![TASK.into(), TASK_2.into(), TASK_3.into()];
        queued.record.queued_at = Some(1_001);
        requested.validate_transition(&queued).unwrap();

        let mut planning = queued.clone();
        planning.revision = 3;
        planning.record.status = CreativeTemplateRunStatus::Running;
        planning.record.started_at = Some(1_002);
        queued.validate_transition(&planning).unwrap();

        let drafts = vec![
            CreativeTemplatePromptDraft {
                id: DRAFT_1.into(),
                template_id: DEFINITION_ID.into(),
                run_request_id: RUN.into(),
                series_index: 0,
                title: "第一张".into(),
                prompt: "第一张海报".into(),
                status: CreativeTemplatePromptDraftStatus::PendingReview,
                created_at: 1_003,
                reviewed_at: None,
                review_note: None,
            },
            CreativeTemplatePromptDraft {
                id: DRAFT_2.into(),
                template_id: DEFINITION_ID.into(),
                run_request_id: RUN.into(),
                series_index: 1,
                title: "第二张".into(),
                prompt: "第二张海报".into(),
                status: CreativeTemplatePromptDraftStatus::PendingReview,
                created_at: 1_003,
                reviewed_at: None,
                review_note: None,
            },
        ];
        let mut review = planning.clone();
        review.revision = 4;
        review.record.status = CreativeTemplateRunStatus::AwaitingReview;
        review.record.prompt_draft_ids = drafts.iter().map(|draft| draft.id.clone()).collect();
        review.prompt_drafts = drafts;
        planning.validate_transition(&review).unwrap();

        let mut approved = review.clone();
        approved.revision = 5;
        for draft in &mut approved.prompt_drafts {
            draft.status = CreativeTemplatePromptDraftStatus::Approved;
            draft.reviewed_at = Some(1_004);
        }
        review.validate_transition(&approved).unwrap();

        let mut image_phase = approved.clone();
        image_phase.revision = 6;
        image_phase.record.status = CreativeTemplateRunStatus::Running;
        approved.validate_transition(&image_phase).unwrap();

        let mut illegal_edit = image_phase.clone();
        illegal_edit.revision = 7;
        illegal_edit.prompt_drafts[0].prompt = "偷偷改写".into();
        assert!(image_phase.validate_transition(&illegal_edit).is_err());
    }
}
