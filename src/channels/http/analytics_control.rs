use super::*;

pub(super) fn parse_range_param(input: Option<&String>) -> chrono::Duration {
    // Defaults and simple parsing (24h, 7d, 30d, 90d).
    let raw = input
        .map(|s| s.trim().to_ascii_lowercase())
        .unwrap_or_default();
    if raw.is_empty() {
        return chrono::Duration::hours(24);
    }
    if raw == "all" {
        return chrono::Duration::days(365 * 100);
    }
    let (num, unit) = raw.split_at(raw.len().saturating_sub(1));
    let n = num.parse::<i64>().unwrap_or(24);
    match unit {
        "h" => chrono::Duration::hours(n),
        "d" => chrono::Duration::days(n),
        "w" => chrono::Duration::weeks(n),
        _ => chrono::Duration::hours(24),
    }
}

pub(super) fn parse_analytics_datetime_param(
    input: Option<&String>,
) -> Option<chrono::DateTime<chrono::Utc>> {
    let raw = input.map(|s| s.trim()).unwrap_or("");
    if raw.is_empty() {
        return None;
    }
    parse_utc_rfc3339(raw)
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M")
                .ok()
                .map(|dt| {
                    chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(dt, chrono::Utc)
                })
        })
        .or_else(|| {
            chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d")
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
                .map(|dt| {
                    chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(dt, chrono::Utc)
                })
        })
}

pub(super) fn bucket_start(
    dt: chrono::DateTime<chrono::Utc>,
    bucket: &str,
) -> chrono::DateTime<chrono::Utc> {
    let naive = dt.naive_utc();
    match bucket {
        "day" => chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
            naive.date().and_hms_opt(0, 0, 0).unwrap(),
            chrono::Utc,
        ),
        "week" => {
            let date = naive.date();
            let weekday = date.weekday().num_days_from_monday() as i64;
            let start = date.and_hms_opt(0, 0, 0).unwrap() - chrono::Duration::days(weekday);
            chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(start, chrono::Utc)
        }
        _ => {
            // hour
            let start = naive
                .with_minute(0)
                .and_then(|x| x.with_second(0))
                .and_then(|x| x.with_nanosecond(0))
                .unwrap_or(naive);
            chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(start, chrono::Utc)
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct LlmAnalyticsTotals {
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
    request_count: i64,
    estimated_count: i64,
    cost_usd: Option<f64>,
}

#[derive(Debug, Serialize)]
pub(super) struct LlmAnalyticsPoint {
    bucket_start: String,
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
    request_count: i64,
    primary_prompt_tokens: i64,
    primary_completion_tokens: i64,
    primary_total_tokens: i64,
    primary_request_count: i64,
    helper_prompt_tokens: i64,
    helper_completion_tokens: i64,
    helper_total_tokens: i64,
    helper_request_count: i64,
    cost_usd: Option<f64>,
}

#[derive(Debug, Serialize)]
pub(super) struct LlmAnalyticsBreakdownRow {
    provider: String,
    model: String,
    channel: Option<String>,
    purpose: Option<String>,
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
    request_count: i64,
    cost_usd: Option<f64>,
}

#[derive(Debug, Clone)]
pub(super) struct OpenRouterModelPricing {
    pub(super) prompt_per_token: f64,
    pub(super) completion_per_token: f64,
    pub(super) request_per_request: f64,
}

#[derive(Debug, Clone)]
pub(super) struct OpenRouterPricingCacheEntry {
    fetched_at: Instant,
    prices: HashMap<String, OpenRouterModelPricing>,
}

pub(super) static OPENROUTER_PRICING_CACHE: OnceLock<RwLock<Option<OpenRouterPricingCacheEntry>>> =
    OnceLock::new();
pub(super) const OPENROUTER_PRICING_CACHE_TTL: Duration = Duration::from_secs(6 * 60 * 60);

pub(super) fn openrouter_pricing_cache() -> &'static RwLock<Option<OpenRouterPricingCacheEntry>> {
    OPENROUTER_PRICING_CACHE.get_or_init(|| RwLock::new(None))
}

/// Look up cost from the OpenRouter pricing cache. Used by agent.rs for trace cost estimation.
pub(crate) fn estimate_cost_from_pricing_cache(
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
) -> Option<f64> {
    let cache = openrouter_pricing_cache().try_read().ok()?;
    let entry = cache.as_ref()?;
    let pricing = find_openrouter_pricing(model, &entry.prices)?;
    Some(
        input_tokens as f64 * pricing.prompt_per_token
            + output_tokens as f64 * pricing.completion_per_token
            + pricing.request_per_request,
    )
}

pub(super) fn parse_openrouter_price_value(value: &serde_json::Value) -> Option<f64> {
    if let Some(v) = value.as_f64() {
        return Some(v);
    }
    if let Some(v) = value.as_i64() {
        return Some(v as f64);
    }
    value
        .as_str()
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v >= 0.0)
}

