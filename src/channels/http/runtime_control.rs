use super::*;

/// Health check endpoint
pub(super) fn heartbeat_recent_with_threshold(value: Option<&str>, max_age_secs: i64) -> bool {
    value
        .and_then(parse_utc_rfc3339)
        .map(|ts| (chrono::Utc::now() - ts).num_seconds() <= max_age_secs)
        .unwrap_or(false)
}

#[derive(Debug, Clone, Copy)]
struct RuntimeProcSample {
    at: Instant,
    cpu_total: u64,
    cpu_idle: u64,
    disk_read_sectors: u64,
    disk_write_sectors: u64,
}

static LAST_RUNTIME_PROC_SAMPLE: once_cell::sync::Lazy<
    parking_lot::Mutex<Option<RuntimeProcSample>>,
> = once_cell::sync::Lazy::new(|| parking_lot::Mutex::new(None));

fn collect_local_runtime_health(uptime_seconds: u64) -> RuntimeHealthResponse {
    let memory = read_linux_memory_pressure();
    let sample = read_linux_proc_sample();
    let (cpu_percent, disk_read_bytes_per_sec, disk_write_bytes_per_sec) = sample
        .map(update_runtime_proc_rates)
        .unwrap_or((None, None, None));

    RuntimeHealthResponse {
        uptime_seconds,
        cpu_percent,
        ram_percent: memory.map(|item| item.2),
        memory_pressure_percent: memory.map(|item| item.2),
        memory_used_bytes: memory.map(|item| item.0),
        memory_total_bytes: memory.map(|item| item.1),
        memory_source: memory.map(|item| item.3.to_string()),
        memory_container_count: None,
        disk_read_bytes_per_sec,
        disk_write_bytes_per_sec,
        temperature_celsius: read_linux_temperature_celsius(),
        load_average_1m: read_linux_load_average_1m(),
        sampled_at: chrono::Utc::now().to_rfc3339(),
    }
}

async fn collect_status_runtime_health(
    state: &AppState,
    uptime_seconds: u64,
) -> RuntimeHealthResponse {
    let mut health = collect_local_runtime_health(uptime_seconds);
    if let Some(stats) = read_docker_stack_memory_stats(state).await {
        health.memory_used_bytes = Some(stats.memory_used_bytes);
        health.memory_total_bytes = stats.memory_total_bytes;
        health.memory_pressure_percent = stats.memory_pressure_percent;
        health.ram_percent = stats.memory_pressure_percent;
        health.memory_source = Some(stats.source);
        health.memory_container_count = Some(stats.container_count);
        health.sampled_at = stats.sampled_at;
    }
    health
}

async fn read_docker_stack_memory_stats(
    state: &AppState,
) -> Option<crate::clients::StackMemoryStatsResponse> {
    if state.server_role != HttpServerRole::ControlPlane {
        return None;
    }
    let executor = state
        .executor_client
        .as_ref()
        .cloned()
        .or_else(|| build_executor_client().ok().flatten())?;
    match tokio::time::timeout(
        std::time::Duration::from_secs(3),
        executor.stack_memory_stats(),
    )
    .await
    {
        Ok(Ok(stats)) if stats.status == "ok" && stats.container_count > 0 => Some(stats),
        Ok(Err(error)) => {
            tracing::debug!("Docker stack memory stats unavailable: {}", error);
            None
        }
        Err(_) => {
            tracing::debug!("Docker stack memory stats timed out");
            None
        }
        _ => None,
    }
}

fn update_runtime_proc_rates(sample: RuntimeProcSample) -> (Option<f64>, Option<f64>, Option<f64>) {
    let mut previous = LAST_RUNTIME_PROC_SAMPLE.lock();
    let last = previous.replace(sample);
    let Some(last) = last else {
        return (None, None, None);
    };

    let elapsed_secs = sample.at.duration_since(last.at).as_secs_f64();
    if elapsed_secs <= 0.05 {
        return (None, None, None);
    }

    let cpu_total_delta = sample.cpu_total.saturating_sub(last.cpu_total);
    let cpu_idle_delta = sample.cpu_idle.saturating_sub(last.cpu_idle);
    let cpu_percent = if cpu_total_delta > 0 {
        let active = cpu_total_delta.saturating_sub(cpu_idle_delta) as f64;
        Some(round_1(
            (active / cpu_total_delta as f64 * 100.0).clamp(0.0, 100.0),
        ))
    } else {
        None
    };

    let read_bps = sample
        .disk_read_sectors
        .checked_sub(last.disk_read_sectors)
        .map(|sectors| round_1(sectors as f64 * 512.0 / elapsed_secs));
    let write_bps = sample
        .disk_write_sectors
        .checked_sub(last.disk_write_sectors)
        .map(|sectors| round_1(sectors as f64 * 512.0 / elapsed_secs));

    (cpu_percent, read_bps, write_bps)
}

fn read_linux_proc_sample() -> Option<RuntimeProcSample> {
    let (cpu_total, cpu_idle) = read_linux_cpu_counters()?;
    let (disk_read_sectors, disk_write_sectors) = read_linux_disk_sectors().unwrap_or((0, 0));
    Some(RuntimeProcSample {
        at: Instant::now(),
        cpu_total,
        cpu_idle,
        disk_read_sectors,
        disk_write_sectors,
    })
}

fn read_linux_cpu_counters() -> Option<(u64, u64)> {
    let content = std::fs::read_to_string("/proc/stat").ok()?;
    let line = content.lines().find(|line| line.starts_with("cpu "))?;
    let values = line
        .split_whitespace()
        .skip(1)
        .filter_map(|part| part.parse::<u64>().ok())
        .collect::<Vec<_>>();
    if values.len() < 4 {
        return None;
    }
    let idle = values[3].saturating_add(values.get(4).copied().unwrap_or(0));
    let total = values.iter().copied().sum::<u64>();
    Some((total, idle))
}

fn read_linux_memory_pressure() -> Option<(u64, u64, f64, &'static str)> {
    if let Some(memory) = read_linux_cgroup_memory_pressure() {
        return Some((memory.0, memory.1, memory.2, "cgroup"));
    }

    let content = std::fs::read_to_string("/proc/meminfo").ok()?;
    let mut total_kb = None;
    let mut available_kb = None;
    let mut free_kb = None;

    for line in content.lines() {
        let mut parts = line.split_whitespace();
        let key = parts.next().unwrap_or_default().trim_end_matches(':');
        let value = parts.next().and_then(|part| part.parse::<u64>().ok());
        match key {
            "MemTotal" => total_kb = value,
            "MemAvailable" => available_kb = value,
            "MemFree" => free_kb = value,
            _ => {}
        }
    }

    let total = total_kb?;
    if total == 0 {
        return None;
    }
    let available = available_kb.or(free_kb).unwrap_or(0).min(total);
    let used = total.saturating_sub(available);
    let percent = round_1((used as f64 / total as f64 * 100.0).clamp(0.0, 100.0));
    Some((
        used.saturating_mul(1024),
        total.saturating_mul(1024),
        percent,
        "proc_meminfo",
    ))
}

fn read_linux_cgroup_memory_pressure() -> Option<(u64, u64, f64)> {
    fn read_u64_file(path: &str) -> Option<u64> {
        let raw = std::fs::read_to_string(path).ok()?;
        let trimmed = raw.trim();
        if trimmed.eq_ignore_ascii_case("max") {
            return None;
        }
        trimmed.parse::<u64>().ok()
    }

    fn valid_limit(value: u64) -> Option<u64> {
        if value == 0 || value >= (1_u64 << 60) {
            None
        } else {
            Some(value)
        }
    }

    let usage = read_u64_file("/sys/fs/cgroup/memory.current")
        .or_else(|| read_u64_file("/sys/fs/cgroup/memory/memory.usage_in_bytes"))?;
    let limit = read_u64_file("/sys/fs/cgroup/memory.max")
        .or_else(|| read_u64_file("/sys/fs/cgroup/memory/memory.limit_in_bytes"))
        .and_then(valid_limit)?;
    if limit == 0 {
        return None;
    }
    let used = usage.min(limit);
    let percent = round_1((used as f64 / limit as f64 * 100.0).clamp(0.0, 100.0));
    Some((used, limit, percent))
}

