//! Canonical Creative Studio workflow definition v1.
//!
//! This is a closed server-owned persistence boundary.  It mirrors the
//! renderer's typed workflow domain, rejects unknown fields through Serde, and
//! validates graph/reference invariants before any definition reaches SQLite.

use std::collections::{BTreeSet, HashMap, VecDeque};

use nomifun_common::validate_uuidv7;
use nomifun_db::CreativeStudioWorkflowRow;
use serde::{Deserialize, Serialize};

pub const MAX_WORKFLOW_DEFINITION_BYTES: usize = 8 * 1024 * 1024;
const MAX_TEXT: usize = 20_000;
const MAX_PROMPT: usize = 200_000;
const MAX_VARIABLES: usize = 100;
const MAX_TEMPLATES: usize = 50;
const MAX_TEMPLATE_SEGMENTS: usize = 500;
const MAX_STEPS: usize = 200;
const MAX_TAGS: usize = 30;
const MAX_SERIES_ITEMS: usize = 20;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeWorkflowDefinitionV1 {
    pub id: String,
    pub revision: i64,
    pub metadata: CreativeWorkflowMetadata,
    pub output: CreativeWorkflowOutputPlan,
    pub variables: Vec<CreativeWorkflowVariable>,
    pub templates: Vec<CreativeWorkflowTemplate>,
    pub steps: Vec<CreativeWorkflowStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeWorkflowMetadata {
    pub name: String,
    pub description: String,
    pub category: String,
    pub visibility: CreativeWorkflowVisibility,
    pub tags: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CreativeWorkflowVisibility {
    Private,
    Public,
}

impl CreativeWorkflowVisibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Public => "public",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CreativeWorkflowOutputPlan {
    SingleImage,
    MultiImageSeries {
        target_count: usize,
        concurrency: usize,
        review_required: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CreativeWorkflowVariable {
    Text {
        id: String,
        key: String,
        label: String,
        description: String,
        required: bool,
        default_value: Option<String>,
        placeholder: String,
        min_length: usize,
        max_length: usize,
    },
    MultilineText {
        id: String,
        key: String,
        label: String,
        description: String,
        required: bool,
        default_value: Option<String>,
        placeholder: String,
        min_length: usize,
        max_length: usize,
    },
    Number {
        id: String,
        key: String,
        label: String,
        description: String,
        required: bool,
        default_value: Option<f64>,
        minimum: Option<f64>,
        maximum: Option<f64>,
        step: Option<f64>,
    },
    Boolean {
        id: String,
        key: String,
        label: String,
        description: String,
        required: bool,
        default_value: bool,
    },
    Choice {
        id: String,
        key: String,
        label: String,
        description: String,
        required: bool,
        default_value: Option<String>,
        options: Vec<String>,
    },
    Image {
        id: String,
        key: String,
        label: String,
        description: String,
        required: bool,
        default_asset_id: Option<String>,
    },
    ImageSeries {
        id: String,
        key: String,
        label: String,
        description: String,
        required: bool,
        default_asset_ids: Vec<String>,
        min_items: usize,
        max_items: usize,
    },
}

impl CreativeWorkflowVariable {
    fn common(&self) -> (&str, &str, &str, &str) {
        match self {
            Self::Text { id, key, label, description, .. }
            | Self::MultilineText { id, key, label, description, .. }
            | Self::Number { id, key, label, description, .. }
            | Self::Boolean { id, key, label, description, .. }
            | Self::Choice { id, key, label, description, .. }
            | Self::Image { id, key, label, description, .. }
            | Self::ImageSeries { id, key, label, description, .. } => {
                (id, key, label, description)
            }
        }
    }

    fn is_image(&self) -> bool {
        matches!(self, Self::Image { .. } | Self::ImageSeries { .. })
    }

    fn collect_asset_ids<'a>(&'a self, output: &mut BTreeSet<&'a str>) {
        match self {
            Self::Image { default_asset_id: Some(id), .. } => {
                output.insert(id);
            }
            Self::ImageSeries { default_asset_ids, .. } => output.extend(default_asset_ids.iter().map(String::as_str)),
            _ => {}
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeWorkflowTemplate {
    pub id: String,
    pub name: String,
    pub segments: Vec<CreativeWorkflowTemplateSegment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CreativeWorkflowTemplateSegment {
    Text { text: String },
    Variable { variable_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CreativeWorkflowPromptSource {
    Template { template_id: String },
    PromptDrafts { step_id: String },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CreativeWorkflowImageTask {
    ImageGeneration,
    ImageEdit,
}

impl CreativeWorkflowImageTask {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ImageGeneration => "image_generation",
            Self::ImageEdit => "image_edit",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeWorkflowImageModelBinding {
    pub provider_id: String,
    pub model: String,
    pub task: CreativeWorkflowImageTask,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CreativeWorkflowImageQuality {
    Auto,
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeWorkflowImageGenerationSettings {
    pub model: Option<CreativeWorkflowImageModelBinding>,
    pub quality: CreativeWorkflowImageQuality,
    pub width: usize,
    pub height: usize,
    pub images_per_prompt: usize,
}

impl CreativeWorkflowImageGenerationSettings {
    fn validate(&self, path: &str) -> Result<(), String> {
        if let Some(model) = self.model.as_ref() {
            validate_id(&format!("{path}.model.providerId"), &model.provider_id)?;
            validate_text(&format!("{path}.model.model"), &model.model, 512, false)?;
        }
        for (name, value) in [("width", self.width), ("height", self.height)] {
            if !(64..=8192).contains(&value) || value % 16 != 0 {
                return Err(format!(
                    "{path}.{name} must be 64..8192 and aligned to 16 pixels"
                ));
            }
        }
        if !(1..=6).contains(&self.images_per_prompt) {
            return Err(format!("{path}.imagesPerPrompt must be between 1 and 6"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CreativeWorkflowStep {
    RenderTemplate {
        id: String,
        name: String,
        depends_on: Vec<String>,
        enabled: bool,
        template_id: String,
    },
    DraftPrompts {
        id: String,
        name: String,
        depends_on: Vec<String>,
        enabled: bool,
        template_id: String,
    },
    GenerateImages {
        id: String,
        name: String,
        depends_on: Vec<String>,
        enabled: bool,
        prompt_source: CreativeWorkflowPromptSource,
        reference_variable_ids: Vec<String>,
        generation: CreativeWorkflowImageGenerationSettings,
    },
    RecordHistory {
        id: String,
        name: String,
        depends_on: Vec<String>,
        enabled: bool,
        source_step_ids: Vec<String>,
    },
}

impl CreativeWorkflowStep {
    fn common(&self) -> (&str, &str, &[String]) {
        match self {
            Self::RenderTemplate { id, name, depends_on, .. }
            | Self::DraftPrompts { id, name, depends_on, .. }
            | Self::GenerateImages { id, name, depends_on, .. }
            | Self::RecordHistory { id, name, depends_on, .. } => (id, name, depends_on),
        }
    }

    fn is_generate_images(&self) -> bool {
        matches!(self, Self::GenerateImages { .. })
    }
}

impl CreativeWorkflowDefinitionV1 {
    pub fn validate(&self) -> Result<(), String> {
        validate_id("id", &self.id)?;
        if self.revision < 1 {
            return Err("revision must be a positive integer".into());
        }
        validate_text("metadata.name", &self.metadata.name, 120, false)?;
        validate_text("metadata.description", &self.metadata.description, 2_000, true)?;
        validate_text("metadata.category", &self.metadata.category, 80, true)?;
        validate_unique_texts("metadata.tags", &self.metadata.tags, MAX_TAGS, 120)?;
        if self.metadata.created_at < 0 || self.metadata.updated_at < self.metadata.created_at {
            return Err("workflow metadata timestamps are invalid".into());
        }
        if self.variables.len() > MAX_VARIABLES
            || self.templates.len() > MAX_TEMPLATES
            || self.steps.len() > MAX_STEPS
        {
            return Err("workflow collection limit exceeded".into());
        }
        if self.templates.is_empty()
            || self.steps.is_empty()
            || !self.steps.iter().any(CreativeWorkflowStep::is_generate_images)
        {
            return Err("workflow requires a prompt template and image-generation step".into());
        }
        if let CreativeWorkflowOutputPlan::MultiImageSeries {
            target_count,
            concurrency,
            ..
        } = self.output
        {
            if !(2..=MAX_SERIES_ITEMS).contains(&target_count)
                || concurrency == 0
                || concurrency > 6
                || concurrency > target_count
            {
                return Err("multi-image output bounds are invalid".into());
            }
        }

        let mut owned_ids = BTreeSet::new();
        owned_ids.insert(self.id.as_str());
        let mut variable_keys = BTreeSet::new();
        for (index, variable) in self.variables.iter().enumerate() {
            let (id, key, label, description) = variable.common();
            validate_id(&format!("variables[{index}].id"), id)?;
            if !owned_ids.insert(id) {
                return Err("workflow identifiers must be globally unique".into());
            }
            if !variable_keys.insert(key) || !valid_variable_key(key) {
                return Err(format!("variables[{index}].key is invalid or duplicated"));
            }
            validate_text(&format!("variables[{index}].label"), label, 120, false)?;
            validate_text(&format!("variables[{index}].description"), description, 1_000, true)?;
            validate_variable(variable, index)?;
        }

        let variables: HashMap<&str, &CreativeWorkflowVariable> = self
            .variables
            .iter()
            .map(|variable| (variable.common().0, variable))
            .collect();
        let mut templates = HashMap::new();
        for (index, template) in self.templates.iter().enumerate() {
            validate_id(&format!("templates[{index}].id"), &template.id)?;
            if !owned_ids.insert(&template.id) || templates.insert(template.id.as_str(), template).is_some() {
                return Err("workflow identifiers must be globally unique".into());
            }
            validate_text(&format!("templates[{index}].name"), &template.name, 120, false)?;
            if template.segments.is_empty() || template.segments.len() > MAX_TEMPLATE_SEGMENTS {
                return Err(format!("templates[{index}].segments must contain 1 to 500 entries"));
            }
            for (segment_index, segment) in template.segments.iter().enumerate() {
                match segment {
                    CreativeWorkflowTemplateSegment::Text { text } => validate_text(
                        &format!("templates[{index}].segments[{segment_index}].text"),
                        text,
                        MAX_PROMPT,
                        true,
                    )?,
                    CreativeWorkflowTemplateSegment::Variable { variable_id } => {
                        validate_id("template variableId", variable_id)?;
                        let variable = variables.get(variable_id.as_str()).ok_or_else(|| {
                            format!("templates[{index}] references a missing variable")
                        })?;
                        if variable.is_image() {
                            return Err("image inputs cannot be interpolated into prompt text".into());
                        }
                    }
                }
            }
        }

        let mut steps = HashMap::new();
        for (index, step) in self.steps.iter().enumerate() {
            let (id, name, dependencies) = step.common();
            validate_id(&format!("steps[{index}].id"), id)?;
            if !owned_ids.insert(id) || steps.insert(id, step).is_some() {
                return Err("workflow identifiers must be globally unique".into());
            }
            validate_text(&format!("steps[{index}].name"), name, 120, false)?;
            validate_unique_ids(&format!("steps[{index}].dependsOn"), dependencies, MAX_STEPS)?;
            if dependencies.iter().any(|dependency| dependency == id) {
                return Err(format!("steps[{index}] cannot depend on itself"));
            }
        }
        for (index, step) in self.steps.iter().enumerate() {
            for dependency in step.common().2 {
                if !steps.contains_key(dependency.as_str()) {
                    return Err(format!("steps[{index}] references a missing dependency"));
                }
            }
            validate_step_references(self, index, step, &variables, &templates, &steps)?;
        }
        validate_acyclic(&self.steps, &steps)?;
        Ok(())
    }

    pub fn collect_asset_ids(&self) -> BTreeSet<&str> {
        let mut ids = BTreeSet::new();
        for variable in &self.variables {
            variable.collect_asset_ids(&mut ids);
        }
        ids
    }

    pub fn image_model_bindings(
        &self,
    ) -> impl Iterator<Item = &CreativeWorkflowImageModelBinding> {
        self.steps.iter().filter_map(|step| match step {
            CreativeWorkflowStep::GenerateImages { generation, .. } => generation.model.as_ref(),
            _ => None,
        })
    }

    pub fn to_row(&self) -> Result<CreativeStudioWorkflowRow, String> {
        self.validate()?;
        let definition_json = serde_json::to_string(self)
            .map_err(|error| format!("failed to serialize workflow definition: {error}"))?;
        if definition_json.len() > MAX_WORKFLOW_DEFINITION_BYTES {
            return Err("workflow definition exceeds the 8 MiB limit".into());
        }
        Ok(CreativeStudioWorkflowRow {
            id: 0,
            workflow_id: self.id.clone(),
            revision: self.revision,
            name: self.metadata.name.clone(),
            description: self.metadata.description.clone(),
            category: self.metadata.category.clone(),
            visibility: self.metadata.visibility.as_str().into(),
            definition_json,
            created_at: self.metadata.created_at,
            updated_at: self.metadata.updated_at,
        })
    }
}

pub fn parse_workflow_row(
    row: &CreativeStudioWorkflowRow,
) -> Result<CreativeWorkflowDefinitionV1, String> {
    if row.definition_json.len() > MAX_WORKFLOW_DEFINITION_BYTES {
        return Err("stored workflow definition exceeds the 8 MiB limit".into());
    }
    let definition: CreativeWorkflowDefinitionV1 = serde_json::from_str(&row.definition_json)
        .map_err(|error| format!("stored workflow JSON is invalid: {error}"))?;
    definition.validate()?;
    if definition.id != row.workflow_id
        || definition.revision != row.revision
        || definition.metadata.name != row.name
        || definition.metadata.description != row.description
        || definition.metadata.category != row.category
        || definition.metadata.visibility.as_str() != row.visibility
        || definition.metadata.created_at != row.created_at
        || definition.metadata.updated_at != row.updated_at
    {
        return Err("stored workflow row metadata does not match its canonical definition".into());
    }
    Ok(definition)
}

fn validate_variable(variable: &CreativeWorkflowVariable, index: usize) -> Result<(), String> {
    match variable {
        CreativeWorkflowVariable::Text {
            default_value,
            placeholder,
            min_length,
            max_length,
            ..
        }
        | CreativeWorkflowVariable::MultilineText {
            default_value,
            placeholder,
            min_length,
            max_length,
            ..
        } => {
            if *max_length > MAX_TEXT || min_length > max_length {
                return Err(format!("variables[{index}] text bounds are invalid"));
            }
            validate_text(&format!("variables[{index}].placeholder"), placeholder, 500, true)?;
            if let Some(value) = default_value {
                validate_text(&format!("variables[{index}].defaultValue"), value, MAX_TEXT, true)?;
                let length = utf16_len(value);
                if length < *min_length || length > *max_length {
                    return Err(format!("variables[{index}] default text is outside its bounds"));
                }
            }
        }
        CreativeWorkflowVariable::Number { default_value, minimum, maximum, step, .. } => {
            for value in [default_value, minimum, maximum, step].into_iter().flatten() {
                if !value.is_finite() {
                    return Err(format!("variables[{index}] number values must be finite"));
                }
            }
            if minimum.zip(*maximum).is_some_and(|(minimum, maximum)| minimum > maximum)
                || step.is_some_and(|step| step <= 0.0)
                || default_value.is_some_and(|value| {
                    minimum.is_some_and(|minimum| value < minimum)
                        || maximum.is_some_and(|maximum| value > maximum)
                })
            {
                return Err(format!("variables[{index}] number bounds are invalid"));
            }
        }
        CreativeWorkflowVariable::Choice { default_value, options, .. } => {
            validate_unique_texts(&format!("variables[{index}].options"), options, 100, 120)?;
            if options.is_empty() || default_value.as_ref().is_some_and(|value| !options.contains(value)) {
                return Err(format!("variables[{index}] choice default is invalid"));
            }
        }
        CreativeWorkflowVariable::Image { default_asset_id, .. } => {
            if let Some(id) = default_asset_id {
                validate_id(&format!("variables[{index}].defaultAssetId"), id)?;
            }
        }
        CreativeWorkflowVariable::ImageSeries {
            default_asset_ids,
            min_items,
            max_items,
            ..
        } => {
            validate_unique_ids(
                &format!("variables[{index}].defaultAssetIds"),
                default_asset_ids,
                MAX_SERIES_ITEMS,
            )?;
            if min_items > max_items
                || *max_items > MAX_SERIES_ITEMS
                || default_asset_ids.len() < *min_items
                || default_asset_ids.len() > *max_items
            {
                return Err(format!("variables[{index}] image-series bounds are invalid"));
            }
        }
        CreativeWorkflowVariable::Boolean { .. } => {}
    }
    Ok(())
}

fn validate_step_references<'a>(
    definition: &CreativeWorkflowDefinitionV1,
    index: usize,
    step: &CreativeWorkflowStep,
    variables: &HashMap<&'a str, &'a CreativeWorkflowVariable>,
    templates: &HashMap<&'a str, &'a CreativeWorkflowTemplate>,
    steps: &HashMap<&'a str, &'a CreativeWorkflowStep>,
) -> Result<(), String> {
    match step {
        CreativeWorkflowStep::RenderTemplate { template_id, .. } => {
            if !templates.contains_key(template_id.as_str()) {
                return Err(format!("steps[{index}] references a missing template"));
            }
        }
        CreativeWorkflowStep::DraftPrompts { template_id, .. } => {
            if !templates.contains_key(template_id.as_str()) {
                return Err(format!("steps[{index}] references a missing template"));
            }
            if !matches!(definition.output, CreativeWorkflowOutputPlan::MultiImageSeries { .. }) {
                return Err("draft-prompts is only valid for a multi-image workflow".into());
            }
        }
        CreativeWorkflowStep::GenerateImages {
            depends_on,
            prompt_source,
            reference_variable_ids,
            generation,
            ..
        } => {
            generation.validate(&format!("steps[{index}].generation"))?;
            validate_unique_ids(
                &format!("steps[{index}].referenceVariableIds"),
                reference_variable_ids,
                MAX_VARIABLES,
            )?;
            if reference_variable_ids.iter().any(|id| {
                variables.get(id.as_str()).is_none_or(|variable| !variable.is_image())
            }) {
                return Err(format!("steps[{index}] has an invalid image reference variable"));
            }
            let expected_task = if reference_variable_ids.is_empty() {
                CreativeWorkflowImageTask::ImageGeneration
            } else {
                CreativeWorkflowImageTask::ImageEdit
            };
            if generation
                .model
                .as_ref()
                .is_some_and(|model| model.task != expected_task)
            {
                return Err(format!(
                    "steps[{index}] generation model must use {}",
                    expected_task.as_str()
                ));
            }
            match prompt_source {
                CreativeWorkflowPromptSource::Template { template_id } => {
                    if !templates.contains_key(template_id.as_str())
                        || !matches!(definition.output, CreativeWorkflowOutputPlan::SingleImage)
                    {
                        return Err(format!("steps[{index}] has an invalid template prompt source"));
                    }
                }
                CreativeWorkflowPromptSource::PromptDrafts { step_id } => {
                    let valid_source = steps
                        .get(step_id.as_str())
                        .is_some_and(|source| matches!(source, CreativeWorkflowStep::DraftPrompts { .. }));
                    if !valid_source
                        || !depends_on.contains(step_id)
                        || !matches!(definition.output, CreativeWorkflowOutputPlan::MultiImageSeries { .. })
                    {
                        return Err(format!("steps[{index}] has an invalid prompt-draft source"));
                    }
                }
            }
        }
        CreativeWorkflowStep::RecordHistory { depends_on, source_step_ids, .. } => {
            validate_unique_ids(
                &format!("steps[{index}].sourceStepIds"),
                source_step_ids,
                MAX_STEPS,
            )?;
            if source_step_ids.is_empty()
                || source_step_ids.iter().any(|source_id| {
                    !depends_on.contains(source_id)
                        || steps.get(source_id.as_str()).is_none_or(|source| !source.is_generate_images())
                })
            {
                return Err(format!("steps[{index}] has an invalid history source"));
            }
        }
    }
    Ok(())
}

fn validate_acyclic(
    ordered_steps: &[CreativeWorkflowStep],
    steps: &HashMap<&str, &CreativeWorkflowStep>,
) -> Result<(), String> {
    let mut indegree: HashMap<&str, usize> = ordered_steps
        .iter()
        .map(|step| (step.common().0, step.common().2.len()))
        .collect();
    let mut followers: HashMap<&str, Vec<&str>> = HashMap::new();
    for step in ordered_steps {
        let id = step.common().0;
        for dependency in step.common().2 {
            followers.entry(dependency).or_default().push(id);
        }
    }
    let mut queue: VecDeque<&str> = ordered_steps
        .iter()
        .filter_map(|step| (step.common().2.is_empty()).then_some(step.common().0))
        .collect();
    let mut visited = 0;
    while let Some(id) = queue.pop_front() {
        if !steps.contains_key(id) {
            continue;
        }
        visited += 1;
        for follower in followers.get(id).into_iter().flatten() {
            if let Some(value) = indegree.get_mut(follower) {
                *value -= 1;
                if *value == 0 {
                    queue.push_back(follower);
                }
            }
        }
    }
    if visited != ordered_steps.len() {
        return Err("workflow step graph must be acyclic".into());
    }
    Ok(())
}

fn validate_id(path: &str, value: &str) -> Result<(), String> {
    validate_uuidv7(value)
        .map(|_| ())
        .map_err(|error| format!("{path} must be a canonical UUIDv7: {error}"))
}

fn utf16_len(value: &str) -> usize {
    value.encode_utf16().count()
}

fn validate_text(path: &str, value: &str, maximum: usize, allow_empty: bool) -> Result<(), String> {
    let has_control = value.chars().any(|value| {
        matches!(value as u32, 0x00..=0x08 | 0x0b | 0x0c | 0x0e..=0x1f | 0x7f)
    });
    if utf16_len(value) > maximum || has_control || (!allow_empty && value.trim().is_empty()) {
        return Err(format!("{path} must contain bounded display text"));
    }
    Ok(())
}

fn validate_unique_texts(
    path: &str,
    values: &[String],
    maximum: usize,
    text_maximum: usize,
) -> Result<(), String> {
    if values.len() > maximum {
        return Err(format!("{path} exceeds its item limit"));
    }
    let mut unique = BTreeSet::new();
    for value in values {
        validate_text(path, value, text_maximum, false)?;
        if !unique.insert(value.as_str()) {
            return Err(format!("{path} contains duplicate values"));
        }
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

fn valid_variable_key(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    first.is_ascii_lowercase()
        && value.len() <= 64
        && characters.all(|character| character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORKFLOW: &str = "0190f5fe-7c00-7a00-8abc-000000000201";
    const VARIABLE: &str = "0190f5fe-7c00-7a00-8abc-000000000202";
    const TEMPLATE: &str = "0190f5fe-7c00-7a00-8abc-000000000203";
    const RENDER: &str = "0190f5fe-7c00-7a00-8abc-000000000204";
    const GENERATE: &str = "0190f5fe-7c00-7a00-8abc-000000000205";

    fn definition() -> CreativeWorkflowDefinitionV1 {
        CreativeWorkflowDefinitionV1 {
            id: WORKFLOW.into(),
            revision: 1,
            metadata: CreativeWorkflowMetadata {
                name: "电商海报".into(),
                description: "固定结构".into(),
                category: "电商".into(),
                visibility: CreativeWorkflowVisibility::Private,
                tags: vec!["海报".into()],
                created_at: 100,
                updated_at: 100,
            },
            output: CreativeWorkflowOutputPlan::SingleImage,
            variables: vec![CreativeWorkflowVariable::Text {
                id: VARIABLE.into(),
                key: "product_name".into(),
                label: "产品名称".into(),
                description: String::new(),
                required: true,
                default_value: None,
                placeholder: String::new(),
                min_length: 0,
                max_length: 200,
            }],
            templates: vec![CreativeWorkflowTemplate {
                id: TEMPLATE.into(),
                name: "主提示词".into(),
                segments: vec![
                    CreativeWorkflowTemplateSegment::Text { text: "为 ".into() },
                    CreativeWorkflowTemplateSegment::Variable { variable_id: VARIABLE.into() },
                    CreativeWorkflowTemplateSegment::Text { text: " 生成海报".into() },
                ],
            }],
            steps: vec![
                CreativeWorkflowStep::RenderTemplate {
                    id: RENDER.into(),
                    name: "渲染提示词".into(),
                    depends_on: Vec::new(),
                    enabled: true,
                    template_id: TEMPLATE.into(),
                },
                CreativeWorkflowStep::GenerateImages {
                    id: GENERATE.into(),
                    name: "生成图片".into(),
                    depends_on: vec![RENDER.into()],
                    enabled: true,
                    prompt_source: CreativeWorkflowPromptSource::Template { template_id: TEMPLATE.into() },
                    reference_variable_ids: Vec::new(),
                    generation: CreativeWorkflowImageGenerationSettings {
                        model: None,
                        quality: CreativeWorkflowImageQuality::Auto,
                        width: 1024,
                        height: 1024,
                        images_per_prompt: 1,
                    },
                },
            ],
        }
    }

    #[test]
    fn validates_and_round_trips_a_closed_definition() {
        let definition = definition();
        definition.validate().unwrap();
        let json = serde_json::to_string(&definition).unwrap();
        let parsed: CreativeWorkflowDefinitionV1 = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, definition);
        assert_eq!(parsed.to_row().unwrap().name, "电商海报");
    }

    #[test]
    fn rejects_unknown_fields_broken_references_and_cycles() {
        let mut value = serde_json::to_value(definition()).unwrap();
        value.as_object_mut().unwrap().insert("legacy".into(), serde_json::json!(true));
        assert!(serde_json::from_value::<CreativeWorkflowDefinitionV1>(value).is_err());

        let mut missing = definition();
        if let CreativeWorkflowStep::GenerateImages { depends_on, .. } = &mut missing.steps[1] {
            depends_on[0] = "0190f5fe-7c00-7a00-8abc-000000000299".into();
        }
        assert!(missing.validate().unwrap_err().contains("missing dependency"));

        let mut cyclic = definition();
        if let CreativeWorkflowStep::RenderTemplate { depends_on, .. } = &mut cyclic.steps[0] {
            depends_on.push(GENERATE.into());
        }
        assert!(cyclic.validate().unwrap_err().contains("acyclic"));

        let mut mismatched = definition();
        if let CreativeWorkflowStep::GenerateImages { generation, .. } = &mut mismatched.steps[1] {
            generation.model = Some(CreativeWorkflowImageModelBinding {
                provider_id: "0190f5fe-7c00-7a00-8abc-000000000206".into(),
                model: "image-model".into(),
                task: CreativeWorkflowImageTask::ImageEdit,
            });
        }
        assert!(
            mismatched
                .validate()
                .unwrap_err()
                .contains("must use image_generation")
        );
    }
}
