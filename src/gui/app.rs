//! Main GUI application

use eframe::egui;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::core::orchestra::SubAgentType;
use crate::core::swarm::{AgentCapability, SpecialistConfig};
use crate::core::Agent;
use crate::core::LlmProvider;

/// Active view in the GUI
#[derive(Debug, Clone, PartialEq)]
pub enum ActiveView {
    Chat,
    Tasks,
    Memory,
    Actions,
    Safety,
    Proofs,
    Settings,
}

/// Main GUI application state
pub struct AgentArkApp {
    /// Shared agent instance
    agent: Arc<RwLock<Agent>>,

    /// Tokio runtime for async operations
    runtime: tokio::runtime::Runtime,

    /// Current active view
    active_view: ActiveView,

    /// Chat input
    chat_input: String,

    /// Chat history
    chat_history: Vec<ChatMessage>,

    /// Status message
    status: String,

    /// Pending response receiver
    pending_response: Option<std::sync::mpsc::Receiver<Result<String, String>>>,

    /// Whether the add-agent inline form is open
    show_add_agent_form: bool,
    /// Index of agent currently being edited (None = not editing)
    editing_agent_index: Option<usize>,
    /// Agent form fields (shared between add and edit)
    agent_form: AgentFormState,
}

/// Form state for adding/editing a specialist agent
#[derive(Debug, Clone, Default)]
struct AgentFormState {
    name: String,
    agent_type_index: usize,
    llm_provider_index: usize,
    model: String,
    base_url: String,
    api_key: String,
    capabilities: String,
    description: String,
    system_prompt: String,
}