fn read_linux_disk_sectors() -> Option<(u64, u64)> {
    let content = std::fs::read_to_string("/proc/diskstats").ok()?;
    let mut read_sectors = 0_u64;
    let mut write_sectors = 0_u64;
    let mut found = false;

    for line in content.lines() {
        let parts = line.split_whitespace().collect::<Vec<_>>();
        if parts.len() < 10 {
            continue;
        }
        let name = parts[2];
        if name.starts_with("loop") || name.starts_with("ram") || name.starts_with("fd") {
            continue;
        }
        let read = parts[5].parse::<u64>().ok();
        let written = parts[9].parse::<u64>().ok();
        if let (Some(read), Some(written)) = (read, written) {
            read_sectors = read_sectors.saturating_add(read);
            write_sectors = write_sectors.saturating_add(written);
            found = true;
        }
    }

    found.then_some((read_sectors, write_sectors))
}

fn read_linux_temperature_celsius() -> Option<f64> {
    let entries = std::fs::read_dir("/sys/class/thermal").ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("thermal_zone") {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(entry.path().join("temp")) else {
            continue;
        };
        let Ok(value) = raw.trim().parse::<f64>() else {
            continue;
        };
        let celsius = if value.abs() > 1000.0 {
            value / 1000.0
        } else {
            value
        };
        if (-40.0..=130.0).contains(&celsius) {
            return Some(round_1(celsius));
        }
    }
    None
}

fn read_linux_load_average_1m() -> Option<f64> {
    let content = std::fs::read_to_string("/proc/loadavg").ok()?;
    content
        .split_whitespace()
        .next()
        .and_then(|value| value.parse::<f64>().ok())
        .map(round_2)
}

fn round_1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn round_2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

pub(super) async fn build_runtime_health_payload(
    state: &AppState,
    readiness_mode: bool,
) -> (StatusCode, serde_json::Value) {
    let storage = { state.agent.read().await.storage.clone() };
    let storage_ok = storage.get("__health_probe").await.is_ok();
    let expected_migration_version = storage.expected_migration_version();
    let migration_version = storage.latest_migration_version().await.ok().flatten();
    let migration_tracked = migration_version.is_some();
    let table_names = storage.database_table_names().await.ok();
    let database_size_bytes = storage.database_size_bytes().await.ok().flatten();
    let lease_status = storage
        .lease_status_summary()
        .await
        .ok()
        .unwrap_or_default();
    let housekeeping = storage.housekeeping_status().await.ok().unwrap_or_default();
    let required_tables = [
        "approval_log",
        "automation_runs",
        "automation_supervisor_states",
        "background_sessions",
        "conversations",
        "execution_runs",
        "kv_store",
        "messages",
        "notifications",
        "run_checkpoints",
        "tasks",
        "tool_attempts",
        "watchers",
    ];
    let missing_tables = table_names
        .as_ref()
        .map(|tables| {
            let set = tables.iter().cloned().collect::<HashSet<_>>();
            required_tables
                .iter()
                .filter(|table| !set.contains(**table))
                .map(|table| (*table).to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| {
            required_tables
                .iter()
                .map(|table| (*table).to_string())
                .collect()
        });
    let schema_ok = missing_tables.is_empty();
    let migration_current = migration_version
        .map(|value| value == expected_migration_version)
        .unwrap_or(schema_ok);

    let scheduler_heartbeat = storage
        .get(crate::sentinel::SENTINEL_SCHEDULER_HEARTBEAT_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| String::from_utf8(raw).ok());
    let watcher_heartbeat = storage
        .get(crate::sentinel::SENTINEL_WATCHER_HEARTBEAT_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| String::from_utf8(raw).ok());
    let integration_sync_heartbeat = storage
        .get(crate::sentinel::SENTINEL_INTEGRATION_SYNC_HEARTBEAT_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| String::from_utf8(raw).ok());
    let approval_expiry_heartbeat = storage
        .get(crate::sentinel::SENTINEL_APPROVAL_EXPIRY_HEARTBEAT_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| String::from_utf8(raw).ok());
    let arkpulse_heartbeat = storage
        .get(crate::sentinel::SENTINEL_ARKPULSE_HEARTBEAT_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| String::from_utf8(raw).ok());
    let auto_analysis_heartbeat = storage
        .get(crate::sentinel::SENTINEL_AUTO_ANALYSIS_HEARTBEAT_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| String::from_utf8(raw).ok());

    let sentinel_config = crate::sentinel::SentinelConfig::default();
    let scheduler_loop_ok = if state.server_role == HttpServerRole::ControlPlane {
        heartbeat_recent_with_threshold(
            scheduler_heartbeat.as_deref(),
            (sentinel_config.scheduler_interval as i64 * 3).max(5 * 60),
        )
    } else {
        true
    };
    let watcher_loop_ok = if state.server_role == HttpServerRole::ControlPlane {
        heartbeat_recent_with_threshold(
            watcher_heartbeat.as_deref(),
            (sentinel_config.watcher_interval as i64 * 3).max(5 * 60),
        )
    } else {
        true
    };
    let integration_sync_loop_ok = if state.server_role == HttpServerRole::ControlPlane
        && sentinel_config.integration_sync_interval > 0
    {
        heartbeat_recent_with_threshold(
            integration_sync_heartbeat.as_deref(),
            (sentinel_config.integration_sync_interval as i64 * 3).max(8 * 60),
        )
    } else {
        true
    };
    let approval_expiry_loop_ok = if state.server_role == HttpServerRole::ControlPlane {
        heartbeat_recent_with_threshold(
            approval_expiry_heartbeat.as_deref(),
            (sentinel_config.approval_expiry_interval as i64 * 3).max(10 * 60),
        )
    } else {
        true
    };
    let arkpulse_loop_ok = if state.server_role == HttpServerRole::ControlPlane
        && sentinel_config.pulse_interval > 0
    {
        heartbeat_recent_with_threshold(
            arkpulse_heartbeat.as_deref(),
            (sentinel_config.pulse_interval as i64 * 3).max(60 * 60),
        )
    } else {
        true
    };
    let auto_analysis_loop_ok = if state.server_role == HttpServerRole::ControlPlane
        && sentinel_config.auto_analysis_interval > 0
    {
        heartbeat_recent_with_threshold(
            auto_analysis_heartbeat.as_deref(),
            (sentinel_config.auto_analysis_interval as i64 * 3).max(20 * 60),
        )
    } else {
        true
    };

    let (
        embedding_client,
        playwright_url,
        public_app_base_url_configured,
        startup_issues_handle,
        docker_ok,
        active_container_count,
        container_reaper_status,
    ) = {
        let agent = state.agent.read().await;
        (
            agent.embedding_client.clone(),
            agent.config.browser.bridge_url.clone(),
            agent
                .config
                .public_apps
                .base_url
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_some(),
            agent.startup_issues_handle(),
            if state.server_role == HttpServerRole::ControlPlane
                && crate::runtime::ActionRuntime::should_manage_local_sandbox_containers()
            {
                agent.runtime.docker_available().await
            } else {
                true
            },
            agent.runtime.active_container_count().await,
            agent.runtime.container_reaper_status().await,
        )
    };
    let startup_issues = startup_issues_handle.read().await.clone();
    let blocking_startup_issue_count = startup_issues
        .iter()
        .filter(|issue| issue.blocks_readiness())
        .count();

    let health_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .ok();
    let pgvector_retrieval_ok = if state.server_role == HttpServerRole::ControlPlane {
        let pgvector_ok = storage.pgvector_health_check().await.is_ok();
        let embeddings_ok = if let Some(client) = embedding_client.as_ref() {
            client.health_check().await.is_ok()
        } else {
            false
        };
        pgvector_ok && embeddings_ok
    } else {
        true
    };
    let playwright_ok = if state.server_role == HttpServerRole::ControlPlane {
        if let Some(client) = health_client.as_ref() {
            client
                .get(format!("{}/health", playwright_url.trim_end_matches('/')))
                .send()
                .await
                .map(|resp| resp.status().is_success())
                .unwrap_or(false)
        } else {
            false
        }
    } else {
        true
    };
    let whatsapp_active = state.whatsapp_bridge.read().await.active;
    let tunnel_active = state.tunnel.read().await.active;
    let restore = state.app_registry.restore_snapshot().await;
    let restore_ready = if state.server_role == HttpServerRole::ControlPlane {
        !restore.active && restore.pending == 0 && restore.degraded == 0
    } else {
        true
    };
    let public_app_origin_ok = if state.server_role == HttpServerRole::ControlPlane
        && state.deployment_mode == DeploymentMode::InternetFacing
    {
        public_app_base_url_configured
    } else {
        true
    };

    let healthy = storage_ok
        && schema_ok
        && migration_current
        && pgvector_retrieval_ok
        && docker_ok
        && playwright_ok
        && public_app_origin_ok
        && scheduler_loop_ok
        && watcher_loop_ok;
    let ready = healthy
        && integration_sync_loop_ok
        && approval_expiry_loop_ok
        && arkpulse_loop_ok
        && auto_analysis_loop_ok
        && restore_ready
        && blocking_startup_issue_count == 0;
    let overall_ok = if readiness_mode { ready } else { true };
    let status_text = if readiness_mode {
        if ready {
            "ok"
        } else {
            "not_ready"
        }
    } else if healthy {
        "ok"
    } else {
        "degraded"
    };

    (
        if overall_ok {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        serde_json::json!({
            "status": status_text,
            "mode": if readiness_mode { "readiness" } else { "health" },
            "ready": ready,
            "server_role": match state.server_role {
                HttpServerRole::ControlPlane => "control_plane",
                HttpServerRole::PublicApps => "public_apps",
            },
            "deployment_mode": state.deployment_mode.as_str(),
            "database": {
                "kind": "postgres",
                "connected": storage_ok,
                "migration_version": migration_version,
                "expected_migration_version": expected_migration_version,
                "migration_tracking": if migration_tracked { "tracked" } else { "bootstrap_only" },
                "migration_current": migration_current,
                "schema_ok": schema_ok,
                "missing_tables": missing_tables,
                "table_count": table_names.as_ref().map(|tables| tables.len()).unwrap_or(0),
                "size_bytes": database_size_bytes,
                "lease_status": lease_status,
            },
            "startup": {
                "issue_count": startup_issues.len(),
                "blocking_issue_count": blocking_startup_issue_count,
                "issues": startup_issues,
            },
            "apps": {
                "restore": restore,
            },
            "runtime": {
                "active_container_count": active_container_count,
                "container_reaper": container_reaper_status,
            },
            "housekeeping": housekeeping,
            "checks": {
                "storage": storage_ok,
                "postgres_connected": storage_ok,
                "schema_ok": schema_ok,
                "migration_current": migration_current,
                "postgres_pgvector_retrieval": pgvector_retrieval_ok,
                "docker": docker_ok,
                "playwright_bridge": playwright_ok,
                "whatsapp_bridge": whatsapp_active,
                "tunnel": tunnel_active,
                "scheduler_loop": scheduler_loop_ok,
                "watcher_loop": watcher_loop_ok,
                "integration_sync_loop": integration_sync_loop_ok,
                "approval_expiry_loop": approval_expiry_loop_ok,
                "arkpulse_loop": arkpulse_loop_ok,
                "auto_analysis_loop": auto_analysis_loop_ok,
                "app_restore_ready": restore_ready,
                "public_app_origin_ready": public_app_origin_ok,
            },
            "heartbeats": {
                "scheduler": scheduler_heartbeat,
                "watcher": watcher_heartbeat,
                "integration_sync": integration_sync_heartbeat,
                "approval_expiry": approval_expiry_heartbeat,
                "arkpulse": arkpulse_heartbeat,
                "auto_analysis": auto_analysis_heartbeat,
            }
        }),
    )
}

pub(super) fn timed_out_health_payload(readiness_mode: bool) -> (StatusCode, serde_json::Value) {
    (
        if readiness_mode {
            StatusCode::SERVICE_UNAVAILABLE
        } else {
            StatusCode::OK
        },
        serde_json::json!({
            "status": if readiness_mode { "not_ready" } else { "degraded" },
            "mode": if readiness_mode { "readiness" } else { "health" },
            "ready": false,
            "checks": {
                "health_probe_timeout": true,
            },
            "error": "Health probe timed out before dependency checks completed.",
        }),
    )
}

pub(super) async fn health(State(state): State<AppState>) -> Response {
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "mode": "health",
            "ready": true,
            "server_role": match state.server_role {
                HttpServerRole::ControlPlane => "control_plane",
                HttpServerRole::PublicApps => "public_apps",
            },
            "deployment_mode": state.deployment_mode.as_str(),
        })),
    )
        .into_response()
}