pub(super) fn add_openrouter_model_aliases(
    prices: &mut HashMap<String, OpenRouterModelPricing>,
    model: &str,
    pricing: OpenRouterModelPricing,
) {
    let lower = model.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return;
    }
    prices.insert(lower.clone(), pricing.clone());
    if let Some((_, tail)) = lower.rsplit_once('/') {
        prices.entry(tail.to_string()).or_insert(pricing.clone());
    }
    if let Some((_, tail)) = lower.rsplit_once(':') {
        prices.entry(tail.to_string()).or_insert(pricing);
    }
}

pub(super) async fn fetch_openrouter_pricing(
    api_key: Option<&str>,
) -> std::result::Result<HashMap<String, OpenRouterModelPricing>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| format!("Failed to build OpenRouter pricing client: {}", e))?;

    let mut req = client
        .get("https://openrouter.ai/api/v1/models")
        .header("Accept", "application/json")
        .header("HTTP-Referer", crate::branding::REPOSITORY_URL)
        .header("X-Title", crate::branding::PRODUCT_NAME);
    if let Some(key) = api_key.filter(|k| !k.trim().is_empty()) {
        req = req.bearer_auth(key.trim());
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("OpenRouter pricing request failed: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!(
            "OpenRouter pricing request failed with status {}",
            resp.status()
        ));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse OpenRouter pricing response: {}", e))?;

    let data = body
        .get("data")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "OpenRouter pricing payload missing data array".to_string())?;

    let mut prices: HashMap<String, OpenRouterModelPricing> = HashMap::new();
    for item in data {
        let model_id = item
            .get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());
        let Some(model_id) = model_id else {
            continue;
        };

        let pricing = item.get("pricing").and_then(|v| v.as_object());
        let Some(pricing) = pricing else {
            continue;
        };

        let prompt_price = pricing
            .get("prompt")
            .or_else(|| pricing.get("input"))
            .and_then(parse_openrouter_price_value);
        let completion_price = pricing
            .get("completion")
            .or_else(|| pricing.get("output"))
            .and_then(parse_openrouter_price_value);
        let request_price = pricing
            .get("request")
            .and_then(parse_openrouter_price_value)
            .unwrap_or(0.0);

        let (Some(prompt_per_token), Some(completion_per_token)) = (prompt_price, completion_price)
        else {
            continue;
        };

        add_openrouter_model_aliases(
            &mut prices,
            model_id,
            OpenRouterModelPricing {
                prompt_per_token,
                completion_per_token,
                request_per_request: request_price,
            },
        );
    }

    Ok(prices)
}

pub(super) async fn get_openrouter_pricing_cached(
    api_key: Option<&str>,
) -> HashMap<String, OpenRouterModelPricing> {
    let cache = openrouter_pricing_cache();
    let stale_prices = {
        let guard = cache.read().await;
        if let Some(entry) = guard.as_ref() {
            if entry.fetched_at.elapsed() < OPENROUTER_PRICING_CACHE_TTL {
                return entry.prices.clone();
            }
            Some(entry.prices.clone())
        } else {
            None
        }
    };

    match fetch_openrouter_pricing(api_key).await {
        Ok(prices) if !prices.is_empty() => {
            let mut guard = cache.write().await;
            *guard = Some(OpenRouterPricingCacheEntry {
                fetched_at: Instant::now(),
                prices: prices.clone(),
            });
            prices
        }
        Ok(_) => stale_prices.unwrap_or_default(),
        Err(e) => {
            tracing::warn!("OpenRouter pricing fetch failed: {}", e);
            stale_prices.unwrap_or_default()
        }
    }
}

pub(super) fn add_model_aliases(models: &mut HashSet<String>, model: &str) {
    let lower = model.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return;
    }
    models.insert(lower.clone());
    if let Some((_, tail)) = lower.rsplit_once('/') {
        models.insert(tail.to_string());
    }
    if let Some((_, tail)) = lower.rsplit_once(':') {
        models.insert(tail.to_string());
    }
}