const AGENT_TYPES: &[&str] = &[
    "Researcher",
    "Coder",
    "Analyst",
    "Writer",
    "Validator",
    "Planner",
    "Custom",
];
const LLM_PROVIDERS: &[&str] = &["Select provider", "Anthropic", "OpenAI", "Ollama"];

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl AgentArkApp {
    pub fn new(agent: Agent) -> Self {
        let runtime = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");

        Self {
            agent: Arc::new(RwLock::new(agent)),
            runtime,
            active_view: ActiveView::Chat,
            chat_input: String::new(),
            chat_history: Vec::new(),
            status: "Ready".to_string(),
            pending_response: None,
            show_add_agent_form: false,
            editing_agent_index: None,
            agent_form: AgentFormState::default(),
        }
    }

    fn render_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.heading(crate::branding::PRODUCT_NAME);
        ui.separator();

        ui.vertical(|ui| {
            if ui
                .selectable_label(self.active_view == ActiveView::Chat, "Chat")
                .clicked()
            {
                self.active_view = ActiveView::Chat;
            }
            if ui
                .selectable_label(self.active_view == ActiveView::Tasks, "Tasks")
                .clicked()
            {
                self.active_view = ActiveView::Tasks;
            }
            if ui
                .selectable_label(self.active_view == ActiveView::Memory, "Memory")
                .clicked()
            {
                self.active_view = ActiveView::Memory;
            }
            if ui
                .selectable_label(self.active_view == ActiveView::Actions, "Actions")
                .clicked()
            {
                self.active_view = ActiveView::Actions;
            }
            if ui
                .selectable_label(self.active_view == ActiveView::Safety, "Safety")
                .clicked()
            {
                self.active_view = ActiveView::Safety;
            }
            if ui
                .selectable_label(self.active_view == ActiveView::Proofs, "Proofs")
                .clicked()
            {
                self.active_view = ActiveView::Proofs;
            }
            ui.separator();
            if ui
                .selectable_label(self.active_view == ActiveView::Settings, "Settings")
                .clicked()
            {
                self.active_view = ActiveView::Settings;
            }
        });

        ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status);
            });
        });
    }

    fn render_chat(&mut self, ui: &mut egui::Ui) {
        ui.heading("Chat");
        ui.separator();

        // Check for pending response
        if let Some(rx) = &self.pending_response {
            if let Ok(result) = rx.try_recv() {
                match result {
                    Ok(response) => {
                        self.chat_history.push(ChatMessage {
                            role: "assistant".to_string(),
                            content: response,
                            timestamp: chrono::Utc::now(),
                        });
                    }
                    Err(e) => {
                        self.chat_history.push(ChatMessage {
                            role: "system".to_string(),
                            content: format!("Error: {}", e),
                            timestamp: chrono::Utc::now(),
                        });
                    }
                }
                self.pending_response = None;
                self.status = "Ready".to_string();
            }
        }

        // Chat history
        let available_height = ui.available_height() - 60.0;
        egui::ScrollArea::vertical()
            .max_height(available_height)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for msg in &self.chat_history {
                    let (bg_color, align) = if msg.role == "user" {
                        (egui::Color32::from_rgb(60, 60, 80), egui::Align::RIGHT)
                    } else {
                        (egui::Color32::from_rgb(40, 60, 40), egui::Align::LEFT)
                    };

                    ui.with_layout(egui::Layout::top_down(align), |ui| {
                        egui::Frame::none()
                            .fill(bg_color)
                            .rounding(8.0)
                            .inner_margin(10.0)
                            .show(ui, |ui| {
                                ui.label(&msg.content);
                                ui.small(msg.timestamp.format("%H:%M").to_string());
                            });
                    });
                    ui.add_space(5.0);
                }
            });

        // Input area
        ui.separator();
        ui.horizontal(|ui| {
            let input = egui::TextEdit::singleline(&mut self.chat_input)
                .hint_text("Type a message...")
                .desired_width(ui.available_width() - 80.0);

            let response = ui.add(input);

            let send_clicked = ui.button("Send").clicked();
            let enter_pressed =
                response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

            if (send_clicked || enter_pressed)
                && !self.chat_input.is_empty()
                && self.pending_response.is_none()
            {
                let message = self.chat_input.clone();
                self.chat_input.clear();

                // Add user message to history
                self.chat_history.push(ChatMessage {
                    role: "user".to_string(),
                    content: message.clone(),
                    timestamp: chrono::Utc::now(),
                });

                self.status = "Processing...".to_string();

                // Process message synchronously using block_on
                // This is simpler and avoids Send issues
                let (tx, rx) = std::sync::mpsc::channel();
                self.pending_response = Some(rx);

                let agent = self.agent.clone();
                std::thread::spawn(move || {
                    let rt = tokio::runtime::Runtime::new().unwrap();
                    let result = rt.block_on(async {
                        let agent = agent.read().await;
                        agent.process_message(&message, "gui", None, None).await
                    });
                    let _ = tx.send(result.map_err(|e| e.to_string()));
                });
            }
        });
    }

    fn render_tasks(&mut self, ui: &mut egui::Ui) {
        ui.heading("Tasks");
        ui.separator();

        let tasks = self.runtime.block_on(async {
            let agent = self.agent.read().await;
            let tasks = agent.tasks.read().await;
            tasks.all().to_vec()
        });

        if tasks.is_empty() {
            ui.label("No tasks");
            return;
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            for task in tasks {
                egui::Frame::none()
                    .fill(egui::Color32::from_rgb(40, 40, 50))
                    .rounding(5.0)
                    .inner_margin(10.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let status_emoji = match &task.status {
                                crate::core::TaskStatus::Pending => "O",
                                crate::core::TaskStatus::AwaitingApproval => "!",
                                crate::core::TaskStatus::ExpiredNeedsReapproval => "?",
                                crate::core::TaskStatus::Paused => "||",
                                crate::core::TaskStatus::InProgress => "*",
                                crate::core::TaskStatus::Completed => "+",
                                crate::core::TaskStatus::Failed { .. } => "X",
                                crate::core::TaskStatus::Cancelled => "-",
                            };
                            ui.label(status_emoji);
                            ui.strong(&task.description);
                        });
                        ui.label(format!("Action: {}", task.action));
                    });
                ui.add_space(5.0);
            }
        });
    }

    fn render_memory(&mut self, ui: &mut egui::Ui) {
        ui.heading("Memory");
        ui.separator();

        let count = self.runtime.block_on(async {
            let agent = self.agent.read().await;
            agent.storage.count_facts(None).await.unwrap_or(0) as usize
        });

        ui.label(format!("Total learned facts: {}", count));

        ui.separator();
        ui.label("Durable memory surfaces:");
        ui.horizontal(|ui| {
            ui.label("Facts");
            ui.label("Preferences");
            ui.label("Knowledge");
        });
    }

    fn render_actions(&mut self, ui: &mut egui::Ui) {
        ui.heading("Actions");
        ui.separator();

        let actions = self.runtime.block_on(async {
            let agent = self.agent.read().await;
            agent.runtime.list_actions().await.unwrap_or_default()
        });

        egui::ScrollArea::vertical().show(ui, |ui| {
            for action in actions {
                egui::Frame::none()
                    .fill(egui::Color32::from_rgb(40, 50, 40))
                    .rounding(5.0)
                    .inner_margin(10.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.strong(&action.name);
                            ui.small(format!("v{}", action.version));
                        });
                        ui.label(&action.description);
                    });
                ui.add_space(5.0);
            }
        });
    }

    fn render_safety(&mut self, ui: &mut egui::Ui) {
        ui.heading("Safety Rules");
        ui.separator();

        let rules = self.runtime.block_on(async {
            let agent = self.agent.read().await;
            agent.safety.rules()
        });

        egui::ScrollArea::vertical().show(ui, |ui| {
            for rule in rules {
                egui::Frame::none()
                    .fill(egui::Color32::from_rgb(50, 40, 40))
                    .rounding(5.0)
                    .inner_margin(10.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let verified = if rule.verified { "+" } else { "?" };
                            ui.label(verified);
                            ui.strong(&rule.name);
                        });
                        ui.label(&rule.description);
                    });
                ui.add_space(5.0);
            }
        });
    }

    fn render_proofs(&mut self, ui: &mut egui::Ui) {
        ui.heading("Execution Proofs");
        ui.separator();

        let (proof_count, receipts) = self.runtime.block_on(async {
            let agent = self.agent.read().await;
            let trace = agent.proofs.trace();
            let count = trace.proofs.len();
            let receipts: Vec<_> = trace
                .proofs
                .iter()
                .rev()
                .take(20)
                .map(crate::proofs::ProofReceipt::from)
                .collect();
            (count, receipts)
        });

        ui.label(format!("Total proofs: {}", proof_count));

        ui.separator();

        egui::ScrollArea::vertical().show(ui, |ui| {
            for receipt in receipts {
                egui::Frame::none()
                    .fill(egui::Color32::from_rgb(40, 40, 60))
                    .rounding(5.0)
                    .inner_margin(10.0)
                    .show(ui, |ui| {
                        ui.label(format!("ID: {}", receipt.proof_id));
                        ui.small(format!("Time: {}", receipt.timestamp));
                        ui.small(format!(
                            "Hash: {}...",
                            &receipt.proof_hash.chars().take(16).collect::<String>()
                        ));
                    });
                ui.add_space(5.0);
            }
        });
    }

    fn render_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Settings");
        ui.separator();

        let (did, has_telegram) = self.runtime.block_on(async {
            let agent = self.agent.read().await;
            (
                agent.identity.did().to_string(),
                agent.config.telegram.is_some(),
            )
        });

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.group(|ui| {
                ui.label("Identity");
                ui.label(format!("DID: {}", did));
            });

            ui.group(|ui| {
                ui.label("LLM Provider");
                ui.label("Configure in Settings");
            });

            ui.group(|ui| {
                ui.label("Telegram");
                if has_telegram {
                    ui.label("Configured");
                } else {
                    ui.label("Not configured");
                }
            });

            ui.add_space(10.0);
            self.render_swarm_settings(ui);
        });
    }

    fn render_swarm_settings(&mut self, ui: &mut egui::Ui) {
        // Read current swarm config
        let swarm_config = self.runtime.block_on(async {
            let agent = self.agent.read().await;
            agent.config.swarm.clone()
        });

        ui.group(|ui| {
            // Header
            ui.horizontal(|ui| {
                ui.heading("Custom Specialist Agents");
                ui.label(" - Multi-agent coordination for complex tasks");
            });
            ui.separator();

            // Status row
            ui.horizontal(|ui| {
                ui.label("Swarm Status:");
                ui.colored_label(egui::Color32::from_rgb(100, 200, 100), "Active");

                ui.separator();

                ui.label(format!("{} Total Agents", swarm_config.specialists.len()));
                let active = swarm_config
                    .specialists
                    .iter()
                    .filter(|s| s.enabled)
                    .count();
                ui.label(format!("{} Active", active));
            });

            ui.add_space(10.0);
            ui.separator();

            // Add Agent button
            ui.horizontal(|ui| {
                ui.strong("Specialist Agents");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !self.show_add_agent_form
                        && self.editing_agent_index.is_none()
                        && ui.button("+ Add Specialist Agent").clicked()
                    {
                        self.agent_form = AgentFormState::default();
                        self.show_add_agent_form = true;
                    }
                });
            });

            ui.add_space(5.0);

            // Inline add form
            if self.show_add_agent_form {
                self.render_agent_form(ui, "Add Specialist Agent");

                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        if let Some(config) = self.build_specialist_config() {
                            self.runtime.block_on(async {
                                let mut agent = self.agent.write().await;
                                agent.config.swarm.specialists.push(config);
                            });
                            self.show_add_agent_form = false;
                            self.agent_form = AgentFormState::default();
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        self.show_add_agent_form = false;
                        self.agent_form = AgentFormState::default();
                    }
                });
                ui.separator();
            }

            // List existing agents
            let specialists = swarm_config.specialists.clone();
            if specialists.is_empty() && !self.show_add_agent_form {
                ui.add_space(10.0);
                ui.vertical_centered(|ui| {
                    ui.label("No specialist agents configured.");
                    ui.label("Click \"+ Add Specialist Agent\" to get started.");
                });
            }

            let mut remove_index: Option<usize> = None;
            let mut save_edit_index: Option<usize> = None;

            for (i, spec) in specialists.iter().enumerate() {
                let is_editing = self.editing_agent_index == Some(i);

                egui::Frame::none()
                    .fill(egui::Color32::from_rgb(35, 35, 45))
                    .rounding(6.0)
                    .inner_margin(10.0)
                    .outer_margin(egui::Margin::symmetric(0.0, 3.0))
                    .show(ui, |ui| {
                        if is_editing {
                            // Edit mode
                            self.render_agent_form(ui, &format!("Edit: {}", spec.name));

                            ui.horizontal(|ui| {
                                if ui.button("Save").clicked() {
                                    save_edit_index = Some(i);
                                }
                                if ui.button("Cancel").clicked() {
                                    self.editing_agent_index = None;
                                    self.agent_form = AgentFormState::default();
                                }
                            });
                        } else {
                            // Display mode
                            ui.horizontal(|ui| {
                                // Enabled indicator
                                let indicator = if spec.enabled { "●" } else { "○" };
                                let color = if spec.enabled {
                                    egui::Color32::from_rgb(100, 200, 100)
                                } else {
                                    egui::Color32::from_rgb(150, 150, 150)
                                };
                                ui.colored_label(color, indicator);

                                ui.strong(&spec.name);

                                ui.label(format!("({:?})", spec.agent_type));

                                let model = match &spec.llm_provider {
                                    LlmProvider::Anthropic { model, .. } => {
                                        format!("Anthropic/{}", model)
                                    }
                                    LlmProvider::OpenAI { model, .. } => {
                                        format!("OpenAI/{}", model)
                                    }
                                    LlmProvider::Ollama { model, .. } => {
                                        format!("Ollama/{}", model)
                                    }
                                };
                                ui.label(model);

                                // Action buttons on the right
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.small_button("Delete").clicked() {
                                            remove_index = Some(i);
                                        }
                                        if ui.small_button("Edit").clicked() {
                                            self.populate_form_from_config(spec);
                                            self.editing_agent_index = Some(i);
                                            self.show_add_agent_form = false;
                                        }
                                    },
                                );
                            });

                            // Capabilities row
                            if !spec.capabilities.is_empty() {
                                ui.horizontal(|ui| {
                                    ui.add_space(20.0);
                                    ui.label("Capabilities:");
                                    for cap in &spec.capabilities {
                                        egui::Frame::none()
                                            .fill(egui::Color32::from_rgb(50, 50, 70))
                                            .rounding(3.0)
                                            .inner_margin(egui::Margin::symmetric(6.0, 2.0))
                                            .show(ui, |ui| {
                                                ui.small(&cap.name);
                                            });
                                    }
                                });
                            }
                        }
                    });
            }

            // Apply deferred mutations
            if let Some(idx) = remove_index {
                self.runtime.block_on(async {
                    let mut agent = self.agent.write().await;
                    if idx < agent.config.swarm.specialists.len() {
                        agent.config.swarm.specialists.remove(idx);
                    }
                });
            }

            if let Some(idx) = save_edit_index {
                if let Some(config) = self.build_specialist_config() {
                    self.runtime.block_on(async {
                        let mut agent = self.agent.write().await;
                        if idx < agent.config.swarm.specialists.len() {
                            agent.config.swarm.specialists[idx] = config;
                        }
                    });
                    self.editing_agent_index = None;
                    self.agent_form = AgentFormState::default();
                }
            }
        });
    }

    /// Render the agent add/edit form fields
    fn render_agent_form(&mut self, ui: &mut egui::Ui, title: &str) {
        ui.strong(title);
        ui.add_space(5.0);

        egui::Grid::new(format!("agent_form_{}", title))
            .num_columns(2)
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                ui.label("Agent Name:");
                ui.text_edit_singleline(&mut self.agent_form.name);
                ui.end_row();

                ui.label("Agent Type:");
                egui::ComboBox::from_id_salt(format!("agent_type_{}", title))
                    .selected_text(AGENT_TYPES[self.agent_form.agent_type_index])
                    .show_ui(ui, |ui| {
                        for (idx, label) in AGENT_TYPES.iter().enumerate() {
                            ui.selectable_value(&mut self.agent_form.agent_type_index, idx, *label);
                        }
                    });
                ui.end_row();

                ui.label("LLM Provider:");
                egui::ComboBox::from_id_salt(format!("llm_provider_{}", title))
                    .selected_text(LLM_PROVIDERS[self.agent_form.llm_provider_index])
                    .show_ui(ui, |ui| {
                        for (idx, label) in LLM_PROVIDERS.iter().enumerate() {
                            ui.selectable_value(
                                &mut self.agent_form.llm_provider_index,
                                idx,
                                *label,
                            );
                        }
                    });
                ui.end_row();

                ui.label("Model:");
                ui.text_edit_singleline(&mut self.agent_form.model);
                ui.end_row();

                // Show base URL for OpenAI/Ollama
                if matches!(self.agent_form.llm_provider_index, 2 | 3) {
                    ui.label("Base URL:");
                    ui.text_edit_singleline(&mut self.agent_form.base_url);
                    ui.end_row();
                }

                // Show API key for Anthropic/OpenAI
                if matches!(self.agent_form.llm_provider_index, 1 | 2) {
                    ui.label("API Key:");
                    ui.add(egui::TextEdit::singleline(&mut self.agent_form.api_key).password(true));
                    ui.end_row();
                }

                ui.label("Capabilities:");
                ui.text_edit_singleline(&mut self.agent_form.capabilities)
                    .on_hover_text("Comma-separated capability names");
                ui.end_row();

                ui.label("Description:");
                ui.text_edit_singleline(&mut self.agent_form.description)
                    .on_hover_text("Describe what this agent is good at");
                ui.end_row();

                ui.label("System Prompt:");
                ui.add(
                    egui::TextEdit::multiline(&mut self.agent_form.system_prompt)
                        .desired_rows(3)
                        .hint_text("Optional override"),
                );
                ui.end_row();
            });
        ui.add_space(5.0);
    }

    /// Build a SpecialistConfig from the current form state
    fn build_specialist_config(&self) -> Option<SpecialistConfig> {
        let name = self.agent_form.name.trim();
        if name.is_empty() || self.agent_form.llm_provider_index == 0 {
            return None;
        }

        let agent_type = match self.agent_form.agent_type_index {
            0 => SubAgentType::Researcher,
            1 => SubAgentType::Coder,
            2 => SubAgentType::Analyst,
            3 => SubAgentType::Writer,
            4 => SubAgentType::Validator,
            5 => SubAgentType::Planner,
            6 => SubAgentType::Custom {
                name: name.to_string(),
                instructions: self.agent_form.system_prompt.clone(),
            },
            _ => SubAgentType::Researcher,
        };

        let model = self.agent_form.model.trim().to_string();
        if model.is_empty() {
            return None;
        }

        let llm_provider = match self.agent_form.llm_provider_index {
            1 => LlmProvider::Anthropic {
                api_key: self.agent_form.api_key.clone(),
                model,
            },
            2 => LlmProvider::OpenAI {
                api_key: self.agent_form.api_key.clone(),
                model,
                base_url: if self.agent_form.base_url.trim().is_empty() {
                    None
                } else {
                    Some(self.agent_form.base_url.trim().to_string())
                },
            },
            3 => LlmProvider::Ollama {
                base_url: self.agent_form.base_url.trim().to_string(),
                model,
            },
            _ => return None,
        };

        let capabilities: Vec<AgentCapability> = self
            .agent_form
            .capabilities
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(|name| AgentCapability {
                name: name.clone(),
                description: self.agent_form.description.clone(),
                keywords: name
                    .to_lowercase()
                    .split_whitespace()
                    .map(String::from)
                    .collect(),
            })
            .collect();

        let system_prompt_override = if self.agent_form.system_prompt.trim().is_empty() {
            None
        } else {
            Some(self.agent_form.system_prompt.clone())
        };

        Some(SpecialistConfig {
            id: None,
            name: name.to_string(),
            agent_type,
            llm_provider,
            system_prompt_override,
            max_memory_retrieval: 3,
            capabilities,
            access_scope: crate::core::swarm::AgentAccessScope::default(),
            enabled: true,
        })
    }

    /// Populate form fields from an existing SpecialistConfig
    fn populate_form_from_config(&mut self, config: &SpecialistConfig) {
        self.agent_form.name = config.name.clone();

        self.agent_form.agent_type_index = match &config.agent_type {
            SubAgentType::Researcher => 0,
            SubAgentType::Coder => 1,
            SubAgentType::Analyst => 2,
            SubAgentType::Writer => 3,
            SubAgentType::Validator => 4,
            SubAgentType::Planner => 5,
            SubAgentType::Custom { .. } => 6,
        };

        match &config.llm_provider {
            LlmProvider::Anthropic { api_key, model } => {
                self.agent_form.llm_provider_index = 1;
                self.agent_form.model = model.clone();
                self.agent_form.api_key = api_key.clone();
                self.agent_form.base_url.clear();
            }
            LlmProvider::OpenAI {
                api_key,
                model,
                base_url,
            } => {
                self.agent_form.llm_provider_index = 2;
                self.agent_form.model = model.clone();
                self.agent_form.api_key = api_key.clone();
                self.agent_form.base_url = base_url.clone().unwrap_or_default();
            }
            LlmProvider::Ollama { base_url, model } => {
                self.agent_form.llm_provider_index = 3;
                self.agent_form.model = model.clone();
                self.agent_form.api_key.clear();
                self.agent_form.base_url = base_url.clone();
            }
        }

        self.agent_form.capabilities = config
            .capabilities
            .iter()
            .map(|c| c.name.clone())
            .collect::<Vec<_>>()
            .join(", ");

        self.agent_form.description = config
            .capabilities
            .first()
            .map(|c| c.description.clone())
            .unwrap_or_default();

        self.agent_form.system_prompt = config.system_prompt_override.clone().unwrap_or_default();
    }
}

impl eframe::App for AgentArkApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Sidebar
        egui::SidePanel::left("sidebar")
            .min_width(150.0)
            .max_width(200.0)
            .show(ctx, |ui| {
                self.render_sidebar(ui);
            });

        // Main content
        egui::CentralPanel::default().show(ctx, |ui| match self.active_view {
            ActiveView::Chat => self.render_chat(ui),
            ActiveView::Tasks => self.render_tasks(ui),
            ActiveView::Memory => self.render_memory(ui),
            ActiveView::Actions => self.render_actions(ui),
            ActiveView::Safety => self.render_safety(ui),
            ActiveView::Proofs => self.render_proofs(ui),
            ActiveView::Settings => self.render_settings(ui),
        });

        // Request repaint for animations
        if self.pending_response.is_some() {
            ctx.request_repaint();
        }
    }
}