pub(super) async fn readiness(State(state): State<AppState>) -> Response {
    let (status, payload) = match tokio::time::timeout(
        HEALTH_PROBE_TIMEOUT,
        build_runtime_health_payload(&state, true),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => timed_out_health_payload(true),
    };
    (status, Json(payload)).into_response()
}

pub(super) async fn metrics(State(state): State<AppState>) -> Response {
    let (queue_depth, active_tasks) = {
        let tasks = state.tasks.read().await;
        let all = tasks.all();
        let active = all
            .iter()
            .filter(|task| {
                matches!(
                    task.status,
                    TaskStatus::Pending
                        | TaskStatus::AwaitingApproval
                        | TaskStatus::ExpiredNeedsReapproval
                        | TaskStatus::Paused
                        | TaskStatus::InProgress
                )
            })
            .count();
        (all.len(), active)
    };
    let (active_container_count, reaper_status) = {
        let agent = state.agent.read().await;
        (
            agent.runtime.active_container_count().await,
            agent.runtime.container_reaper_status().await,
        )
    };
    let mut extra_metrics = Vec::new();
    extra_metrics.push("# HELP agentark_task_queue_depth Total tasks currently stored in the in-memory scheduler queue.".to_string());
    extra_metrics.push("# TYPE agentark_task_queue_depth gauge".to_string());
    extra_metrics.push(format!("agentark_task_queue_depth {}", queue_depth));
    extra_metrics.push("# HELP agentark_active_tasks Tasks currently pending or otherwise active in the scheduler queue.".to_string());
    extra_metrics.push("# TYPE agentark_active_tasks gauge".to_string());
    extra_metrics.push(format!("agentark_active_tasks {}", active_tasks));
    extra_metrics.push("# HELP agentark_runtime_active_containers Active sandbox containers reported by the runtime.".to_string());
    extra_metrics.push("# TYPE agentark_runtime_active_containers gauge".to_string());
    extra_metrics.push(format!(
        "agentark_runtime_active_containers {}",
        active_container_count
    ));
    extra_metrics.push("# HELP agentark_container_reaper_last_removed_count Containers removed by the most recent orphan-container sweep.".to_string());
    extra_metrics.push("# TYPE agentark_container_reaper_last_removed_count gauge".to_string());
    extra_metrics.push(format!(
        "agentark_container_reaper_last_removed_count {}",
        reaper_status.last_removed_count
    ));
    extra_metrics.push("# HELP agentark_container_reaper_total_removed_count Cumulative orphan sandbox containers removed by the runtime reaper.".to_string());
    extra_metrics.push("# TYPE agentark_container_reaper_total_removed_count counter".to_string());
    extra_metrics.push(format!(
        "agentark_container_reaper_total_removed_count {}",
        reaper_status.total_removed_count
    ));
    let body = crate::metrics::render_prometheus(&extra_metrics);
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
        .into_response()
}

pub(super) async fn get_run(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    let storage = { state.agent.read().await.storage.clone() };
    match storage.load_execution_run(&id).await {
        Ok(Some(run)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "run": run,
            })),
        )
            .into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "Run not found",
                "run_id": id,
            })),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Failed to load run: {}", error),
                "run_id": id,
            })),
        )
            .into_response(),
    }
}

pub(super) async fn stream_run_events(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let since_seq = params
        .get("since_seq")
        .and_then(|value| value.parse::<u64>().ok());
    let agent = state.agent.read().await;
    let Some((replay, rx)) = agent.subscribe_live_run(&id, since_seq).await else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "Run stream not found",
                "run_id": id,
            })),
        )
            .into_response();
    };
    drop(agent);

    let replay_stream = futures::stream::iter(
        replay
            .into_iter()
            .map(|event| Ok::<Event, std::convert::Infallible>(run_event_to_sse_event(event))),
    );
    let live_stream = futures::stream::unfold(rx, |maybe_rx| async move {
        let mut rx = maybe_rx?;
        loop {
            match rx.recv().await {
                Ok(event) => {
                    return Some((
                        Ok::<Event, std::convert::Infallible>(run_event_to_sse_event(event)),
                        Some(rx),
                    ));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    return None;
                }
            }
        }
    });
    let stream = replay_stream.chain(live_stream);

    Sse::new(cap_sse_lifetime(stream))
        .keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(10)))
        .into_response()
}

