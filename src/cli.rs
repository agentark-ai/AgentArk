use anyhow::Result;
use clap::{Args as ClapArgs, Subcommand, ValueEnum};
use serde::Serialize;

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[value(rename_all = "kebab_case")]
pub enum ServiceMode {
    #[value(alias = "control-plane")]
    #[default]
    Control,
    Executor,
    Workspace,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Compatibility alias for --chat.
    Chat,
    /// Compatibility alias for --setup.
    Setup,
    /// Compatibility alias for --pulse.
    Pulse,
    Gateway(StatusCommandArgs),
    Channel(StatusCommandArgs),
    Routing(RoutingCommandArgs),
    Node(NodeCommandArgs),
    Browser(BrowserCommandArgs),
    Failover(FailoverCommandArgs),
    LanHelper(LanHelperCommandArgs),
    Doctor(StatusCommandArgs),
    Onboard(StatusCommandArgs),
}

#[derive(ClapArgs, Debug, Clone, Copy, Default)]
pub struct StatusCommandArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(ClapArgs, Debug, Clone)]
pub struct LanHelperCommandArgs {
    /// Host-side bind address for Docker-aware LAN discovery.
    #[arg(long, default_value = "127.0.0.1:8995")]
    pub bind: String,
    /// Bearer token required by AgentArk when the helper is called from Docker.
    #[arg(long, env = "AGENTARK_LAN_HELPER_TOKEN")]
    pub token: Option<String>,
}

#[derive(ClapArgs, Debug)]
pub struct RoutingCommandArgs {
    #[command(subcommand)]
    pub command: Option<RoutingSubcommand>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Subcommand, Debug)]
pub enum RoutingSubcommand {
    Simulate(RoutingSimulateArgs),
}

#[derive(ClapArgs, Debug, Default)]
pub struct RoutingSimulateArgs {
    #[arg(long)]
    pub channel_id: Option<String>,
    #[arg(long)]
    pub account_id: Option<String>,
    #[arg(long)]
    pub match_kind: Option<String>,
    #[arg(long)]
    pub match_value: Option<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(ClapArgs, Debug)]
pub struct NodeCommandArgs {
    #[command(subcommand)]
    pub command: Option<NodeSubcommand>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Subcommand, Debug)]
pub enum NodeSubcommand {
    Revoke(NodeTargetArgs),
}

#[derive(ClapArgs, Debug)]
pub struct NodeTargetArgs {
    pub node_id: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(ClapArgs, Debug)]
pub struct BrowserCommandArgs {
    #[command(subcommand)]
    pub command: Option<BrowserSubcommand>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Subcommand, Debug)]
pub enum BrowserSubcommand {
    Unlock(BrowserUnlockArgs),
}

#[derive(ClapArgs, Debug)]
pub struct BrowserUnlockArgs {
    pub profile_id: String,
    #[arg(long)]
    pub owner: Option<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(ClapArgs, Debug)]
pub struct FailoverCommandArgs {
    #[command(subcommand)]
    pub command: Option<FailoverSubcommand>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Subcommand, Debug)]
pub enum FailoverSubcommand {
    Select(FailoverSelectArgs),
}

#[derive(ClapArgs, Debug, Default)]
pub struct FailoverSelectArgs {
    #[arg(long)]
    pub chain_id: Option<String>,
    #[arg(long)]
    pub provider_id: Option<String>,
    #[arg(long)]
    pub auth_profile_id: Option<String>,
    #[arg(long)]
    pub session_id: Option<String>,
    #[arg(long)]
    pub model_id: Option<String>,
    #[arg(long)]
    pub allow_disabled: bool,
    #[arg(long)]
    pub allow_cooling: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Serialize)]
struct GatewayStatusSnapshot {
    generated_at: String,
    channels: crate::core::GatewayChannelsResponse,
    routing: crate::core::GatewayRoutingResponse,
    nodes: crate::core::NodeControlPlaneStatus,
    browser: crate::core::BrowserProfileListResponse,
    failover: crate::core::ModelFailoverListResponse,
    latest_pulse: Option<crate::sentinel::PulseEvent>,
}

