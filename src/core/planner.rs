use std::collections::HashSet;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanPromptMode {
    ChatExecution,
    TaskAutomation,
    GoalLoop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlanStepStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanSubstep {
    pub id: usize,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<PlanStepStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    pub id: usize,
    pub title: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<PlanStepStatus>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub substeps: Vec<PlanSubstep>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecutionPlan {
    pub plan_id: String,
    pub revision: u32,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub steps: Vec<PlanStep>,
}

const DEFAULT_MAX_PLAN_STEPS: usize = 8;
pub const DEFAULT_MAX_CONFIRMATION_PLAN_STEPS: usize = 7;
pub const DEFAULT_MAX_ACTIONS_FOR_PLAN: usize = 8;

#[derive(Debug, Clone, PartialEq)]
pub struct ConfirmationPlanRelevance {
    pub accepted: bool,
    pub anchor_hits: usize,
    pub anchor_count: usize,
    pub grounding_ratio: f32,
    pub request_anchors: Vec<String>,
    pub matched_anchors: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct RequestOutlineSection {
    heading: Option<String>,
    items: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct RequestOutline {
    objective: String,
    sections: Vec<RequestOutlineSection>,
}

fn extract_json(text: &str) -> Option<serde_json::Value> {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .or_else(|| {
            let start = text.find('{').or_else(|| text.find('['))?;
            let end = text.rfind('}').or_else(|| text.rfind(']'))?;
            serde_json::from_str::<serde_json::Value>(&text[start..=end]).ok()
        })
}

fn action_catalog_for_prompt(
    actions: &[crate::actions::ActionDef],
    compact: bool,
) -> Vec<serde_json::Value> {
    actions
        .iter()
        .map(|action| {
            let mut record = serde_json::Map::new();
            record.insert("name".to_string(), serde_json::json!(action.name));
            record.insert(
                "description".to_string(),
                serde_json::json!(action.description),
            );
            record.insert(
                "action_metadata".to_string(),
                serde_json::json!(action.action_metadata()),
            );
            if !compact {
                record.insert(
                    "input_schema".to_string(),
                    serde_json::json!(action.input_schema),
                );
            }
            serde_json::Value::Object(record)
        })
        .collect()
}

fn allowed_action_names(actions: &[crate::actions::ActionDef]) -> HashSet<String> {
    actions
        .iter()
        .map(|action| action.name.trim().to_ascii_lowercase())
        .collect()
}

fn normalize_relevance_text(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|ch| if ch.is_alphanumeric() { ch } else { ' ' })
        .collect()
}

fn request_anchor_tokens(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut tokens = normalize_relevance_text(text)
        .split_whitespace()
        .filter(|token| token.chars().count() >= 4 && token.chars().any(|ch| ch.is_alphabetic()))
        .filter_map(|token| {
            let token = token.trim().to_string();
            if token.is_empty() || !seen.insert(token.clone()) {
                None
            } else {
                Some(token)
            }
        })
        .collect::<Vec<_>>();

    tokens.sort_by(|left, right| {
        right
            .chars()
            .count()
            .cmp(&left.chars().count())
            .then_with(|| left.cmp(right))
    });
    tokens.truncate(12);
    tokens
}

fn trim_request_list_marker(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    for prefix in ["- ", "* ", "+ "] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let rest = rest.trim();
            if !rest.is_empty() {
                return Some(rest);
            }
        }
    }

    let digit_prefix_len = trimmed
        .char_indices()
        .take_while(|(_, ch)| ch.is_ascii_digit())
        .last()
        .map(|(index, ch)| index + ch.len_utf8())
        .unwrap_or(0);
    if digit_prefix_len == 0 || digit_prefix_len >= trimmed.len() {
        return None;
    }

    let after_digits = &trimmed[digit_prefix_len..];
    let mut after_chars = after_digits.chars();
    let marker = after_chars.next()?;
    if !matches!(marker, '.' | ')' | ':') {
        return None;
    }

    let rest = after_chars.as_str().trim();
    if rest.is_empty() {
        None
    } else {
        Some(rest)
    }
}

fn parse_request_outline_heading(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trim_request_list_marker(trimmed).is_some() {
        return None;
    }
    let heading = trimmed.strip_suffix(':')?.trim();
    if heading.is_empty() || heading.chars().count() > 90 {
        return None;
    }
    Some(heading.to_string())
}

fn truncate_prompt_text(text: &str, max_chars: usize) -> String {
    let mut truncated = text.chars().take(max_chars).collect::<String>();
    if text.chars().count() > max_chars {
        truncated.push_str("...");
    }
    truncated
}

fn push_request_outline_section(
    sections: &mut Vec<RequestOutlineSection>,
    heading: Option<String>,
    items: &mut Vec<String>,
) {
    if items.is_empty() {
        return;
    }
    sections.push(RequestOutlineSection {
        heading,
        items: std::mem::take(items),
    });
}

fn extract_request_outline(request: &str) -> RequestOutline {
    let mut outline = RequestOutline::default();
    let mut current_heading: Option<String> = None;
    let mut current_items: Vec<String> = Vec::new();

    for raw_line in request.lines().take(240) {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        if outline.objective.is_empty()
            && trim_request_list_marker(line).is_none()
            && parse_request_outline_heading(line).is_none()
        {
            outline.objective = truncate_prompt_text(line, 260);
            continue;
        }

        if let Some(heading) = parse_request_outline_heading(line) {
            push_request_outline_section(
                &mut outline.sections,
                current_heading.take(),
                &mut current_items,
            );
            current_heading = Some(heading);
            continue;
        }

        let entry = trim_request_list_marker(line).unwrap_or(line);
        let entry = truncate_prompt_text(entry, 220);
        if !entry.is_empty() {
            current_items.push(entry);
        }
    }

    push_request_outline_section(
        &mut outline.sections,
        current_heading.take(),
        &mut current_items,
    );
    outline
}

pub fn render_confirmation_request_grounding(request: &str) -> String {
    let outline = extract_request_outline(request);
    let anchors = request_anchor_tokens(request);
    let mut lines = Vec::new();

    if !outline.objective.is_empty() {
        lines.push(format!("Primary objective: {}", outline.objective));
    }

    for section in outline.sections.iter().take(8) {
        if let Some(heading) = section.heading.as_ref() {
            lines.push(format!("{}:", heading));
        }
        for item in section.items.iter().take(6) {
            lines.push(format!("- {}", item));
        }
    }

    if !anchors.is_empty() {
        lines.push(format!("Anchor terms: {}", anchors.join(", ")));
    }

    lines.join("\n")
}

pub fn confirmation_request_objective_and_items(request: &str) -> (String, Vec<String>) {
    let outline = extract_request_outline(request);
    let mut items = Vec::new();
    for section in outline.sections {
        for item in section.items {
            if !item.trim().is_empty() {
                items.push(item);
            }
        }
    }
    (outline.objective, items)
}

fn dense_alnum_text(text: &str) -> String {
    normalize_relevance_text(text)
        .chars()
        .filter(|ch| ch.is_alphanumeric())
        .collect()
}

fn char_ngrams(text: &str, n: usize) -> HashSet<String> {
    let chars = text.chars().collect::<Vec<_>>();
    if chars.len() < n {
        return HashSet::new();
    }

    (0..=chars.len() - n)
        .map(|index| chars[index..index + n].iter().collect::<String>())
        .collect()
}

fn confirmation_plan_text(plan: &ExecutionPlan) -> String {
    let mut parts = Vec::new();
    if !plan.summary.trim().is_empty() {
        parts.push(plan.summary.trim().to_string());
    }
    for step in &plan.steps {
        if !step.title.trim().is_empty() {
            parts.push(step.title.trim().to_string());
        }
        if !step.description.trim().is_empty() {
            parts.push(step.description.trim().to_string());
        }
    }
    parts.join(" ")
}

pub fn assess_confirmation_plan_relevance(
    request: &str,
    plan: &ExecutionPlan,
) -> ConfirmationPlanRelevance {
    let request_anchors = request_anchor_tokens(request);
    let plan_text = normalize_relevance_text(&confirmation_plan_text(plan));
    let matched_anchors = request_anchors
        .iter()
        .filter(|anchor| plan_text.contains(anchor.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let anchor_hits = matched_anchors.len();

    let request_ngrams = char_ngrams(&dense_alnum_text(request), 5);
    let plan_ngrams = char_ngrams(&dense_alnum_text(&confirmation_plan_text(plan)), 5);
    let grounding_overlap = request_ngrams.intersection(&plan_ngrams).count();
    let grounding_ratio = if plan_ngrams.is_empty() {
        0.0
    } else {
        grounding_overlap as f32 / plan_ngrams.len() as f32
    };

    let required_anchor_hits = match request_anchors.len() {
        0 => 0,
        1..=4 => 1,
        5..=9 => 2,
        _ => 3,
    };
    let accepted = if request_anchors.is_empty() {
        grounding_ratio >= 0.18
    } else {
        anchor_hits >= required_anchor_hits
            || grounding_ratio >= 0.22
            || (anchor_hits >= 1 && grounding_ratio >= 0.14)
    };

    ConfirmationPlanRelevance {
        accepted,
        anchor_hits,
        anchor_count: request_anchors.len(),
        grounding_ratio,
        request_anchors,
        matched_anchors,
    }
}

fn normalize_status(value: &serde_json::Value) -> Option<PlanStepStatus> {
    match value.as_str()?.trim().to_ascii_lowercase().as_str() {
        "pending" => Some(PlanStepStatus::Pending),
        "running" => Some(PlanStepStatus::Running),
        "completed" => Some(PlanStepStatus::Completed),
        "failed" => Some(PlanStepStatus::Failed),
        "skipped" => Some(PlanStepStatus::Skipped),
        _ => None,
    }
}

fn normalize_action_name(
    value: Option<&serde_json::Value>,
    allowed_actions: &HashSet<String>,
) -> Option<String> {
    let raw = value?.as_str()?.trim();
    if raw.is_empty() {
        return None;
    }
    let normalized = raw.to_ascii_lowercase();
    if allowed_actions.contains(&normalized) {
        Some(raw.to_string())
    } else {
        None
    }
}

fn normalize_arguments(value: Option<&serde_json::Value>) -> Option<serde_json::Value> {
    let value = value?.clone();
    if value.is_object() {
        Some(value)
    } else {
        None
    }
}

fn normalize_plan_step(
    index: usize,
    value: &serde_json::Value,
    allowed_actions: &HashSet<String>,
    include_status: bool,
) -> Option<PlanStep> {
    let record = value.as_object()?;
    let title = record
        .get("title")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("Step")
        .to_string();
    let description = record
        .get("description")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .unwrap_or("")
        .to_string();

    let mut action = normalize_action_name(record.get("action"), allowed_actions);
    let mut tool_hint = normalize_action_name(record.get("tool_hint"), allowed_actions);
    if action.is_none() {
        action = tool_hint.clone();
    }
    if tool_hint.is_none() {
        tool_hint = action.clone();
    }

    let status = if include_status {
        record.get("status").and_then(normalize_status)
    } else {
        None
    };

    let substeps = record
        .get("substeps")
        .and_then(|value| value.as_array())
        .map(|values| {
            values
                .iter()
                .enumerate()
                .filter_map(|(sub_index, value)| {
                    let record = value.as_object()?;
                    let title = record
                        .get("title")
                        .and_then(|value| value.as_str())
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .unwrap_or("Substep")
                        .to_string();
                    let description = record
                        .get("description")
                        .and_then(|value| value.as_str())
                        .map(str::trim)
                        .unwrap_or("")
                        .to_string();
                    let mut tool_hint =
                        normalize_action_name(record.get("tool_hint"), allowed_actions);
                    if tool_hint.is_none() {
                        tool_hint = normalize_action_name(record.get("action"), allowed_actions);
                    }
                    let status = if include_status {
                        record.get("status").and_then(normalize_status)
                    } else {
                        None
                    };
                    Some(PlanSubstep {
                        id: sub_index + 1,
                        title,
                        description,
                        tool_hint,
                        status,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Some(PlanStep {
        id: index + 1,
        title,
        description,
        action,
        arguments: normalize_arguments(record.get("arguments")),
        tool_hint,
        status,
        substeps,
    })
}

fn normalize_plan_steps(
    raw_steps: &[serde_json::Value],
    actions: &[crate::actions::ActionDef],
    include_status: bool,
) -> Vec<PlanStep> {
    let allowed = allowed_action_names(actions);
    raw_steps
        .iter()
        .enumerate()
        .filter_map(|(index, step)| normalize_plan_step(index, step, &allowed, include_status))
        .take(DEFAULT_MAX_PLAN_STEPS)
        .collect()
}

pub fn create_plan(
    summary: impl Into<String>,
    steps: Vec<PlanStep>,
    plan_id: Option<String>,
    revision: u32,
) -> ExecutionPlan {
    ExecutionPlan {
        plan_id: plan_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        revision,
        summary: summary.into().trim().to_string(),
        steps,
    }
}

pub fn build_action_selector_prompt(
    request: &str,
    refinement: Option<&str>,
    available_actions: &[crate::actions::ActionDef],
) -> (String, String) {
    let system = format!(
        "You are a task planner for an AI agent.\n\
Return ONLY valid JSON.\n\n\
Output schema:\n\
{{\n  \"summary\": \"short summary\",\n  \"needed_actions\": [\"action_name\"]\n}}\n\n\
Rules:\n\
- Use only the provided actions.\n\
- Keep the list minimal and relevant.\n\
- Select at most {} actions.\n\
- Do not include actions that are only for presenting the final answer.\n",
        DEFAULT_MAX_ACTIONS_FOR_PLAN
    );

    let mut user = format!(
        "Request:\n{}\n\nAvailable actions:\n{}",
        request.trim(),
        serde_json::to_string_pretty(
            &available_actions
                .iter()
                .map(|action| {
                    serde_json::json!({
                        "name": action.name,
                        "description": action.description,
                        "action_metadata": action.action_metadata(),
                    })
                })
                .collect::<Vec<_>>()
        )
        .unwrap_or_default()
    );

    if let Some(refinement) = refinement.map(str::trim).filter(|value| !value.is_empty()) {
        user.push_str("\n\nRefinement:\n");
        user.push_str(refinement);
    }

    (system, user)
}

pub fn parse_action_selection(
    raw: &str,
    available_actions: &[crate::actions::ActionDef],
    max_actions: usize,
) -> Vec<String> {
    let allowed = allowed_action_names(available_actions);
    let Some(value) = extract_json(raw) else {
        return Vec::new();
    };
    value
        .get("needed_actions")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let raw = item.as_str()?.trim();
                    if raw.is_empty() {
                        return None;
                    }
                    let normalized = raw.to_ascii_lowercase();
                    if allowed.contains(&normalized) {
                        Some(raw.to_string())
                    } else {
                        None
                    }
                })
                .take(max_actions)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

pub fn shortlist_actions(
    available_actions: &[crate::actions::ActionDef],
    selected_names: &[String],
    max_actions: usize,
) -> Vec<crate::actions::ActionDef> {
    let mut scoped = available_actions
        .iter()
        .filter(|action| selected_names.iter().any(|name| name == &action.name))
        .take(max_actions)
        .cloned()
        .collect::<Vec<_>>();

    if scoped.is_empty() {
        scoped = available_actions
            .iter()
            .take(max_actions)
            .cloned()
            .collect::<Vec<_>>();
    }

    scoped
}

pub fn build_plan_prompt(
    request: &str,
    refinement: Option<&str>,
    available_actions: &[crate::actions::ActionDef],
    mode: PlanPromptMode,
    current_plan: Option<&ExecutionPlan>,
) -> (String, String) {
    let mode_line = match mode {
        PlanPromptMode::ChatExecution => {
            "This plan is for live execution. Prefer concrete, observable steps that map directly to available actions."
        }
        PlanPromptMode::TaskAutomation => {
            "This plan is for a stored task/automation. Prefer steps with runnable action names and arguments."
        }
        PlanPromptMode::GoalLoop => {
            "This plan is for an ongoing goal loop. Prefer compact, reusable execution steps with runnable action names and arguments."
        }
    };

    let system = format!(
        "You are a task planner for an AI agent.\n\
Return ONLY valid JSON.\n\n\
Output schema:\n\
{{\n  \"summary\": \"short summary\",\n  \"steps\": [\n    {{\n      \"title\": \"short step title\",\n      \"description\": \"one sentence\",\n      \"action\": \"action_name or null\",\n      \"arguments\": {{}} ,\n      \"tool_hint\": \"action_name or null\"\n    }}\n  ]\n}}\n\n\
Rules:\n\
- Use only the provided actions.\n\
- 1-{} steps maximum.\n\
- Each step should be one logical action, not a sub-plan.\n\
- Do not add a separate final step just to summarize or present the result.\n\
- If a step maps directly to an available action, set both `action` and `tool_hint` to that exact action name.\n\
- If a step does not directly map to an available action, set both `action` and `tool_hint` to null.\n\
- Keep descriptions concrete and avoid filler.\n\
- Do not include `status`, `plan_id`, or `revision` in the response.\n\
- {}\n",
        DEFAULT_MAX_PLAN_STEPS, mode_line
    );

    let mut user = format!(
        "Request:\n{}\n\nAvailable actions:\n{}",
        request.trim(),
        serde_json::to_string_pretty(&action_catalog_for_prompt(available_actions, false))
            .unwrap_or_default()
    );

    if let Some(refinement) = refinement.map(str::trim).filter(|value| !value.is_empty()) {
        user.push_str("\n\nRefinement:\n");
        user.push_str(refinement);
    }

    if let Some(current_plan) = current_plan {
        user.push_str("\n\nCurrent execution plan:\n");
        user.push_str(&serde_json::to_string_pretty(current_plan).unwrap_or_default());
        user.push_str(
            "\n\nIf the plan must change, return a full replacement plan for the remaining work only.",
        );
    }

    (system, user)
}

pub fn build_confirmation_plan_prompt(
    request: &str,
    refinement: Option<&str>,
    available_actions: &[crate::actions::ActionDef],
) -> (String, String) {
    let action_names = available_actions
        .iter()
        .map(|action| action.name.trim())
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();
    let system = format!(
        "Generate a compact, request-specific execution outline that the user can review before the run starts.\n\
Return ONLY valid JSON with this schema:\n\
{{\n  \"summary\": \"short plan title\",\n  \"steps\": [\n    {{\n      \"title\": \"short step title\",\n      \"description\": \"short step description\",\n      \"action\": \"action_name or null\",\n      \"arguments\": {{}} ,\n      \"tool_hint\": \"action_name or null\"\n    }}\n  ]\n}}\n\n\
Rules:\n\
- Use only the provided action names.\n\
- 4-{} steps maximum.\n\
- Every step must stay on the request's actual subject.\n\
- Reuse the request's own entities, topics, focus items, comparisons, dates, and required sections when relevant.\n\
- Keep titles concrete and short.\n\
- Keep descriptions to one brief sentence.\n\
- If a step maps directly to an available action, set both `action` and `tool_hint` to that exact action name.\n\
- Otherwise set both `action` and `tool_hint` to null.\n\
- Do not include `status`, `plan_id`, or `revision`.\n",
        DEFAULT_MAX_CONFIRMATION_PLAN_STEPS
    );

    let mut user = format!(
        "Request:\n{}\n\nRequest grounding:\n{}\n\nAvailable action names:\n{}",
        request.trim(),
        {
            let request_grounding = render_confirmation_request_grounding(request);
            if request_grounding.is_empty() {
                "(none)".to_string()
            } else {
                request_grounding
            }
        },
        if action_names.is_empty() {
            "(none)".to_string()
        } else {
            action_names.join(", ")
        }
    );

    if let Some(refinement) = refinement.map(str::trim).filter(|value| !value.is_empty()) {
        user.push_str("\n\nRefinement:\n");
        user.push_str(refinement);
    }

    (system, user)
}

pub fn parse_plan_from_llm_content(
    raw: &str,
    available_actions: &[crate::actions::ActionDef],
    plan_id: Option<String>,
    revision: u32,
    include_status: bool,
) -> Option<ExecutionPlan> {
    let value = extract_json(raw)?;
    parse_plan_from_value(&value, available_actions, plan_id, revision, include_status)
}

pub fn parse_plan_from_value(
    value: &serde_json::Value,
    available_actions: &[crate::actions::ActionDef],
    plan_id: Option<String>,
    revision: u32,
    include_status: bool,
) -> Option<ExecutionPlan> {
    let (summary, raw_steps) = if let Some(array) = value.as_array() {
        (String::new(), array.clone())
    } else {
        let summary = value
            .get("summary")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .unwrap_or("")
            .to_string();
        let steps = value
            .get("steps")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        (summary, steps)
    };

    let steps = normalize_plan_steps(&raw_steps, available_actions, include_status);
    if steps.is_empty() {
        return None;
    }

    Some(create_plan(summary, steps, plan_id, revision))
}

#[cfg(test)]
pub fn truncate_plan_steps(plan: &ExecutionPlan, max_steps: usize) -> ExecutionPlan {
    if plan.steps.len() <= max_steps {
        return plan.clone();
    }

    let mut trimmed = plan.clone();
    trimmed.steps.truncate(max_steps.max(1));
    for (index, step) in trimmed.steps.iter_mut().enumerate() {
        step.id = index + 1;
    }
    trimmed
}

#[cfg(test)]
pub fn next_revision_plan(
    current_plan: &ExecutionPlan,
    replacement: &ExecutionPlan,
) -> ExecutionPlan {
    let mut steps = current_plan
        .steps
        .iter()
        .filter(|step| {
            matches!(
                step.status,
                Some(PlanStepStatus::Completed)
                    | Some(PlanStepStatus::Failed)
                    | Some(PlanStepStatus::Skipped)
            )
        })
        .cloned()
        .collect::<Vec<_>>();

    steps.extend(replacement.steps.iter().map(|step| {
        PlanStep {
            id: 0,
            title: step.title.clone(),
            description: step.description.clone(),
            action: step.action.clone(),
            arguments: step.arguments.clone(),
            tool_hint: step.tool_hint.clone(),
            status: Some(PlanStepStatus::Pending),
            substeps: step
                .substeps
                .iter()
                .enumerate()
                .map(|(sub_index, substep)| PlanSubstep {
                    id: sub_index + 1,
                    title: substep.title.clone(),
                    description: substep.description.clone(),
                    tool_hint: substep.tool_hint.clone(),
                    status: Some(PlanStepStatus::Pending),
                })
                .collect(),
        }
    }));

    for (index, step) in steps.iter_mut().enumerate() {
        step.id = index + 1;
    }

    create_plan(
        if replacement.summary.trim().is_empty() {
            current_plan.summary.clone()
        } else {
            replacement.summary.clone()
        },
        steps,
        Some(current_plan.plan_id.clone()),
        current_plan.revision.saturating_add(1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::{ActionDef, ActionSource};

    fn action(name: &str) -> ActionDef {
        ActionDef {
            name: name.to_string(),
            description: format!("{} description", name),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({}),
            capabilities: vec![],
            sandbox_mode: None,
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        }
    }

    fn plan_step(
        id: usize,
        title: &str,
        tool_hint: Option<&str>,
        status: Option<PlanStepStatus>,
    ) -> PlanStep {
        PlanStep {
            id,
            title: title.to_string(),
            description: format!("{} description", title),
            action: tool_hint.map(str::to_string),
            arguments: None,
            tool_hint: tool_hint.map(str::to_string),
            status,
            substeps: Vec::new(),
        }
    }

    #[test]
    fn parse_plan_from_canonical_object_keeps_summary_and_known_actions() {
        let actions = vec![action("file_write"), action("app_deploy")];
        let parsed = parse_plan_from_llm_content(
            r#"
            {
              "summary": "Ship the dashboard",
              "steps": [
                {
                  "title": "Write files",
                  "description": "Create the source files",
                  "action": "file_write",
                  "arguments": {"path":"/tmp/demo"},
                  "tool_hint": "file_write"
                },
                {
                  "title": "Deploy",
                  "description": "Launch the app",
                  "action": "app_deploy",
                  "tool_hint": "app_deploy"
                }
              ]
            }
            "#,
            &actions,
            Some("plan-1".to_string()),
            1,
            false,
        )
        .expect("canonical plan should parse");

        assert_eq!(parsed.plan_id, "plan-1");
        assert_eq!(parsed.revision, 1);
        assert_eq!(parsed.summary, "Ship the dashboard");
        assert_eq!(parsed.steps.len(), 2);
        assert_eq!(parsed.steps[0].action.as_deref(), Some("file_write"));
        assert_eq!(
            parsed.steps[0]
                .arguments
                .as_ref()
                .and_then(|value| value.get("path"))
                .and_then(|value| value.as_str()),
            Some("/tmp/demo")
        );
        assert_eq!(parsed.steps[1].tool_hint.as_deref(), Some("app_deploy"));
    }

    #[test]
    fn parse_plan_from_legacy_array_drops_unknown_actions() {
        let actions = vec![action("http_get")];
        let parsed = parse_plan_from_llm_content(
            r#"
            [
              {"title":"Check health","description":"Verify the site","tool_hint":"http_get"},
              {"title":"Notify","description":"Send an update","tool_hint":"email_send"}
            ]
            "#,
            &actions,
            Some("plan-legacy".to_string()),
            2,
            false,
        )
        .expect("legacy plan array should still parse");

        assert_eq!(parsed.summary, "");
        assert_eq!(parsed.steps.len(), 2);
        assert_eq!(parsed.steps[0].tool_hint.as_deref(), Some("http_get"));
        assert_eq!(parsed.steps[1].tool_hint, None);
        assert_eq!(parsed.steps[1].action, None);
    }

    #[test]
    fn parse_plan_rejects_malformed_or_empty_payloads() {
        let actions = vec![action("file_write")];
        assert!(parse_plan_from_llm_content("not json", &actions, None, 1, false).is_none());
        assert!(parse_plan_from_llm_content(
            r#"{"summary":"No steps","steps":[]}"#,
            &actions,
            None,
            1,
            false
        )
        .is_none());
    }

    #[test]
    fn next_revision_plan_preserves_completed_steps_and_appends_remaining_work() {
        let current = ExecutionPlan {
            plan_id: "plan-42".to_string(),
            revision: 3,
            summary: "Current summary".to_string(),
            steps: vec![
                plan_step(
                    1,
                    "Inspect",
                    Some("file_read"),
                    Some(PlanStepStatus::Completed),
                ),
                plan_step(
                    2,
                    "Patch",
                    Some("file_write"),
                    Some(PlanStepStatus::Running),
                ),
                plan_step(3, "Verify", Some("http_get"), Some(PlanStepStatus::Pending)),
            ],
        };
        let replacement = ExecutionPlan {
            plan_id: "ignored".to_string(),
            revision: 99,
            summary: "Revised summary".to_string(),
            steps: vec![
                plan_step(1, "Patch with new approach", Some("file_write"), None),
                plan_step(2, "Verify again", Some("http_get"), None),
            ],
        };

        let revised = next_revision_plan(&current, &replacement);

        assert_eq!(revised.plan_id, "plan-42");
        assert_eq!(revised.revision, 4);
        assert_eq!(revised.summary, "Revised summary");
        assert_eq!(revised.steps.len(), 3);
        assert_eq!(revised.steps[0].title, "Inspect");
        assert_eq!(revised.steps[0].status, Some(PlanStepStatus::Completed));
        assert_eq!(revised.steps[1].title, "Patch with new approach");
        assert_eq!(revised.steps[1].status, Some(PlanStepStatus::Pending));
        assert_eq!(revised.steps[2].title, "Verify again");
        assert_eq!(revised.steps[2].status, Some(PlanStepStatus::Pending));
    }

    #[test]
    fn confirmation_prompt_uses_compact_action_catalog() {
        let actions = vec![action("research"), action("web_search")];

        let (_, user) = build_confirmation_plan_prompt(
            "Assess alpha-beta-gamma tradeoffs for the next planning cycle",
            None,
            &actions,
        );

        assert!(user.contains("Request grounding:"));
        assert!(user.contains("Available action names:\nresearch, web_search"));
        assert!(!user.contains("\"action_metadata\""));
        assert!(!user.contains("\"input_schema\""));
        assert!(!user.contains("\"description\""));
    }

    #[test]
    fn confirmation_plan_relevance_accepts_request_aligned_plan() {
        let plan = ExecutionPlan {
            plan_id: "plan-confirm".to_string(),
            revision: 1,
            summary: "Alpha beta gamma review".to_string(),
            steps: vec![
                plan_step(1, "Map alpha capacity", Some("research"), None),
                plan_step(2, "Compare beta options", Some("research"), None),
                plan_step(3, "Evaluate gamma risks", None, None),
            ],
        };

        let assessment = assess_confirmation_plan_relevance(
            "Assess alpha capacity, beta infrastructure, and gamma funding priorities for the next planning cycle.",
            &plan,
        );

        assert!(assessment.accepted);
        assert!(assessment.anchor_hits >= 2);
        assert!(assessment.grounding_ratio > 0.05);
    }

    #[test]
    fn confirmation_plan_relevance_rejects_off_topic_plan() {
        let plan = ExecutionPlan {
            plan_id: "plan-off-topic".to_string(),
            revision: 1,
            summary: "Badge issuance workflow".to_string(),
            steps: vec![
                plan_step(1, "Collect badge request forms", None, None),
                plan_step(2, "Approve building access", None, None),
                plan_step(3, "Provision door credentials", None, None),
            ],
        };

        let assessment = assess_confirmation_plan_relevance(
            "Assess alpha capacity, beta infrastructure, and gamma funding priorities for the next planning cycle.",
            &plan,
        );

        assert!(!assessment.accepted);
        assert_eq!(assessment.anchor_hits, 0);
        assert!(assessment.grounding_ratio < 0.08);
    }

    #[test]
    fn truncate_plan_steps_keeps_summary_and_reindexes() {
        let plan = ExecutionPlan {
            plan_id: "plan-preview".to_string(),
            revision: 1,
            summary: "Compact plan".to_string(),
            steps: vec![
                plan_step(1, "One", Some("research"), None),
                plan_step(2, "Two", Some("research"), None),
                plan_step(3, "Three", Some("research"), None),
                plan_step(4, "Four", Some("research"), None),
            ],
        };

        let trimmed = truncate_plan_steps(&plan, 2);

        assert_eq!(trimmed.summary, "Compact plan");
        assert_eq!(trimmed.steps.len(), 2);
        assert_eq!(trimmed.steps[0].id, 1);
        assert_eq!(trimmed.steps[1].id, 2);
        assert_eq!(trimmed.steps[1].title, "Two");
    }
}