pub(super) async fn cancel_run(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    let sender = {
        let guard = state.chat_conversation_cancellations.read().await;
        guard
            .values()
            .find(|entry| entry.request_id == id)
            .map(|entry| entry.sender.clone())
    };

    if let Some(sender) = sender {
        let _ = sender.send(true);
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "cancellation_requested",
                "run_id": id,
                "cancellation_requested": true,
            })),
        )
            .into_response();
    }

    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({
            "error": "Active run not found",
            "run_id": id,
            "cancellation_requested": false,
        })),
    )
        .into_response()
}

pub(super) async fn get_conversation_latest_run(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    let runs = match agent
        .storage
        .list_execution_runs_for_conversation(&id, 1)
        .await
    {
        Ok(runs) => runs,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to load conversation runs: {}", error),
                    "conversation_id": id,
                })),
            )
                .into_response();
        }
    };
    let Some(run) = runs.into_iter().next() else {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "run": serde_json::Value::Null,
                "events": [],
            })),
        )
            .into_response();
    };

    let events = agent.load_persisted_run_events(&run.id).await;

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "run": run,
            "events": events,
        })),
    )
        .into_response()
}

pub(super) async fn resume_run(
    State(state): State<AppState>,
    maybe_caller: Option<Extension<crate::actions::ActionCallerPrincipal>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    let result = {
        let agent = state.agent.read().await;
        agent
            .resume_execution_run(&id, maybe_caller.as_ref().map(|caller| &caller.0))
            .await
    };

    match result {
        Ok(processed) => (
            StatusCode::OK,
            Json(ChatResponse {
                response: processed.response,
                proof_id: None,
                conversation_id: processed.conversation_id,
                conversation_title: processed.conversation_title,
                run_id: processed.run_id,
                run_status: processed.run_status,
                trace_id: processed.trace_id,
                total_tokens: processed.total_tokens,
                choices: processed.choices,
                degradation: processed.degradation,
                attempted_models: processed.attempted_models,
                user_outcome: processed.user_outcome,
            }),
        )
            .into_response(),
        Err(error) if error.to_string() == "Run not found" => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "Run not found",
                "run_id": id,
            })),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Failed to resume run: {}", error),
                "run_id": id,
            })),
        )
            .into_response(),
    }
}

// - WhatsApp Webhook -

/// GET /webhook/whatsapp - Meta verification handshake
pub(super) async fn whatsapp_webhook_verify(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Response {
    let verify_token = {
        let agent = state.agent.read().await;
        agent
            .config
            .whatsapp
            .as_ref()
            .map(|w| w.verify_token.clone())
    };

    let Some(token) = verify_token else {
        return (StatusCode::FORBIDDEN, "WhatsApp not configured").into_response();
    };

    match crate::channels::whatsapp::verify_webhook(&params, &token).await {
        Ok(challenge) => challenge.into_response(),
        Err(e) => {
            tracing::warn!("WhatsApp webhook verify failed: {}", e);
            (StatusCode::FORBIDDEN, format!("Verification failed: {}", e)).into_response()
        }
    }
}

/// POST /webhook/whatsapp - Inbound messages from Meta
pub(super) async fn whatsapp_webhook_handler(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
) -> Response {
    let (parts, body) = request.into_parts();
    let signature = parts
        .headers
        .get("x-hub-signature-256")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string());
    let body_bytes = match axum::body::to_bytes(body, 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(serde_json::json!({ "error": "request body too large" })),
            )
                .into_response();
        }
    };
    let body = match serde_json::from_slice::<serde_json::Value>(&body_bytes) {
        Ok(body) => body,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": format!("Failed to parse WhatsApp webhook payload: {}", error)
                })),
            )
                .into_response();
        }
    };
    let config = {
        let guard = state.agent.read().await;
        guard.config.whatsapp.clone()
    };
    let Some(config) = config else {
        return (StatusCode::FORBIDDEN, "WhatsApp not configured").into_response();
    };
    let is_baileys = body.get("_source").and_then(|value| value.as_str()) == Some("baileys");

    if is_baileys {
        if config.mode != crate::channels::whatsapp::WhatsAppMode::Baileys {
            return (
                StatusCode::FORBIDDEN,
                "WhatsApp bridge payload rejected because Cloud API mode is configured",
            )
                .into_response();
        }
        let expected_api_key = state
            .api_key
            .read()
            .await
            .clone()
            .filter(|value| !value.trim().is_empty());
        let Some(expected_api_key) = expected_api_key else {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "HTTP API key is required before bridge webhooks can be accepted",
            )
                .into_response();
        };
        if !auth::has_valid_bearer_api_key(&parts.headers, Some(expected_api_key.as_str())) {
            return (
                StatusCode::UNAUTHORIZED,
                "WhatsApp bridge authorization failed",
            )
                .into_response();
        }
    } else {
        if config.mode != crate::channels::whatsapp::WhatsAppMode::CloudApi {
            return (
                StatusCode::FORBIDDEN,
                "WhatsApp Cloud API payload rejected because Baileys mode is configured",
            )
                .into_response();
        }
        if let Err(error) = crate::channels::whatsapp::verify_cloud_api_request_signature(
            &config,
            &body_bytes,
            signature.as_deref(),
        ) {
            tracing::warn!("WhatsApp webhook request rejected: {}", error);
            return channel_webhook_error_response(error);
        }
    }

    let agent = state.agent.clone();
    crate::spawn_logged!("src/channels/http.rs:11217", async move {
        if let Err(e) = crate::channels::whatsapp::handle_webhook(agent, &body).await {
            tracing::error!("WhatsApp webhook processing error: {}", e);
        }
    });
    StatusCode::OK.into_response()
}

/// POST /webhook/slack - Slack Events API ingress
pub(super) async fn slack_webhook_handler(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
) -> Response {
    let (parts, body) = request.into_parts();
    let timestamp = parts
        .headers
        .get("x-slack-request-timestamp")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string());
    let signature = parts
        .headers
        .get("x-slack-signature")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string());
    let body_bytes = match axum::body::to_bytes(body, 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(serde_json::json!({ "error": "request body too large" })),
            )
                .into_response();
        }
    };

    let agent = state.agent.clone();
    if let Err(error) = crate::channels::slack::verify_webhook_request(
        agent.clone(),
        &body_bytes,
        timestamp.as_deref(),
        signature.as_deref(),
    )
    .await
    {
        tracing::warn!("Slack webhook request rejected: {}", error);
        return (StatusCode::BAD_REQUEST, error.to_string()).into_response();
    }

    let is_url_verification = serde_json::from_slice::<serde_json::Value>(&body_bytes)
        .ok()
        .and_then(|payload| {
            payload
                .get("type")
                .and_then(|value| value.as_str())
                .map(str::to_string)
        })
        .is_some_and(|event_type| event_type == "url_verification");

    if is_url_verification {
        return match crate::channels::slack::handle_webhook(
            agent,
            &body_bytes,
            timestamp.as_deref(),
            signature.as_deref(),
        )
        .await
        {
            Ok(response) => (StatusCode::OK, response).into_response(),
            Err(error) => {
                tracing::warn!("Slack webhook processing failed: {}", error);
                (StatusCode::BAD_REQUEST, error.to_string()).into_response()
            }
        };
    }

    let timestamp_owned = timestamp.clone();
    let signature_owned = signature.clone();
    let body_owned = body_bytes.to_vec();
    crate::spawn_logged!("src/channels/http.rs:11295", async move {
        if let Err(error) = crate::channels::slack::handle_webhook(
            agent,
            &body_owned,
            timestamp_owned.as_deref(),
            signature_owned.as_deref(),
        )
        .await
        {
            tracing::warn!("Slack webhook processing failed: {}", error);
        }
    });

    (StatusCode::OK, "ok").into_response()
}

