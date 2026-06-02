use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, Serialize, Default)]
pub(super) struct ArkDistillAnalyticsTotals {
    pub result_count: i64,
    pub original_chars: i64,
    pub distilled_chars: i64,
    pub saved_chars: i64,
    pub estimated_original_tokens: i64,
    pub estimated_distilled_tokens: i64,
    pub estimated_saved_tokens: i64,
    pub estimated_prompt_cost_saved_usd: Option<f64>,
    pub average_reduction_ratio: f64,
    pub savings_percent: f64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub(super) struct ArkDistillAnalyticsPoint {
    pub bucket_start: String,
    pub result_count: i64,
    pub original_chars: i64,
    pub distilled_chars: i64,
    pub saved_chars: i64,
    pub estimated_saved_tokens: i64,
    pub estimated_prompt_cost_saved_usd: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub(super) struct ArkDistillToolSavingsRow {
    pub tool_name: String,
    pub action: Option<String>,
    pub result_count: i64,
    pub saved_chars: i64,
    pub estimated_saved_tokens: i64,
    pub estimated_prompt_cost_saved_usd: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub(super) struct ArkDistillAnalyticsSummary {
    pub totals: ArkDistillAnalyticsTotals,
    pub series: Vec<ArkDistillAnalyticsPoint>,
    pub by_tool: Vec<ArkDistillToolSavingsRow>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct ArkDistillModelPricingContext {
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, Default)]
pub(super) struct ArkDistillPricingContext {
    pub model_slots: HashMap<String, ArkDistillModelPricingContext>,
    pub default_model: Option<ArkDistillModelPricingContext>,
    pub openrouter_prices: HashMap<String, super::analytics_control::OpenRouterModelPricing>,
}

#[derive(Debug, Clone)]
struct ArkDistillParsedLog {
    created_at: chrono::DateTime<chrono::Utc>,
    tool_name: String,
    action: Option<String>,
    model_slot: Option<String>,
    model_provider: Option<String>,
    model: Option<String>,
    original_chars: i64,
    distilled_chars: i64,
    saved_chars: i64,
    estimated_original_tokens: i64,
    estimated_distilled_tokens: i64,
    estimated_saved_tokens: i64,
    estimated_prompt_cost_saved_usd: Option<f64>,
}

#[cfg(test)]
pub(super) fn summarize_arkdistill_logs(
    rows: &[crate::storage::entities::operational_log::Model],
) -> ArkDistillAnalyticsSummary {
    summarize_arkdistill_logs_window(rows, None, None, "hour")
}

#[cfg(test)]
pub(super) fn summarize_arkdistill_logs_window(
    rows: &[crate::storage::entities::operational_log::Model],
    since: Option<chrono::DateTime<chrono::Utc>>,
    until: Option<chrono::DateTime<chrono::Utc>>,
    bucket: &str,
) -> ArkDistillAnalyticsSummary {
    summarize_arkdistill_logs_window_with_pricing(
        rows,
        since,
        until,
        bucket,
        &ArkDistillPricingContext::default(),
    )
}

pub(super) fn summarize_arkdistill_logs_window_with_pricing(
    rows: &[crate::storage::entities::operational_log::Model],
    since: Option<chrono::DateTime<chrono::Utc>>,
    until: Option<chrono::DateTime<chrono::Utc>>,
    bucket: &str,
    pricing: &ArkDistillPricingContext,
) -> ArkDistillAnalyticsSummary {
    let parsed = rows
        .iter()
        .filter_map(parse_arkdistill_log)
        .filter(|row| since.is_none_or(|value| row.created_at >= value))
        .filter(|row| until.is_none_or(|value| row.created_at <= value))
        .collect::<Vec<_>>();

    let mut totals = ArkDistillAnalyticsTotals::default();
    let mut cost_sum = 0.0f64;
    let mut has_cost = false;
    let mut series: BTreeMap<String, ArkDistillAnalyticsPoint> = BTreeMap::new();
    let mut by_tool: HashMap<(String, Option<String>), ArkDistillToolSavingsRow> = HashMap::new();

    for row in parsed {
        totals.result_count += 1;
        totals.original_chars += row.original_chars;
        totals.distilled_chars += row.distilled_chars;
        totals.saved_chars += row.saved_chars;
        totals.estimated_original_tokens += row.estimated_original_tokens;
        totals.estimated_distilled_tokens += row.estimated_distilled_tokens;
        totals.estimated_saved_tokens += row.estimated_saved_tokens;
        let estimated_cost = row
            .estimated_prompt_cost_saved_usd
            .or_else(|| estimate_arkdistill_prompt_cost_saved_usd(&row, pricing));
        if let Some(cost) = estimated_cost {
            has_cost = true;
            cost_sum += cost;
        }

        let key = super::analytics_control::bucket_start(row.created_at, bucket).to_rfc3339();
        let point = series
            .entry(key.clone())
            .or_insert_with(|| ArkDistillAnalyticsPoint {
                bucket_start: key,
                ..ArkDistillAnalyticsPoint::default()
            });
        point.result_count += 1;
        point.original_chars += row.original_chars;
        point.distilled_chars += row.distilled_chars;
        point.saved_chars += row.saved_chars;
        point.estimated_saved_tokens += row.estimated_saved_tokens;
        add_optional_cost(&mut point.estimated_prompt_cost_saved_usd, estimated_cost);

        let tool_key = (row.tool_name.clone(), row.action.clone());
        let tool_row =
            by_tool
                .entry(tool_key.clone())
                .or_insert_with(|| ArkDistillToolSavingsRow {
                    tool_name: tool_key.0,
                    action: tool_key.1,
                    ..ArkDistillToolSavingsRow::default()
                });
        tool_row.result_count += 1;
        tool_row.saved_chars += row.saved_chars;
        tool_row.estimated_saved_tokens += row.estimated_saved_tokens;
        add_optional_cost(
            &mut tool_row.estimated_prompt_cost_saved_usd,
            estimated_cost,
        );
    }

    totals.average_reduction_ratio = if totals.original_chars > 0 {
        round4(totals.saved_chars as f64 / totals.original_chars as f64)
    } else {
        0.0
    };
    totals.savings_percent = round2(totals.average_reduction_ratio * 100.0);
    totals.estimated_prompt_cost_saved_usd = has_cost.then(|| round6(cost_sum));
    let mut by_tool = by_tool.into_values().collect::<Vec<_>>();
    by_tool.sort_by(|a, b| b.estimated_saved_tokens.cmp(&a.estimated_saved_tokens));

    ArkDistillAnalyticsSummary {
        totals,
        series: series.into_values().collect(),
        by_tool,
    }
}

fn parse_arkdistill_log(
    row: &crate::storage::entities::operational_log::Model,
) -> Option<ArkDistillParsedLog> {
    if row.event_type != crate::core::ARKDISTILL_EVENT_TYPE {
        return None;
    }
    let created_at = super::parse_utc_rfc3339(&row.created_at)?;
    let payload = row.payload.as_deref()?;
    let value = serde_json::from_str::<serde_json::Value>(payload).ok()?;
    let original_chars = json_i64(&value, "original_chars")
        .unwrap_or_default()
        .max(0);
    let distilled_chars = json_i64(&value, "distilled_chars")
        .unwrap_or_default()
        .max(0);
    let saved_chars = json_i64(&value, "saved_chars")
        .unwrap_or_else(|| original_chars.saturating_sub(distilled_chars))
        .max(0);
    let estimated_original_tokens = json_i64(&value, "estimated_original_tokens")
        .unwrap_or_else(|| estimate_tokens(original_chars))
        .max(0);
    let estimated_distilled_tokens = json_i64(&value, "estimated_distilled_tokens")
        .unwrap_or_else(|| estimate_tokens(distilled_chars))
        .max(0);
    let estimated_saved_tokens = json_i64(&value, "estimated_saved_tokens")
        .unwrap_or_else(|| estimated_original_tokens.saturating_sub(estimated_distilled_tokens))
        .max(0);
    if original_chars == 0 && estimated_saved_tokens == 0 {
        return None;
    }
    let tool_name = row
        .tool_name
        .clone()
        .or_else(|| json_string(&value, "primitive"))
        .unwrap_or_else(|| "unknown".to_string());
    Some(ArkDistillParsedLog {
        created_at,
        tool_name,
        action: json_string(&value, "action"),
        model_slot: row.model_slot.clone(),
        model_provider: json_string(&value, "model_provider"),
        model: json_string(&value, "model"),
        original_chars,
        distilled_chars,
        saved_chars,
        estimated_original_tokens,
        estimated_distilled_tokens,
        estimated_saved_tokens,
        estimated_prompt_cost_saved_usd: json_f64(&value, "estimated_prompt_cost_saved_usd"),
    })
}

fn estimate_arkdistill_prompt_cost_saved_usd(
    row: &ArkDistillParsedLog,
    pricing: &ArkDistillPricingContext,
) -> Option<f64> {
    if row.estimated_saved_tokens <= 0 {
        return None;
    }
    let context = arkdistill_model_context_for_row(row, pricing)?;
    super::analytics_control::estimate_cost_usd(
        &context.provider,
        &context.model,
        row.estimated_saved_tokens,
        0,
        &pricing.openrouter_prices,
    )
}

pub(super) fn arkdistill_logs_need_openrouter_pricing(
    rows: &[crate::storage::entities::operational_log::Model],
    pricing: &ArkDistillPricingContext,
) -> bool {
    rows.iter()
        .filter_map(parse_arkdistill_log)
        .filter(|row| row.estimated_prompt_cost_saved_usd.is_none())
        .any(|row| {
            arkdistill_model_context_for_row(&row, pricing)
                .is_some_and(|context| context.provider == "openrouter")
        })
}

fn arkdistill_model_context_for_row(
    row: &ArkDistillParsedLog,
    pricing: &ArkDistillPricingContext,
) -> Option<ArkDistillModelPricingContext> {
    let direct_context =
        row.model_provider
            .as_ref()
            .zip(row.model.as_ref())
            .map(|(provider, model)| ArkDistillModelPricingContext {
                provider: provider.trim().to_ascii_lowercase(),
                model: model.trim().to_string(),
            });
    let slot_context = row
        .model_slot
        .as_ref()
        .and_then(|slot| pricing.model_slots.get(slot.trim()))
        .cloned();
    direct_context
        .or(slot_context)
        .or_else(|| pricing.default_model.clone())
        .filter(|context| !context.provider.trim().is_empty() && !context.model.trim().is_empty())
}

fn add_optional_cost(target: &mut Option<f64>, value: Option<f64>) {
    if let Some(value) = value {
        *target = Some(round6(target.unwrap_or(0.0) + value));
    }
}

fn json_string(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn json_i64(value: &serde_json::Value, key: &str) -> Option<i64> {
    value.get(key).and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_u64().map(|value| value as i64))
    })
}

fn json_f64(value: &serde_json::Value, key: &str) -> Option<f64> {
    value
        .get(key)
        .and_then(|value| {
            value
                .as_f64()
                .or_else(|| value.as_i64().map(|value| value as f64))
        })
        .filter(|value| value.is_finite() && *value >= 0.0)
}

fn estimate_tokens(chars: i64) -> i64 {
    chars.max(0).saturating_add(3) / 4
}

fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn round6(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}