pub(super) fn collect_openrouter_metadata(agent: &Agent) -> (Option<String>, HashSet<String>) {
    let mut openrouter_api_key: Option<String> = None;
    let mut openrouter_models: HashSet<String> = HashSet::new();

    let mut capture_provider = |provider: &LlmProvider| {
        if let LlmProvider::OpenAI {
            api_key,
            model,
            base_url,
        } = provider
        {
            if base_url
                .as_deref()
                .map(is_openrouter_base_url)
                .unwrap_or(false)
            {
                if openrouter_api_key.is_none() && !api_key.trim().is_empty() {
                    openrouter_api_key = Some(api_key.trim().to_string());
                }
                add_model_aliases(&mut openrouter_models, model);
            }
        }
    };

    capture_provider(&agent.config.llm);
    if let Some(fallback) = agent.config.llm_fallback.as_ref() {
        capture_provider(fallback);
    }
    for slot in &agent.config.model_pool.slots {
        capture_provider(&slot.provider);
    }

    if openrouter_api_key.is_none() {
        if let Ok(env_key) = std::env::var("OPENROUTER_API_KEY") {
            let trimmed = env_key.trim();
            if !trimmed.is_empty() {
                openrouter_api_key = Some(trimmed.to_string());
            }
        }
    }

    (openrouter_api_key, openrouter_models)
}

pub(super) fn normalize_analytics_provider(
    provider: &str,
    model: &str,
    openrouter_models: &HashSet<String>,
) -> String {
    let provider = provider.trim().to_ascii_lowercase();
    if provider == "openrouter" {
        return "openrouter".to_string();
    }
    if provider != "openai-compatible" {
        return provider;
    }

    let mut aliases: Vec<String> = Vec::new();
    let model_lower = model.trim().to_ascii_lowercase();
    if !model_lower.is_empty() {
        aliases.push(model_lower.clone());
        if let Some((_, tail)) = model_lower.rsplit_once('/') {
            aliases.push(tail.to_string());
        }
        if let Some((_, tail)) = model_lower.rsplit_once(':') {
            aliases.push(tail.to_string());
        }
    }

    if aliases.into_iter().any(|m| openrouter_models.contains(&m)) {
        "openrouter".to_string()
    } else {
        "openai-compatible".to_string()
    }
}

pub(super) fn find_openrouter_pricing<'a>(
    model: &str,
    prices: &'a HashMap<String, OpenRouterModelPricing>,
) -> Option<&'a OpenRouterModelPricing> {
    let lower = model.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return None;
    }
    if let Some(p) = prices.get(&lower) {
        return Some(p);
    }
    if let Some((_, tail)) = lower.rsplit_once('/') {
        if let Some(p) = prices.get(tail) {
            return Some(p);
        }
    }
    if let Some((_, tail)) = lower.rsplit_once(':') {
        if let Some(p) = prices.get(tail) {
            return Some(p);
        }
    }
    if !lower.contains('/') {
        if let Some((_, p)) = prices
            .iter()
            .find(|(id, _)| id.ends_with(&format!("/{}", lower)))
        {
            return Some(p);
        }
    }
    None
}

pub(super) fn estimate_cost_usd(
    provider: &str,
    model: &str,
    prompt: i64,
    completion: i64,
    openrouter_prices: &HashMap<String, OpenRouterModelPricing>,
) -> Option<f64> {
    let p = provider.trim().to_ascii_lowercase();
    if p == "ollama" {
        return Some(0.0);
    }
    if p == "openrouter" {
        if let Some(pricing) = find_openrouter_pricing(model, openrouter_prices) {
            let prompt_tokens = prompt.max(0) as f64;
            let completion_tokens = completion.max(0) as f64;
            return Some(
                prompt_tokens * pricing.prompt_per_token
                    + completion_tokens * pricing.completion_per_token
                    + pricing.request_per_request,
            );
        }
    }
    // No hardcoded fallback — if pricing isn't in the cache, return None.
    None
}

pub(super) fn resolve_usage_row_cost_usd(
    row: &crate::storage::entities::llm_usage::Model,
    provider: &str,
    openrouter_prices: &HashMap<String, OpenRouterModelPricing>,
) -> Option<f64> {
    row.cost_usd.or_else(|| {
        estimate_cost_usd(
            provider,
            &row.model,
            row.prompt_tokens as i64,
            row.completion_tokens as i64,
            openrouter_prices,
        )
    })
}