/// POST /webhook/teams - Teams/Bot Framework ingress
pub(super) async fn teams_webhook_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(activity): Json<crate::channels::teams::TeamsActivity>,
) -> Response {
    let agent = state.agent.clone();
    let config = {
        let guard = state.agent.read().await;
        guard
            .config
            .teams
            .clone()
            .or(
                crate::channels::teams::load_config_from_storage(&guard.storage)
                    .await
                    .ok()
                    .flatten(),
            )
    };

    let Some(config) = config else {
        return (StatusCode::FORBIDDEN, "Teams not configured").into_response();
    };

    let authorization = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string());
    let verified = match crate::channels::teams::verify_inbound_activity_request(
        &config,
        authorization.as_deref(),
        &activity,
    )
    .await
    {
        Ok(verified) => verified,
        Err(error) => {
            tracing::warn!("Teams webhook authorization failed: {}", error);
            return (StatusCode::FORBIDDEN, error.to_string()).into_response();
        }
    };

    crate::spawn_logged!("src/channels/http.rs:11354", async move {
        if let Err(error) =
            crate::channels::teams::handle_activity(&agent, &config, activity, verified).await
        {
            tracing::warn!("Teams webhook processing failed: {}", error);
        }
    });

    (
        StatusCode::OK,
        Json(serde_json::json!({ "status": "accepted" })),
    )
        .into_response()
}

/// POST /webhook/google-chat - Google Chat app ingress
pub(super) async fn google_chat_webhook_handler(
    State(state): State<AppState>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    match crate::channels::google_chat::handle_webhook(state.agent.clone(), &payload).await {
        Ok(status) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": status })),
        )
            .into_response(),
        Err(error) => {
            tracing::warn!("Google Chat webhook processing failed: {}", error);
            channel_webhook_error_response(error)
        }
    }
}

/// POST /webhook/signal - Signal bridge ingress
pub(super) async fn signal_webhook_handler(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
) -> Response {
    let (parts, body) = request.into_parts();
    let body_bytes = match axum::body::to_bytes(body, 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(serde_json::json!({ "error": "request body too large" })),
            )
                .into_response();
        }
    };
    let headers = parts.headers.clone();
    match crate::channels::signal::handle_webhook(state.agent.clone(), &headers, &body_bytes).await
    {
        Ok(status) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": status })),
        )
            .into_response(),
        Err(error) => {
            tracing::warn!("Signal webhook processing failed: {}", error);
            channel_webhook_error_response(error)
        }
    }
}

/// POST /webhook/imessage - iMessage bridge ingress
pub(super) async fn imessage_webhook_handler(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
) -> Response {
    let (parts, body) = request.into_parts();
    let body_bytes = match axum::body::to_bytes(body, 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(serde_json::json!({ "error": "request body too large" })),
            )
                .into_response();
        }
    };
    let headers = parts.headers.clone();
    match crate::channels::imessage::handle_webhook(state.agent.clone(), &headers, &body_bytes)
        .await
    {
        Ok(status) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": status })),
        )
            .into_response(),
        Err(error) => {
            tracing::warn!("iMessage webhook processing failed: {}", error);
            channel_webhook_error_response(error)
        }
    }
}

/// POST /webhook/line - LINE Messaging API ingress
pub(super) async fn line_webhook_handler(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
) -> Response {
    let (parts, body) = request.into_parts();
    let signature = parts
        .headers
        .get("x-line-signature")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string());
    let body_bytes = match axum::body::to_bytes(body, 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(serde_json::json!({ "error": "request body too large" })),
            )
                .into_response();
        }
    };
    match crate::channels::line::handle_webhook(
        state.agent.clone(),
        &body_bytes,
        signature.as_deref(),
    )
    .await
    {
        Ok(status) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": status })),
        )
            .into_response(),
        Err(error) => {
            tracing::warn!("LINE webhook processing failed: {}", error);
            channel_webhook_error_response(error)
        }
    }
}

/// POST /webhook/wechat - WeChat bridge ingress
pub(super) async fn wechat_webhook_handler(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
) -> Response {
    let (parts, body) = request.into_parts();
    let body_bytes = match axum::body::to_bytes(body, 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(serde_json::json!({ "error": "request body too large" })),
            )
                .into_response();
        }
    };
    let headers = parts.headers.clone();
    match crate::channels::wechat::handle_webhook(state.agent.clone(), &headers, &body_bytes).await
    {
        Ok(status) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": status })),
        )
            .into_response(),
        Err(error) => {
            tracing::warn!("WeChat webhook processing failed: {}", error);
            channel_webhook_error_response(error)
        }
    }
}

/// POST /webhook/qq - QQ bridge ingress
pub(super) async fn qq_webhook_handler(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
) -> Response {
    let (parts, body) = request.into_parts();
    let body_bytes = match axum::body::to_bytes(body, 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(serde_json::json!({ "error": "request body too large" })),
            )
                .into_response();
        }
    };
    let headers = parts.headers.clone();
    match crate::channels::qq::handle_webhook(state.agent.clone(), &headers, &body_bytes).await {
        Ok(status) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": status })),
        )
            .into_response(),
        Err(error) => {
            tracing::warn!("QQ webhook processing failed: {}", error);
            channel_webhook_error_response(error)
        }
    }
}

pub(super) fn channel_webhook_error_response(error: anyhow::Error) -> Response {
    let message = error.to_string();
    let lowered = message.to_ascii_lowercase();
    let status = if lowered.contains("signature")
        || lowered.contains("verification token")
        || lowered.contains("bridge token mismatch")
        || lowered.contains("token mismatch")
    {
        StatusCode::UNAUTHORIZED
    } else if lowered.contains("did not include")
        || lowered.contains("is required")
        || lowered.contains("missing")
        || lowered.contains("invalid")
        || lowered.contains("failed to parse")
    {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    (
        status,
        Json(serde_json::json!({
            "status": "error",
            "error": message
        })),
    )
        .into_response()
}

// - WhatsApp Bridge Proxy -

pub(super) fn embedded_whatsapp_bridge_unavailable_reason() -> Option<String> {
    if !std::path::Path::new("/app/bridges/whatsapp-bridge/index.js").exists() {
        return Some(
            "Bundled WhatsApp bridge is not installed in this image. Use the full image or switch to an external bridge."
                .to_string(),
        );
    }
    if !std::path::Path::new("/app/bridges/whatsapp-bridge/node_modules").exists() {
        return Some(
            "Bundled WhatsApp bridge dependencies are missing. Rebuild the full image or switch to an external bridge."
                .to_string(),
        );
    }
    None
}

pub(super) async fn should_manage_embedded_whatsapp_bridge(state: &AppState) -> bool {
    let config = { state.agent.read().await.config.whatsapp.clone() };
    config
        .as_ref()
        .is_some_and(crate::channels::whatsapp::WhatsAppChannelConfig::uses_embedded_bridge)
}

pub(super) fn whatsapp_bridge_managed_by(
    config: &crate::channels::whatsapp::WhatsAppChannelConfig,
) -> &'static str {
    match config.bridge_runtime() {
        crate::channels::whatsapp::WhatsAppBridgeRuntime::Embedded => "embedded",
        crate::channels::whatsapp::WhatsAppBridgeRuntime::External => "external",
    }
}

pub(super) fn legacy_whatsapp_bridge_warning(
    config: &crate::channels::whatsapp::WhatsAppChannelConfig,
) -> Option<String> {
    if config.uses_external_bridge() && config.bridge_token.trim().is_empty() {
        Some(
            "This external bridge uses a legacy configuration without a bridge token. Add one to harden bridge requests."
                .to_string(),
        )
    } else {
        None
    }
}

pub(super) fn generate_whatsapp_bridge_token() -> String {
    format!("wa_bridge_{}", uuid::Uuid::new_v4().simple())
}

pub(super) fn whatsapp_bridge_status_payload(
    status: &str,
    managed_by: &str,
    detail: Option<String>,
    error: Option<String>,
    warning: Option<String>,
    installed: Option<bool>,
) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "status": status,
        "managed_by": managed_by,
    });
    if let Some(object) = payload.as_object_mut() {
        if let Some(detail) = detail.filter(|value| !value.trim().is_empty()) {
            object.insert("detail".to_string(), serde_json::json!(detail));
        }
        if let Some(error) = error.filter(|value| !value.trim().is_empty()) {
            object.insert("error".to_string(), serde_json::json!(error));
        }
        if let Some(warning) = warning.filter(|value| !value.trim().is_empty()) {
            object.insert("warning".to_string(), serde_json::json!(warning));
        }
        if let Some(installed) = installed {
            object.insert("installed".to_string(), serde_json::json!(installed));
        }
    }
    payload
}

