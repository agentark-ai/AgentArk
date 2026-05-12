pub const INLINE_CHART_FENCE_LANGUAGE: &str = "agentark-chart";

pub fn app_deploy_inline_report_boundary() -> &'static str {
    "Do not use app_deploy for immediate chat reports, research syntheses, or analyses that merely need visual summaries; those should remain in the conversation response with inline tables/charts when useful."
}

pub fn inline_chart_block(chart: &serde_json::Value) -> String {
    let body = serde_json::to_string_pretty(chart).unwrap_or_else(|_| "{}".to_string());
    format!("```{}\n{}\n```", INLINE_CHART_FENCE_LANGUAGE, body)
}