#[derive(Debug, Serialize)]
struct OnboardChecklist {
    generated_at: String,
    steps: Vec<OnboardStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct OnboardStep {
    id: &'static str,
    title: &'static str,
    status: &'static str,
    detail: String,
    next_action: String,
}

#[derive(Debug, Clone, Copy)]
struct OnboardSummary {
    chat_ready: bool,
    configured_model_count: usize,
    channels_configured: usize,
    channels_connected: usize,
    routing_enabled_rules: usize,
    routing_rules: usize,
    nodes_total: usize,
    nodes_online: usize,
    browser_total: usize,
    browser_logged_in: usize,
    failover_auth_profiles: usize,
    failover_chains: usize,
    failover_cooling_providers: usize,
}

fn build_onboard_steps(summary: OnboardSummary) -> Vec<OnboardStep> {
    let chat_detail = if summary.chat_ready {
        format!(
            "{} chat model route(s) configured.",
            summary.configured_model_count
        )
    } else {
        "No chat model is configured yet.".to_string()
    };

    vec![
        OnboardStep {
            id: "chat_model",
            title: "Configure at least one chat model",
            status: if summary.chat_ready { "ready" } else { "blocking" },
            detail: chat_detail,
            next_action: if summary.chat_ready {
                "Run `agentark --chat` to start using CLI chat.".to_string()
            } else {
                "Run `agentark --setup`, or open http://localhost:8990 and go to Settings > Models."
                    .to_string()
            },
        },
        OnboardStep {
            id: "channels",
            title: "Configure at least one external channel",
            status: if summary.channels_configured > 0 {
                "ready"
            } else {
                "pending"
            },
            detail: format!(
                "{} configured, {} connected.",
                summary.channels_configured, summary.channels_connected
            ),
            next_action:
                "Open Channels and connect Slack, Discord, Telegram, WhatsApp, or WebChat."
                    .to_string(),
        },
        OnboardStep {
            id: "routing",
            title: "Enable deterministic routing",
            status: if summary.routing_enabled_rules > 0 {
                "ready"
            } else {
                "pending"
            },
            detail: format!(
                "{} enabled rules across {} total.",
                summary.routing_enabled_rules, summary.routing_rules
            ),
            next_action:
                "Create at least one route rule in Routing so inbound traffic lands on a bound agent."
                    .to_string(),
        },
        OnboardStep {
            id: "devices",
            title: "Pair a companion device",
            status: if summary.nodes_total > 0 {
                "ready"
            } else {
                "optional"
            },
            detail: format!(
                "{} nodes tracked, {} online.",
                summary.nodes_total, summary.nodes_online
            ),
            next_action:
                "Use Devices to pair a mobile or desktop node if camera, notifications, or system.run are needed."
                    .to_string(),
        },
        OnboardStep {
            id: "browser",
            title: "Provision a managed browser profile",
            status: if summary.browser_total > 0 {
                "ready"
            } else {
                "pending"
            },
            detail: format!(
                "{} profiles, {} logged in.",
                summary.browser_total, summary.browser_logged_in
            ),
            next_action: "Create a browser profile for login handoff and managed sessions."
                .to_string(),
        },
        OnboardStep {
            id: "failover",
            title: "Set up model auth failover",
            status: if summary.failover_auth_profiles > 0 && summary.failover_chains > 0 {
                "ready"
            } else {
                "pending"
            },
            detail: format!(
                "{} auth profiles, {} chains, {} cooling providers.",
                summary.failover_auth_profiles,
                summary.failover_chains,
                summary.failover_cooling_providers
            ),
            next_action: "Create auth profiles and at least one fallback chain in Failover."
                .to_string(),
        },
    ]
}

pub async fn run(agent: crate::core::Agent, command: Command) -> Result<()> {
    match command {
        Command::Chat | Command::Setup | Command::Pulse => {
            anyhow::bail!("top-level CLI alias reached operator command dispatcher")
        }
        Command::Gateway(args) => run_gateway_status(agent, args.json).await,
        Command::Channel(args) => run_channel_status(agent, args.json).await,
        Command::Routing(args) => run_routing(agent, args).await,
        Command::Node(args) => run_nodes(agent, args).await,
        Command::Browser(args) => run_browser(agent, args).await,
        Command::Failover(args) => run_failover(agent, args).await,
        Command::LanHelper(args) => {
            crate::actions::lan::run_lan_helper(args.bind, args.token).await
        }
        Command::Doctor(args) => run_doctor(agent, args.json).await,
        Command::Onboard(args) => run_onboard(agent, args.json).await,
    }
}

async fn build_gateway_snapshot(agent: &crate::core::Agent) -> Result<GatewayStatusSnapshot> {
    let channels = crate::core::load_gateway_channels(&agent.storage, &agent.config).await?;
    let routing = crate::core::load_gateway_routing(&agent.storage).await?;
    let nodes_plane = crate::core::NodeControlPlane::new(agent.storage.clone());
    let nodes = nodes_plane.status().await?;
    let browser = crate::core::BrowserProfileControlPlane::list(&agent.storage).await?;
    let failover = crate::core::ModelFailoverControlPlane::list(&agent.storage).await?;
    let latest_pulse = crate::sentinel::get_pulse_log(agent)
        .await
        .into_iter()
        .max_by_key(|event| event.timestamp.clone());

    Ok(GatewayStatusSnapshot {
        generated_at: chrono::Utc::now().to_rfc3339(),
        channels,
        routing,
        nodes,
        browser,
        failover,
        latest_pulse,
    })
}

async fn run_gateway_status(agent: crate::core::Agent, json: bool) -> Result<()> {
    let snapshot = build_gateway_snapshot(&agent).await?;
    if json {
        return print_json(&snapshot);
    }

    println!("Gateway");
    println!(
        "  Channels: {} connected / {} supported, {} need attention",
        snapshot.channels.summary.connected,
        snapshot.channels.summary.supported,
        snapshot.channels.summary.attention_needed
    );
    println!(
        "  Routing: {} enabled rules / {} total, {} broadcast groups",
        snapshot.routing.summary.enabled_rules,
        snapshot.routing.summary.rules,
        snapshot.routing.summary.broadcast_groups
    );
    println!(
        "  Nodes: {} online / {} total",
        snapshot.nodes.summary.online, snapshot.nodes.summary.total
    );
    println!(
        "  Browser: {} profiles, {} locked, {} need attention",
        snapshot.browser.summary.total,
        snapshot.browser.summary.locked,
        snapshot.browser.summary.needs_attention
    );
    println!(
        "  Failover: {} auth profiles, {} providers, {} cooling",
        snapshot.failover.summary.auth_profiles,
        snapshot.failover.summary.providers,
        snapshot.failover.summary.cooling_providers
    );
    if let Some(event) = snapshot.latest_pulse {
        println!("  Latest doctor: {} [{}]", event.summary, event.status);
    } else {
        println!("  Latest doctor: none");
    }
    Ok(())
}

async fn run_channel_status(agent: crate::core::Agent, json: bool) -> Result<()> {
    let payload = crate::core::load_gateway_channels(&agent.storage, &agent.config).await?;
    if json {
        return print_json(&payload);
    }

    println!("Channels");
    for channel in payload.channels {
        println!(
            "  - {:<20} {:<16} configured={} accounts={} routes={}",
            channel.name,
            channel.status,
            channel.configured,
            channel.account_count,
            channel.route_count
        );
    }
    Ok(())
}

async fn run_routing(agent: crate::core::Agent, args: RoutingCommandArgs) -> Result<()> {
    match args.command {
        Some(RoutingSubcommand::Simulate(simulate)) => {
            run_routing_simulation(agent, simulate).await
        }
        None => {
            let payload = crate::core::load_gateway_routing(&agent.storage).await?;
            if args.json {
                return print_json(&payload);
            }

            println!("Routing");
            println!(
                "  Enabled rules: {} / {}",
                payload.summary.enabled_rules, payload.summary.rules
            );
            for rule in payload.rules {
                println!(
                    "  - {:<28} priority={:<4} enabled={} target={} {}",
                    rule.name, rule.priority, rule.enabled, rule.target_kind, rule.target_value
                );
            }
            if !payload.broadcast_groups.is_empty() {
                println!("  Broadcast groups:");
                for group in payload.broadcast_groups {
                    println!(
                        "    * {} ({} members, enabled={})",
                        group.name, group.member_count, group.enabled
                    );
                }
            }
            Ok(())
        }
    }
}

async fn run_routing_simulation(
    agent: crate::core::Agent,
    args: RoutingSimulateArgs,
) -> Result<()> {
    let payload = crate::core::load_gateway_routing(&agent.storage).await?;
    let request = crate::core::GatewayRoutingSimulationRequest {
        channel_id: args.channel_id,
        account_id: args.account_id,
        match_kind: args.match_kind,
        match_value: args.match_value,
    };
    let result = crate::core::simulate_gateway_routing(&payload.rules, &request);
    if args.json {
        return print_json(&result);
    }

    if result.matched {
        println!(
            "Matched rule {} -> {} {}",
            result.rule_name.unwrap_or_else(|| "unnamed".to_string()),
            result.target_kind.unwrap_or_else(|| "target".to_string()),
            result.target_value.unwrap_or_default()
        );
    } else {
        println!(
            "No route matched. {}",
            result
                .reason
                .unwrap_or_else(|| "No additional detail.".to_string())
        );
    }
    Ok(())
}

async fn run_nodes(agent: crate::core::Agent, args: NodeCommandArgs) -> Result<()> {
    let plane = crate::core::NodeControlPlane::new(agent.storage.clone());
    match args.command {
        Some(NodeSubcommand::Revoke(target)) => {
            let node = plane.revoke(&target.node_id).await?;
            if target.json {
                return print_json(&node);
            }
            match node {
                Some(node) => println!("Revoked node {} ({})", node.display_name, node.id),
                None => println!("Node not found: {}", target.node_id),
            }
            Ok(())
        }
        None => {
            let nodes = plane.list().await?;
            let status = plane.status().await?;
            if args.json {
                return print_json(&serde_json::json!({
                    "nodes": nodes,
                    "summary": status.summary,
                    "generated_at": status.generated_at,
                }));
            }

            println!(
                "Nodes: {} total, {} online, {} degraded, {} revoked",
                status.summary.total,
                status.summary.online,
                status.summary.degraded,
                status.summary.revoked
            );
            for node in nodes {
                let capabilities = node
                    .capabilities
                    .iter()
                    .map(|capability| serde_json::to_string(capability).unwrap_or_default())
                    .collect::<Vec<_>>()
                    .join(", ");
                println!(
                    "  - {:<22} {:<10} {:<10} {}",
                    node.display_name,
                    format!("{:?}", node.state).to_lowercase(),
                    format!("{:?}", node.transport).to_lowercase(),
                    capabilities
                );
            }
            Ok(())
        }
    }
}

async fn run_browser(agent: crate::core::Agent, args: BrowserCommandArgs) -> Result<()> {
    match args.command {
        Some(BrowserSubcommand::Unlock(target)) => {
            let profile = crate::core::BrowserProfileControlPlane::unlock(
                &agent.storage,
                &target.profile_id,
                target.owner.as_deref(),
            )
            .await?;
            if target.json {
                return print_json(&profile);
            }
            println!("Unlocked browser profile {} ({})", profile.name, profile.id);
            Ok(())
        }
        None => {
            let payload = crate::core::BrowserProfileControlPlane::list(&agent.storage).await?;
            if args.json {
                return print_json(&payload);
            }

            println!(
                "Browser profiles: {} total, {} logged in, {} locked",
                payload.summary.total, payload.summary.logged_in, payload.summary.locked
            );
            for profile in payload.profiles {
                println!(
                    "  - {:<24} {:<12} enabled={} login={:?}",
                    profile.name,
                    format!("{:?}", profile.target_kind).to_lowercase(),
                    profile.enabled,
                    profile.login_state
                );
            }
            Ok(())
        }
    }
}

async fn run_failover(agent: crate::core::Agent, args: FailoverCommandArgs) -> Result<()> {
    match args.command {
        Some(FailoverSubcommand::Select(select)) => {
            let result = crate::core::ModelFailoverControlPlane::select_candidate(
                &agent.storage,
                crate::core::ModelFailoverSelectionRequest {
                    chain_id: select.chain_id,
                    provider_id: select.provider_id,
                    auth_profile_id: select.auth_profile_id,
                    session_id: select.session_id,
                    model_id: select.model_id,
                    allow_disabled: select.allow_disabled,
                    allow_cooling: select.allow_cooling,
                },
            )
            .await?;
            if select.json {
                return print_json(&result);
            }

            if result.blocked {
                println!(
                    "No eligible failover candidate. {}",
                    result
                        .blocked_reason
                        .or(result.reason)
                        .unwrap_or_else(|| "No additional detail.".to_string())
                );
            } else {
                println!(
                    "Selected provider={} auth_profile={}",
                    result
                        .selected_provider_id
                        .unwrap_or_else(|| "-".to_string()),
                    result
                        .selected_auth_profile_id
                        .unwrap_or_else(|| "-".to_string())
                );
            }
            Ok(())
        }
        None => {
            let payload = crate::core::ModelFailoverControlPlane::list(&agent.storage).await?;
            if args.json {
                return print_json(&payload);
            }

            println!(
                "Failover: {} providers, {} cooling, {} auth profiles, {} chains",
                payload.summary.providers,
                payload.summary.cooling_providers,
                payload.summary.auth_profiles,
                payload.summary.chains
            );
            for provider in payload.provider_health {
                println!(
                    "  - {:<22} enabled={} disabled={} cooling={} failures={}",
                    provider.provider_id,
                    provider.enabled,
                    provider.disabled,
                    provider.cooldown_until.is_some(),
                    provider.failure_count
                );
            }
            Ok(())
        }
    }
}

async fn run_doctor(agent: crate::core::Agent, json: bool) -> Result<()> {
    let agent = std::sync::Arc::new(tokio::sync::RwLock::new(agent));
    crate::sentinel::run_pulse(&agent).await;
    let latest = {
        let guard = agent.read().await;
        crate::sentinel::get_pulse_log(&guard)
            .await
            .into_iter()
            .max_by_key(|event| event.timestamp.clone())
    };
    if json {
        return print_json(&latest);
    }

    match latest {
        Some(event) => {
            let summary = if event.summary.trim().is_empty() {
                event.message
            } else {
                event.summary
            };
            println!("Doctor [{}] {}", event.status, summary);
            if !event.details.doctor_findings.is_empty() {
                println!("Top findings:");
                for finding in event.details.doctor_findings.iter().take(5) {
                    println!(
                        "  - [{}] {} :: {}",
                        finding.severity, finding.title, finding.target
                    );
                }
            }
        }
        None => println!("No doctor snapshot is available yet."),
    }
    Ok(())
}

async fn run_onboard(agent: crate::core::Agent, json: bool) -> Result<()> {
    let snapshot = build_gateway_snapshot(&agent).await?;
    let readiness = crate::cli_chat_readiness(&agent.config);
    let steps = build_onboard_steps(OnboardSummary {
        chat_ready: readiness.chat_ready,
        configured_model_count: readiness.configured_model_count,
        channels_configured: snapshot.channels.summary.configured,
        channels_connected: snapshot.channels.summary.connected,
        routing_enabled_rules: snapshot.routing.summary.enabled_rules,
        routing_rules: snapshot.routing.summary.rules,
        nodes_total: snapshot.nodes.summary.total,
        nodes_online: snapshot.nodes.summary.online,
        browser_total: snapshot.browser.summary.total,
        browser_logged_in: snapshot.browser.summary.logged_in,
        failover_auth_profiles: snapshot.failover.summary.auth_profiles,
        failover_chains: snapshot.failover.summary.chains,
        failover_cooling_providers: snapshot.failover.summary.cooling_providers,
    });
    let checklist = OnboardChecklist {
        generated_at: chrono::Utc::now().to_rfc3339(),
        steps,
    };

    if json {
        return print_json(&checklist);
    }

    println!("Onboarding");
    for step in checklist.steps {
        println!("  - [{}] {} :: {}", step.status, step.title, step.detail);
        println!("    next: {}", step.next_action);
    }
    Ok(())
}

fn print_json<T>(value: &T) -> Result<()>
where
    T: Serialize,
{
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{OnboardSummary, build_onboard_steps};

    #[test]
    fn onboard_checklist_reports_missing_chat_model_first() {
        let steps = build_onboard_steps(OnboardSummary {
            chat_ready: false,
            configured_model_count: 0,
            channels_configured: 0,
            channels_connected: 0,
            routing_enabled_rules: 0,
            routing_rules: 0,
            nodes_total: 0,
            nodes_online: 0,
            browser_total: 0,
            browser_logged_in: 0,
            failover_auth_profiles: 0,
            failover_chains: 0,
            failover_cooling_providers: 0,
        });

        assert_eq!(steps[0].id, "chat_model");
        assert_eq!(steps[0].status, "blocking");
        assert!(steps[0].detail.contains("No chat model"));
        assert!(steps[0].next_action.contains("agentark --setup"));
    }

    #[test]
    fn onboard_checklist_marks_chat_model_ready_when_configured() {
        let steps = build_onboard_steps(OnboardSummary {
            chat_ready: true,
            configured_model_count: 2,
            channels_configured: 1,
            channels_connected: 1,
            routing_enabled_rules: 1,
            routing_rules: 1,
            nodes_total: 0,
            nodes_online: 0,
            browser_total: 0,
            browser_logged_in: 0,
            failover_auth_profiles: 0,
            failover_chains: 0,
            failover_cooling_providers: 0,
        });

        assert_eq!(steps[0].id, "chat_model");
        assert_eq!(steps[0].status, "ready");
        assert!(steps[0].detail.contains("2 chat model route(s)"));
        assert!(steps[0].next_action.contains("agentark --chat"));
    }
}