/// GET /api/whatsapp-bridge/status - resolve WhatsApp bridge ownership and status
pub(super) async fn whatsapp_bridge_status(State(state): State<AppState>) -> Response {
    let config = {
        let agent = state.agent.read().await;
        agent.config.whatsapp.clone()
    };
    let Some(config) = config else {
        return (
            StatusCode::OK,
            Json(whatsapp_bridge_status_payload(
                "disabled",
                "none",
                Some("WhatsApp is disabled.".to_string()),
                None,
                None,
                None,
            )),
        )
            .into_response();
    };
    if config.mode != crate::channels::whatsapp::WhatsAppMode::Baileys {
        return (
            StatusCode::OK,
            Json(whatsapp_bridge_status_payload(
                "disabled",
                "none",
                Some("Cloud API mode does not use the WhatsApp bridge.".to_string()),
                None,
                None,
                None,
            )),
        )
            .into_response();
    }

    let managed_by = whatsapp_bridge_managed_by(&config);
    let warning = legacy_whatsapp_bridge_warning(&config);
    let installed = if config.uses_embedded_bridge() {
        Some(embedded_whatsapp_bridge_unavailable_reason().is_none())
    } else {
        None
    };
    if let Some(error) = config
        .uses_embedded_bridge()
        .then(embedded_whatsapp_bridge_unavailable_reason)
        .flatten()
    {
        return (
            StatusCode::OK,
            Json(whatsapp_bridge_status_payload(
                "unavailable",
                managed_by,
                Some("Bundled bridge unavailable.".to_string()),
                Some(error),
                warning,
                installed,
            )),
        )
            .into_response();
    }

    let bridge_url = match config.effective_bridge_url() {
        Ok(url) => url,
        Err(error) => {
            return (
                StatusCode::OK,
                Json(whatsapp_bridge_status_payload(
                    "unavailable",
                    managed_by,
                    Some("Bridge configuration is incomplete.".to_string()),
                    Some(error.to_string()),
                    warning,
                    installed,
                )),
            )
                .into_response();
        }
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap_or_default();

    let mut request = client.get(format!("{}/status", bridge_url.trim_end_matches('/')));
    if !config.bridge_token.trim().is_empty() {
        request = request.header("x-agentark-bridge-token", config.bridge_token.trim());
    }

    match request.send().await {
        Ok(resp) => {
            if !resp.status().is_success() {
                let status_code = resp.status();
                let body = resp.text().await.unwrap_or_default();
                let detail = if body.trim().is_empty() {
                    format!("Bridge returned {}", status_code)
                } else {
                    format!("Bridge returned {}: {}", status_code, body)
                };
                return (
                    StatusCode::OK,
                    Json(whatsapp_bridge_status_payload(
                        "unavailable",
                        managed_by,
                        Some("Bridge is unreachable right now.".to_string()),
                        Some(detail),
                        warning,
                        installed,
                    )),
                )
                    .into_response();
            }

            let mut payload = match resp.json::<serde_json::Value>().await {
                Ok(value) if value.is_object() => value,
                Ok(_) | Err(_) => {
                    return (
                        StatusCode::OK,
                        Json(whatsapp_bridge_status_payload(
                            "unavailable",
                            managed_by,
                            Some("Bridge returned invalid status data.".to_string()),
                            Some("Bridge returned invalid JSON.".to_string()),
                            warning,
                            installed,
                        )),
                    )
                        .into_response();
                }
            };
            if let Some(object) = payload.as_object_mut() {
                object.insert("managed_by".to_string(), serde_json::json!(managed_by));
                if let Some(warning) = warning.clone() {
                    object.insert("warning".to_string(), serde_json::json!(warning));
                }
                if let Some(installed) = installed {
                    object.insert("installed".to_string(), serde_json::json!(installed));
                }
            }
            (StatusCode::OK, Json(payload)).into_response()
        }
        Err(e) => (
            StatusCode::OK,
            Json(whatsapp_bridge_status_payload(
                "unavailable",
                managed_by,
                Some("Bridge is unreachable right now.".to_string()),
                Some(format!("Bridge unreachable: {}", e)),
                warning,
                installed,
            )),
        )
            .into_response(),
    }
}

/// POST /api/whatsapp-bridge/logout - proxy logout to Baileys bridge
pub(super) async fn whatsapp_bridge_logout(State(state): State<AppState>) -> Response {
    let config = {
        let agent = state.agent.read().await;
        agent.config.whatsapp.clone()
    };
    let Some(config) = config else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "WhatsApp is disabled." })),
        )
            .into_response();
    };
    if config.mode != crate::channels::whatsapp::WhatsAppMode::Baileys {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({ "error": "Cloud API mode does not use the WhatsApp bridge." }),
            ),
        )
            .into_response();
    }
    if let Some(error) = config
        .uses_embedded_bridge()
        .then(embedded_whatsapp_bridge_unavailable_reason)
        .flatten()
    {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": error })),
        )
            .into_response();
    }
    let bridge_url = match config.effective_bridge_url() {
        Ok(url) => url,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": error.to_string() })),
            )
                .into_response();
        }
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap_or_default();

    let mut request = client.post(format!("{}/logout", bridge_url.trim_end_matches('/')));
    if !config.bridge_token.trim().is_empty() {
        request = request.header("x-agentark-bridge-token", config.bridge_token.trim());
    }

    match request.send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            (
                StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
                [(header::CONTENT_TYPE, "application/json")],
                body,
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": format!("Bridge unreachable: {}", e) })),
        )
            .into_response(),
    }
}

/// GET /api/telegram/status - connectivity check for configured Telegram bot.
pub(super) async fn telegram_channel_status(State(state): State<AppState>) -> Response {
    let (enabled, bot_token) = {
        let agent = state.agent.read().await;
        if let Some(cfg) = &agent.config.telegram {
            (true, cfg.bot_token.clone())
        } else {
            (false, String::new())
        }
    };

    if !enabled {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "disabled",
                "detail": "Telegram is disabled.",
                "enabled": false,
                "configured": false,
                "probe_status": "disabled"
            })),
        )
            .into_response();
    }

    if bot_token.trim().is_empty() {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "missing_token",
                "detail": "Telegram bot token is not configured.",
                "enabled": true,
                "configured": false,
                "probe_status": "missing_token"
            })),
        )
            .into_response();
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(6))
        .build()
        .unwrap_or_default();

    let url = format!("https://api.telegram.org/bot{}/getMe", bot_token.trim());

    match client.get(url).send().await {
        Ok(resp) => {
            let status = resp.status();
            let payload = resp
                .json::<serde_json::Value>()
                .await
                .unwrap_or(serde_json::json!({}));
            if status.is_success() && payload.get("ok").and_then(|v| v.as_bool()) == Some(true) {
                let result = payload
                    .get("result")
                    .and_then(|v| v.as_object())
                    .cloned()
                    .unwrap_or_default();
                let username = result
                    .get("username")
                    .and_then(|v| v.as_str())
                    .unwrap_or("-");
                let bot_id = result
                    .get("id")
                    .and_then(|v| v.as_i64())
                    .unwrap_or_default();
                (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "connected",
                        "detail": format!("Connected as @{} ({})", username, bot_id),
                        "enabled": true,
                        "configured": true,
                        "probe_status": "connected",
                        "username": username,
                        "bot_id": bot_id
                    })),
                )
                    .into_response()
            } else {
                let desc = payload
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Telegram API returned an error.");
                let detail = desc.trim();
                let detail_lower = detail.to_ascii_lowercase();
                let invalid_token =
                    status == StatusCode::UNAUTHORIZED
                        || detail_lower.contains("unauthorized")
                        || detail_lower.contains("invalid")
                        || detail_lower.contains("not found")
                        || detail_lower.contains("bot token");
                (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": if invalid_token { "error" } else { "configured" },
                        "detail": if invalid_token {
                            detail.to_string()
                        } else {
                            format!("Bot token is saved. Last live check failed: {}", detail)
                        },
                        "enabled": true,
                        "configured": true,
                        "probe_status": if invalid_token { "invalid_token" } else { "api_error" }
                    })),
                )
                    .into_response()
            }
        }
        Err(e) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "configured",
                "detail": format!("Bot token is saved. Last live check failed: Telegram API unreachable: {}", e),
                "enabled": true,
                "configured": true,
                "probe_status": "unreachable"
            })),
        )
            .into_response(),
    }
}