pub(super) fn analytics_purpose_kind(channel: &str, purpose: &str) -> &'static str {
    let channel = channel.trim().to_ascii_lowercase();
    let purpose = purpose.trim().to_ascii_lowercase();
    if purpose.is_empty() {
        return "primary";
    }

    let helper_exact = [
        "title",
        "user_fact_quick_capture",
        "user_fact_memory_capture",
        "argument_inference",
        "custom_condition",
    ];

    if helper_exact.contains(&purpose.as_str())
        || purpose.contains("classifier")
        || purpose.ends_with("_selector")
        || purpose.contains("request_shape")
        || purpose.contains("memory_capture")
        || purpose.contains("argument_inference")
        || purpose.contains("custom_condition")
    {
        return "helper";
    }

    if matches!(channel.as_str(), "system" | "watcher" | "automation")
        && !matches!(
            purpose.as_str(),
            "chat" | "chat_tool_followup" | "chat_tool_synthesis" | "chat_tool_repair"
        )
    {
        return "helper";
    }

    "primary"
}

pub(super) async fn llm_analytics_endpoint(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let range = parse_range_param(params.get("range"));
    let bucket = params
        .get("bucket")
        .map(|s| s.trim().to_ascii_lowercase())
        .unwrap_or("hour".to_string());
    let bucket = match bucket.as_str() {
        "hour" | "day" | "week" => bucket,
        _ => "hour".to_string(),
    };

    let now = chrono::Utc::now();
    let mut since = parse_analytics_datetime_param(params.get("from")).unwrap_or(now - range);
    let mut until = parse_analytics_datetime_param(params.get("to")).unwrap_or(now);
    if since > until {
        std::mem::swap(&mut since, &mut until);
    }
    let since_rfc3339 = since.to_rfc3339();
    let until_rfc3339 = until.to_rfc3339();

    let agent = state.agent.read().await;
    let (rows, truncated) = match agent
        .storage
        .list_llm_usage_window_complete(&since_rfc3339, &until_rfc3339)
        .await
    {
        Ok(result) => result,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
                .into_response();
        }
    };
    let (openrouter_api_key, openrouter_models) = collect_openrouter_metadata(&agent);
    drop(agent);

    let has_openrouter_like_rows = rows.iter().any(|r| {
        let provider = r.provider.trim().to_ascii_lowercase();
        (provider == "openrouter" || provider == "openai-compatible") && r.cost_usd.is_none()
    });
    let openrouter_prices = if has_openrouter_like_rows {
        get_openrouter_pricing_cached(openrouter_api_key.as_deref()).await
    } else {
        HashMap::new()
    };

    use std::collections::BTreeMap;
    let mut series: BTreeMap<String, LlmAnalyticsPoint> = BTreeMap::new();
    let mut by_model: std::collections::HashMap<(String, String), LlmAnalyticsBreakdownRow> =
        std::collections::HashMap::new();
    let mut by_channel: std::collections::HashMap<String, LlmAnalyticsBreakdownRow> =
        std::collections::HashMap::new();
    let mut by_purpose: std::collections::HashMap<String, LlmAnalyticsBreakdownRow> =
        std::collections::HashMap::new();

    let mut totals = LlmAnalyticsTotals {
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
        request_count: 0,
        estimated_count: 0,
        cost_usd: Some(0.0),
    };

    for r in rows {
        let dt = parse_utc_rfc3339(&r.created_at).unwrap_or_else(chrono::Utc::now);
        if dt < since || dt > until {
            continue;
        }
        let bstart = bucket_start(dt, &bucket);
        let key = bstart.to_rfc3339();
        let provider = normalize_analytics_provider(&r.provider, &r.model, &openrouter_models);
        let pt = r.prompt_tokens as i64;
        let ct = r.completion_tokens as i64;
        let tt = r.total_tokens as i64;
        let cost = resolve_usage_row_cost_usd(&r, &provider, &openrouter_prices);

        totals.prompt_tokens += pt;
        totals.completion_tokens += ct;
        totals.total_tokens += tt;
        totals.request_count += 1;
        if r.estimated {
            totals.estimated_count += 1;
        }
        match (&mut totals.cost_usd, cost) {
            (Some(sum), Some(c)) => *sum += c,
            (Some(_), None) => totals.cost_usd = None,
            (None, _) => {}
        }

        let entry = series
            .entry(key.clone())
            .or_insert_with(|| LlmAnalyticsPoint {
                bucket_start: key.clone(),
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                request_count: 0,
                primary_prompt_tokens: 0,
                primary_completion_tokens: 0,
                primary_total_tokens: 0,
                primary_request_count: 0,
                helper_prompt_tokens: 0,
                helper_completion_tokens: 0,
                helper_total_tokens: 0,
                helper_request_count: 0,
                cost_usd: Some(0.0),
            });
        entry.prompt_tokens += pt;
        entry.completion_tokens += ct;
        entry.total_tokens += tt;
        entry.request_count += 1;
        match analytics_purpose_kind(&r.channel, &r.purpose) {
            "helper" => {
                entry.helper_prompt_tokens += pt;
                entry.helper_completion_tokens += ct;
                entry.helper_total_tokens += tt;
                entry.helper_request_count += 1;
            }
            _ => {
                entry.primary_prompt_tokens += pt;
                entry.primary_completion_tokens += ct;
                entry.primary_total_tokens += tt;
                entry.primary_request_count += 1;
            }
        }
        match (&mut entry.cost_usd, cost) {
            (Some(sum), Some(c)) => *sum += c,
            (Some(_), None) => entry.cost_usd = None,
            (None, _) => {}
        }

        let mk = (provider.clone(), r.model.clone());
        let model_row = by_model
            .entry(mk.clone())
            .or_insert_with(|| LlmAnalyticsBreakdownRow {
                provider: mk.0.clone(),
                model: mk.1.clone(),
                channel: None,
                purpose: None,
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                request_count: 0,
                cost_usd: Some(0.0),
            });
        model_row.prompt_tokens += pt;
        model_row.completion_tokens += ct;
        model_row.total_tokens += tt;
        model_row.request_count += 1;
        match (&mut model_row.cost_usd, cost) {
            (Some(sum), Some(c)) => *sum += c,
            (Some(_), None) => model_row.cost_usd = None,
            (None, _) => {}
        }

        let ch = r.channel.clone();
        let ch_row = by_channel
            .entry(ch.clone())
            .or_insert_with(|| LlmAnalyticsBreakdownRow {
                provider: "".to_string(),
                model: "".to_string(),
                channel: Some(ch.clone()),
                purpose: None,
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                request_count: 0,
                cost_usd: Some(0.0),
            });
        ch_row.prompt_tokens += pt;
        ch_row.completion_tokens += ct;
        ch_row.total_tokens += tt;
        ch_row.request_count += 1;
        match (&mut ch_row.cost_usd, cost) {
            (Some(sum), Some(c)) => *sum += c,
            (Some(_), None) => ch_row.cost_usd = None,
            (None, _) => {}
        }

        let pur = r.purpose.clone();
        let pur_row = by_purpose
            .entry(pur.clone())
            .or_insert_with(|| LlmAnalyticsBreakdownRow {
                provider: "".to_string(),
                model: "".to_string(),
                channel: None,
                purpose: Some(pur.clone()),
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                request_count: 0,
                cost_usd: Some(0.0),
            });
        pur_row.prompt_tokens += pt;
        pur_row.completion_tokens += ct;
        pur_row.total_tokens += tt;
        pur_row.request_count += 1;
        match (&mut pur_row.cost_usd, cost) {
            (Some(sum), Some(c)) => *sum += c,
            (Some(_), None) => pur_row.cost_usd = None,
            (None, _) => {}
        }
    }

    let mut by_model_list: Vec<LlmAnalyticsBreakdownRow> = by_model.into_values().collect();
    by_model_list.sort_by(|a, b| b.total_tokens.cmp(&a.total_tokens));
    let mut by_channel_list: Vec<LlmAnalyticsBreakdownRow> = by_channel.into_values().collect();
    by_channel_list.sort_by(|a, b| b.total_tokens.cmp(&a.total_tokens));
    let mut by_purpose_list: Vec<LlmAnalyticsBreakdownRow> = by_purpose.into_values().collect();
    by_purpose_list.sort_by(|a, b| b.total_tokens.cmp(&a.total_tokens));

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "range": {
                "since": since_rfc3339,
                "until": until_rfc3339,
                "bucket": bucket,
                "truncated": truncated,
            },
            "totals": totals,
            "truncated": truncated,
            "series": series.into_values().collect::<Vec<_>>(),
            "by_model": by_model_list,
            "by_channel": by_channel_list,
            "by_purpose": by_purpose_list,
        })),
    )
        .into_response()
}