/// Get approval audit log (persisted in database)
pub(super) async fn get_approval_log(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let limit = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(20u64);
    let offset = params
        .get("offset")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0u64);
    let agent = state.agent.read().await;
    match agent
        .encrypted_storage
        .get_approval_log_decrypted(limit, offset)
        .await
    {
        Ok(log) => Json(serde_json::json!({ "approvals": log, "limit": limit, "offset": offset }))
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to get approval log: {}", e),
            }),
        )
            .into_response(),
    }
}

/// Get security event log (persisted in database), with pagination and optional event type filter.
pub(super) async fn get_security_logs(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let limit = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(20u64)
        .clamp(1, 100);
    let offset = params
        .get("offset")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0u64);
    let event_type = params
        .get("event_type")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let agent = state.agent.read().await;
    let total = match agent
        .storage
        .count_security_logs(event_type.as_deref())
        .await
    {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to count security logs: {}", e),
                }),
            )
                .into_response();
        }
    };

    match agent
        .storage
        .list_security_logs_paginated(limit, offset, event_type.as_deref())
        .await
    {
        Ok(logs) => Json(serde_json::json!({
            "logs": logs,
            "total": total,
            "limit": limit,
            "offset": offset,
            "event_type": event_type,
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to get security logs: {}", e),
            }),
        )
            .into_response(),
    }
}

/// Spawn the WhatsApp bridge Node.js process (if not already running)
pub(super) async fn spawn_whatsapp_bridge(state: AppState) -> Result<(), String> {
    let bridge_arc = state.whatsapp_bridge.clone();
    {
        let bridge = bridge_arc.read().await;
        if bridge.active {
            return Ok(());
        }
    }
    let wa_config = {
        let agent = state.agent.read().await;
        agent.config.whatsapp.clone()
    }
    .ok_or_else(|| "WhatsApp is not configured".to_string())?;
    if !wa_config.uses_embedded_bridge() {
        return Err("WhatsApp is not configured to use the bundled embedded bridge".to_string());
    }

    let api_key = state
        .api_key
        .read()
        .await
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            "HTTP API key is required before the WhatsApp bridge can be started".to_string()
        })?;

    if let Some(error) = embedded_whatsapp_bridge_unavailable_reason() {
        let mut bridge = bridge_arc.write().await;
        bridge.active = false;
        bridge.error = Some(error.clone());
        return Err(error);
    }

    let mut command = tokio::process::Command::new("node");
    command
        .arg("/app/bridges/whatsapp-bridge/index.js")
        .env("BRIDGE_PORT", "8999")
        .env("BRIDGE_HOST", "127.0.0.1")
        .env("AGENTARK_URL", "http://127.0.0.1:8990")
        .env("AGENTARK_API_KEY", api_key)
        .env("AUTH_DIR", "/app/data/whatsapp-auth");
    if !wa_config.bridge_token.trim().is_empty() {
        command.env("BRIDGE_TOKEN", wa_config.bridge_token.trim());
    }
    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);

    match command.spawn() {
        Ok(child) => {
            let pid = child.id();
            let mut bridge = bridge_arc.write().await;
            bridge.process = Some(child);
            bridge.active = true;
            bridge.error = None;
            tracing::info!("WhatsApp bridge started (PID: {:?})", pid);
            Ok(())
        }
        Err(e) => {
            let mut bridge = bridge_arc.write().await;
            bridge.active = false;
            bridge.error = Some(format!("Failed to spawn bridge: {}", e));
            Err(format!("Failed to start WhatsApp bridge: {}", e))
        }
    }
}

/// Stop the WhatsApp bridge process
pub(super) async fn stop_whatsapp_bridge(bridge_arc: Arc<RwLock<WhatsAppBridgeState>>) {
    let mut bridge = bridge_arc.write().await;
    if let Some(ref mut child) = bridge.process {
        let _ = child.kill().await;
        tracing::info!("WhatsApp bridge stopped");
    }
    bridge.process = None;
    bridge.active = false;
    bridge.error = None;
}

/// List active watchers
pub(super) fn watcher_history_error_is_notification_summary_failure(error: Option<&str>) -> bool {
    let Some(error) = error else {
        return false;
    };
    let lower = error.to_ascii_lowercase();
    lower.contains("watcher notification")
        || lower.contains("notification summary")
        || lower.contains("follow-up summary")
}

pub(super) async fn get_watchers(State(state): State<AppState>) -> Json<serde_json::Value> {
    let (watchers, supervisor_states) = {
        let agent = state.agent.read().await;
        (
            agent.watcher_manager.list().await,
            crate::core::list_automation_supervisor_states(&agent.storage)
                .await
                .unwrap_or_default(),
        )
    };
    let live_ids: HashSet<String> = watchers.iter().map(|w| w.id.to_string()).collect();
    let mut watcher_list: Vec<serde_json::Value> = watchers
        .iter()
        .map(|w| {
            let status_error = match &w.status {
                crate::core::watcher::WatcherStatus::Failed { error } => Some(error.clone()),
                _ => None,
            };
            serde_json::json!({
                "id": w.id.to_string(),
                "description": w.description,
                "poll_action": w.poll_action,
                "poll_arguments": w.poll_arguments,
                "condition": w.condition,
                "status": automation_watcher_status_label(&w.status),
                "status_error": status_error,
                "interval_secs": w.interval_secs,
                "timeout_secs": w.timeout_secs,
                "poll_count": w.poll_count,
                "created_at": w.created_at.to_rfc3339(),
                "last_poll_at": w.last_poll_at.map(|t| t.to_rfc3339()),
                "notify_channel": w.notify_channel,
                "on_trigger": w.on_trigger,
                "trigger_result": w.trigger_result,
                "last_result": w.last_result,
                "last_error": w.last_error,
                "last_poll_outcome": w.last_poll_outcome,
                "notification_attempts": w.notification_attempts,
                "history_only": false,
            })
        })
        .collect();
    watcher_list.extend(
        supervisor_states
            .into_iter()
            .filter(|state| {
                state.automation_kind == "watcher" && !live_ids.contains(&state.automation_id)
            })
            .map(|state| {
                let created_at = state
                    .created_at
                    .clone()
                    .or_else(|| state.last_run_at.clone())
                    .or_else(|| state.last_success_at.clone());
                let notification_summary_failure = state.status == "failed"
                    && watcher_history_error_is_notification_summary_failure(
                        state.last_error.as_deref(),
                    );
                let status = if notification_summary_failure {
                    "triggered".to_string()
                } else {
                    state.status.clone()
                };
                let status_error = if notification_summary_failure {
                    None
                } else {
                    state.last_error.clone()
                };
                let last_poll_outcome = match status.as_str() {
                    "triggered" => Some("matched"),
                    "failed" | "timed_out" => Some("error"),
                    _ => None,
                };
                serde_json::json!({
                    "id": state.automation_id,
                    "description": state.title,
                    "poll_action": state.action,
                    "poll_arguments": serde_json::Value::Null,
                    "condition": serde_json::Value::Null,
                    "status": status,
                    "status_error": status_error,
                    "interval_secs": serde_json::Value::Null,
                    "timeout_secs": serde_json::Value::Null,
                    "poll_count": state.attempt_count,
                    "created_at": created_at,
                    "last_poll_at": state.last_run_at,
                    "notify_channel": serde_json::Value::Null,
                    "on_trigger": serde_json::Value::Null,
                    "trigger_result": serde_json::Value::Null,
                    "last_result": serde_json::Value::Null,
                    "last_error": status_error,
                    "last_poll_outcome": last_poll_outcome,
                    "notification_attempts": Vec::<serde_json::Value>::new(),
                    "history_only": true,
                })
            }),
    );
    watcher_list.sort_by(|left, right| {
        let left_created = left
            .get("created_at")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let right_created = right
            .get("created_at")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        right_created.cmp(left_created)
    });
    Json(serde_json::json!({ "watchers": watcher_list }))
}

/// Cancel a watcher
pub(super) async fn cancel_watcher(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Json<serde_json::Value> {
    let agent = state.agent.read().await;
    if let Ok(uuid) = uuid::Uuid::parse_str(&id) {
        let cancelled = agent.watcher_manager.cancel(uuid).await;
        if cancelled {
            if let Some(watcher) = agent.watcher_manager.get(uuid).await {
                agent
                    .sync_watcher_supervisor_state(&watcher, Some("cancelled"), None)
                    .await;
            }
        }
        Json(serde_json::json!({ "cancelled": cancelled }))
    } else {
        Json(serde_json::json!({ "error": "Invalid watcher ID" }))
    }
}

pub(super) async fn pause_watcher(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Json<serde_json::Value> {
    let agent = state.agent.read().await;
    if let Ok(uuid) = uuid::Uuid::parse_str(&id) {
        let paused = agent.watcher_manager.pause(uuid).await;
        if paused {
            if let Some(watcher) = agent.watcher_manager.get(uuid).await {
                agent
                    .sync_watcher_supervisor_state(&watcher, Some("paused"), None)
                    .await;
            }
        }
        Json(serde_json::json!({ "paused": paused }))
    } else {
        Json(serde_json::json!({ "error": "Invalid watcher ID" }))
    }
}

pub(super) async fn resume_watcher(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Json<serde_json::Value> {
    let agent = state.agent.read().await;
    if let Ok(uuid) = uuid::Uuid::parse_str(&id) {
        let resumed = agent.watcher_manager.resume(uuid).await;
        if resumed {
            if let Some(watcher) = agent.watcher_manager.get(uuid).await {
                agent
                    .sync_watcher_supervisor_state(&watcher, Some("active"), None)
                    .await;
            }
        }
        Json(serde_json::json!({ "resumed": resumed }))
    } else {
        Json(serde_json::json!({ "error": "Invalid watcher ID" }))
    }
}

pub(super) async fn pause_all_watchers(State(state): State<AppState>) -> Json<serde_json::Value> {
    let agent = state.agent.read().await;
    let paused = agent.watcher_manager.pause_all().await;
    Json(serde_json::json!({ "paused": paused }))
}

pub(super) async fn resume_all_watchers(State(state): State<AppState>) -> Json<serde_json::Value> {
    let agent = state.agent.read().await;
    let resumed = agent.watcher_manager.resume_all().await;
    Json(serde_json::json!({ "resumed": resumed }))
}

pub(super) async fn run_watcher_now(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Json<serde_json::Value> {
    let agent = state.agent.read().await;
    if let Ok(uuid) = uuid::Uuid::parse_str(&id) {
        let queued = agent.watcher_manager.run_now(uuid).await;
        Json(serde_json::json!({ "queued": queued }))
    } else {
        Json(serde_json::json!({ "error": "Invalid watcher ID" }))
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct WatcherExtendRequest {
    #[serde(default)]
    extra_hours: Option<u64>,
    #[serde(default)]
    extra_days: Option<u64>,
    #[serde(default)]
    extra_secs: Option<u64>,
    #[serde(default)]
    until_stopped: Option<bool>,
}

pub(super) async fn extend_watcher(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(request): Json<WatcherExtendRequest>,
) -> Json<serde_json::Value> {
    let agent = state.agent.read().await;
    if let Ok(uuid) = uuid::Uuid::parse_str(&id) {
        let timeout_secs = if request.until_stopped.unwrap_or(false) {
            agent.watcher_manager.extend_until_stopped(uuid).await
        } else {
            let extra_secs = request.extra_secs.unwrap_or(0)
                + request.extra_hours.unwrap_or(0).saturating_mul(60 * 60)
                + request.extra_days.unwrap_or(0).saturating_mul(24 * 60 * 60);
            agent.watcher_manager.extend_timeout(uuid, extra_secs).await
        };
        Json(serde_json::json!({
            "updated": timeout_secs.is_some(),
            "timeout_secs": timeout_secs
        }))
    } else {
        Json(serde_json::json!({ "error": "Invalid watcher ID" }))
    }
}

pub(super) async fn delete_watcher(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Json<serde_json::Value> {
    let agent = state.agent.read().await;
    if let Ok(uuid) = uuid::Uuid::parse_str(&id) {
        let deleted_live = agent.watcher_manager.delete(uuid).await;
        let deleted_history = agent.clear_watcher_supervisor_state(&id).await;
        let deleted = deleted_live || deleted_history;
        let task_ids: Vec<String> = Vec::new();
        let watcher_ids = vec![id.clone()];
        agent
            .background_sessions
            .remove_child_references(&task_ids, &watcher_ids, Some("api"))
            .await;
        let deleted_reflect_units = agent
            .storage
            .delete_semantic_work_units_for_source("watcher", &id)
            .await
            .unwrap_or(0);
        Json(
            serde_json::json!({ "deleted": deleted, "deleted_reflect_units": deleted_reflect_units }),
        )
    } else {
        Json(serde_json::json!({ "error": "Invalid watcher ID" }))
    }
}

/// List active browser automation sessions
pub(super) async fn browser_list_sessions(
    State(state): State<AppState>,
) -> Json<serde_json::Value> {
    let agent = state.agent.read().await;
    let sessions = agent.browser_sessions.list_session_views().await;
    let total = sessions.len();
    Json(serde_json::json!({ "sessions": sessions, "total": total }))
}

#[derive(Debug, Deserialize)]
pub(super) struct BrowserHandoffCompleteRequest {
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    response: Option<String>,
}

pub(super) fn browser_session_action_error_response(error: anyhow::Error) -> Response {
    let message = error.to_string();
    let status = if message.contains("not found") {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::CONFLICT
    };
    (status, Json(ErrorResponse { error: message })).into_response()
}

/// Complete or resume a browser handoff with an operator note.
pub(super) async fn browser_respond(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(body): Json<BrowserHandoffCompleteRequest>,
) -> Response {
    let agent = state.agent.read().await;
    let note = body
        .note
        .as_deref()
        .or(body.response.as_deref())
        .unwrap_or("")
        .trim()
        .to_string();
    match agent
        .browser_sessions
        .complete_operator_handoff(&id, &note)
        .await
    {
        Ok(view) => (
            StatusCode::OK,
            Json(serde_json::to_value(view).unwrap_or_default()),
        )
            .into_response(),
        Err(error) => browser_session_action_error_response(error),
    }
}

/// Get browser session status
pub(super) async fn browser_session_status(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    match agent.browser_sessions.describe_session(&id).await {
        Some(view) => (
            StatusCode::OK,
            Json(serde_json::to_value(view).unwrap_or_default()),
        )
            .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Session not found".to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn browser_claim(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    match agent.browser_sessions.claim_operator_handoff(&id).await {
        Ok(view) => (
            StatusCode::OK,
            Json(serde_json::to_value(view).unwrap_or_default()),
        )
            .into_response(),
        Err(error) => browser_session_action_error_response(error),
    }
}

pub(super) async fn browser_release(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    match agent.browser_sessions.release_operator_handoff(&id).await {
        Ok(view) => (
            StatusCode::OK,
            Json(serde_json::to_value(view).unwrap_or_default()),
        )
            .into_response(),
        Err(error) => browser_session_action_error_response(error),
    }
}

pub(super) async fn browser_complete(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(body): Json<BrowserHandoffCompleteRequest>,
) -> Response {
    browser_respond(State(state), axum::extract::Path(id), Json(body)).await
}

pub(super) async fn browser_stop(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    match agent.browser_sessions.stop_session(&id).await {
        Ok(view) => (
            StatusCode::OK,
            Json(serde_json::to_value(view).unwrap_or_default()),
        )
            .into_response(),
        Err(error) => browser_session_action_error_response(error),
    }
}

pub(super) async fn browser_delete(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    match agent.browser_sessions.delete_session(&id).await {
        Ok(deleted) => (
            StatusCode::OK,
            Json(serde_json::json!({ "deleted": deleted })),
        )
            .into_response(),
        Err(error) => browser_session_action_error_response(error),
    }
}

/// Get agent status
pub(super) async fn status(State(state): State<AppState>) -> Json<StatusResponse> {
    let status = {
        let agent = state.agent.read().await;
        agent.status().await
    };
    let update = current_release_update_summary(&state).await;

    Json(StatusResponse {
        did: status.did,
        memory_entries: status.memory_entries,
        skills_loaded: status.actions_loaded,
        actions_loaded: Some(status.actions_loaded),
        tasks_pending: status.tasks_pending,
        version: env!("CARGO_PKG_VERSION").to_string(),
        runtime_health: collect_status_runtime_health(
            &state,
            state.runtime_started_at.elapsed().as_secs(),
        )
        .await,
        update: Some(update),
    })
}
