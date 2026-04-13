//! GUI views and components

use eframe::egui;

use crate::actions::SearchConfig;
use crate::core::{Agent, LlmProvider};

/// Setup wizard for first-time configuration
pub struct SetupWizard {
    agent: Agent,
    step: SetupStep,

    // Form state
    agent_name: String,
    llm_provider: LlmProviderChoice,
    api_key: String,
    ollama_url: String,
    ollama_model: String,
    openai_base_url: String,
    telegram_token: String,
    telegram_users: String,

    // Search backends
    enable_serper: bool,
    serper_key: String,
    enable_brave: bool,
    brave_key: String,

    // Status
    error: Option<String>,
    _testing_connection: bool,
    connection_result: Option<Result<String, String>>,
}

#[derive(Debug, Clone, PartialEq)]
enum SetupStep {
    Welcome,
    LlmConfig,
    SearchConfig,
    TelegramConfig,
    Review,
    Complete,
}

#[derive(Debug, Clone, PartialEq)]
enum LlmProviderChoice {
    Unset,
    Anthropic,
    OpenAI,
    OpenAICompatible,
    Ollama,
}

impl SetupWizard {
    pub fn new(agent: Agent) -> Self {
        // Try to load existing config values
        let (llm_provider, api_key, ollama_url, ollama_model, openai_base_url) =
            match &agent.config.llm {
                LlmProvider::Anthropic { api_key, .. } => (
                    LlmProviderChoice::Anthropic,
                    api_key.clone(),
                    String::new(),
                    String::new(),
                    String::new(),
                ),
                LlmProvider::OpenAI {
                    api_key,
                    model,
                    base_url,
                } => {
                    if base_url.is_some() {
                        (
                            LlmProviderChoice::OpenAICompatible,
                            api_key.clone(),
                            String::new(),
                            model.clone(),
                            base_url.clone().unwrap_or_default(),
                        )
                    } else {
                        (
                            LlmProviderChoice::OpenAI,
                            api_key.clone(),
                            String::new(),
                            model.clone(),
                            String::new(),
                        )
                    }
                }
                LlmProvider::Ollama { base_url, model }
                    if base_url.trim().is_empty() && model.trim().is_empty() =>
                {
                    (
                        LlmProviderChoice::Unset,
                        String::new(),
                        String::new(),
                        String::new(),
                        String::new(),
                    )
                }
                LlmProvider::Ollama { base_url, model } => (
                    LlmProviderChoice::Ollama,
                    String::new(),
                    base_url.clone(),
                    model.clone(),
                    String::new(),
                ),
            };

        let (telegram_token, telegram_users) = match &agent.config.telegram {
            Some(t) => (
                t.bot_token.clone(),
                t.allowed_users
                    .iter()
                    .map(|u| u.to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            None => (String::new(), String::new()),
        };

        Self {
            agent,
            step: SetupStep::Welcome,
            agent_name: crate::branding::default_agent_name(),
            llm_provider,
            api_key,
            ollama_url,
            ollama_model,
            openai_base_url,
            telegram_token,
            telegram_users,
            enable_serper: false,
            serper_key: String::new(),
            enable_brave: false,
            brave_key: String::new(),
            error: None,
            _testing_connection: false,
            connection_result: None,
        }
    }

    fn render_welcome(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(30.0);

            // Logo/Title
            ui.heading(egui::RichText::new(crate::branding::PRODUCT_NAME).size(32.0));
            ui.add_space(10.0);
            ui.label(egui::RichText::new("Secure, Self-Improving AI Assistant").italics());

            ui.add_space(30.0);

            // Feature highlights
            egui::Frame::none()
                .fill(egui::Color32::from_rgb(40, 45, 50))
                .rounding(10.0)
                .inner_margin(20.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new("[+] Cognitive Memory").strong());
                            ui.small("Episodic, semantic & procedural");
                        });
                        ui.add_space(20.0);
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new("[+] Cryptographic Proofs").strong());
                            ui.small("Verifiable execution history");
                        });
                    });
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new("[+] Sandboxed Execution").strong());
                            ui.small("WASM & Docker isolation");
                        });
                        ui.add_space(20.0);
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new("[+] Web Research").strong());
                            ui.small("Multi-source search & analysis");
                        });
                    });
                });

            ui.add_space(20.0);

            // Identity info
            egui::Frame::none()
                .fill(egui::Color32::from_rgb(35, 50, 45))
                .rounding(8.0)
                .inner_margin(15.0)
                .show(ui, |ui| {
                    ui.label("Your Agent Identity (DID):");
                    ui.add_space(5.0);
                    let did = self.agent.identity.did();
                    ui.label(egui::RichText::new(did).monospace().size(11.0));
                });

            ui.add_space(30.0);

            if ui
                .add(
                    egui::Button::new(egui::RichText::new("  Get Started  ").size(16.0))
                        .min_size(egui::vec2(150.0, 40.0)),
                )
                .clicked()
            {
                self.step = SetupStep::LlmConfig;
            }

            ui.add_space(10.0);
            ui.small("This wizard will help you configure your agent");
        });
    }

    fn render_llm_config(&mut self, ui: &mut egui::Ui) {
        ui.heading("Step 1: LLM Configuration");
        ui.separator();
        ui.add_space(10.0);

        ui.label("Choose how your agent will think:");
        ui.add_space(15.0);

        // Provider selection with descriptions
        egui::Grid::new("llm_grid")
            .num_columns(2)
            .spacing([20.0, 10.0])
            .show(ui, |ui| {
                let unset_selected = self.llm_provider == LlmProviderChoice::Unset;
                if ui
                    .add(egui::SelectableLabel::new(
                        unset_selected,
                        egui::RichText::new("Choose Later").strong(),
                    ))
                    .clicked()
                {
                    self.llm_provider = LlmProviderChoice::Unset;
                    self.connection_result = None;
                }
                ui.label("Leave models unconfigured for now");
                ui.end_row();

                // Ollama (Local)
                let ollama_selected = self.llm_provider == LlmProviderChoice::Ollama;
                if ui
                    .add(egui::SelectableLabel::new(
                        ollama_selected,
                        egui::RichText::new("Ollama (Local)").strong(),
                    ))
                    .clicked()
                {
                    self.llm_provider = LlmProviderChoice::Ollama;
                    self.connection_result = None;
                }
                ui.label("Free, private, runs on your machine");
                ui.end_row();

                // Anthropic
                let anthropic_selected = self.llm_provider == LlmProviderChoice::Anthropic;
                if ui
                    .add(egui::SelectableLabel::new(
                        anthropic_selected,
                        egui::RichText::new("Anthropic Claude").strong(),
                    ))
                    .clicked()
                {
                    self.llm_provider = LlmProviderChoice::Anthropic;
                    self.connection_result = None;
                }
                ui.label("Most capable, requires API key");
                ui.end_row();

                // OpenAI
                let openai_selected = self.llm_provider == LlmProviderChoice::OpenAI;
                if ui
                    .add(egui::SelectableLabel::new(
                        openai_selected,
                        egui::RichText::new("OpenAI GPT").strong(),
                    ))
                    .clicked()
                {
                    self.llm_provider = LlmProviderChoice::OpenAI;
                    self.connection_result = None;
                }
                ui.label("Popular, requires API key");
                ui.end_row();

                // OpenAI-Compatible
                let compat_selected = self.llm_provider == LlmProviderChoice::OpenAICompatible;
                if ui
                    .add(egui::SelectableLabel::new(
                        compat_selected,
                        egui::RichText::new("OpenAI-Compatible").strong(),
                    ))
                    .clicked()
                {
                    self.llm_provider = LlmProviderChoice::OpenAICompatible;
                    self.connection_result = None;
                }
                ui.label("LMStudio, vLLM, etc.");
                ui.end_row();
            });

        ui.add_space(20.0);
        ui.separator();
        ui.add_space(10.0);

        // Provider-specific settings
        match self.llm_provider {
            LlmProviderChoice::Unset => {
                ui.label("No model is configured yet. You can finish setup now and add one later.");
            }
            LlmProviderChoice::Anthropic => {
                ui.horizontal(|ui| {
                    ui.label("API Key: ");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.api_key)
                            .password(true)
                            .hint_text("sk-ant-...")
                            .desired_width(300.0),
                    );
                });
                ui.add_space(5.0);
                ui.horizontal(|ui| {
                    ui.label("Model: ");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.ollama_model)
                            .hint_text("claude-sonnet-4-5")
                            .desired_width(250.0),
                    );
                });
                ui.add_space(5.0);
                ui.horizontal(|ui| {
                    ui.small("Get your API key from ");
                    ui.hyperlink_to("console.anthropic.com", "https://console.anthropic.com");
                });
            }
            LlmProviderChoice::OpenAI => {
                ui.horizontal(|ui| {
                    ui.label("API Key: ");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.api_key)
                            .password(true)
                            .hint_text("sk-...")
                            .desired_width(300.0),
                    );
                });
                ui.add_space(5.0);
                ui.horizontal(|ui| {
                    ui.label("Model: ");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.ollama_model)
                            .hint_text("gpt-4.1")
                            .desired_width(250.0),
                    );
                });
                ui.add_space(5.0);
                ui.horizontal(|ui| {
                    ui.small("Get your API key from ");
                    ui.hyperlink_to("platform.openai.com", "https://platform.openai.com");
                });
            }
            LlmProviderChoice::OpenAICompatible => {
                ui.horizontal(|ui| {
                    ui.label("Base URL: ");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.openai_base_url)
                            .hint_text("http://localhost:1234/v1")
                            .desired_width(300.0),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("Model: ");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.ollama_model)
                            .hint_text("local-model")
                            .desired_width(200.0),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("API Key (if required): ");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.api_key)
                            .password(true)
                            .hint_text("optional")
                            .desired_width(200.0),
                    );
                });
            }
            LlmProviderChoice::Ollama => {
                ui.horizontal(|ui| {
                    ui.label("Ollama URL: ");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.ollama_url)
                            .hint_text("http://localhost:11434")
                            .desired_width(250.0),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("Model: ");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.ollama_model)
                            .hint_text("example: llama3.2")
                            .desired_width(150.0),
                    );
                });
                ui.add_space(5.0);
                ui.small("Make sure Ollama is running: ollama serve");
                ui.small("Example: ollama pull llama3.2");
            }
        }

        // Connection test
        ui.add_space(15.0);
        if let Some(result) = &self.connection_result {
            match result {
                Ok(msg) => {
                    ui.colored_label(egui::Color32::GREEN, format!("[OK] {}", msg));
                }
                Err(msg) => {
                    ui.colored_label(egui::Color32::RED, format!("[ERROR] {}", msg));
                }
            }
        }

        ui.add_space(20.0);

        // Navigation
        ui.horizontal(|ui| {
            if ui.button("<< Back").clicked() {
                self.step = SetupStep::Welcome;
                self.connection_result = None;
            }
            ui.add_space(20.0);
            if ui.button("Next >>").clicked() {
                self.step = SetupStep::SearchConfig;
            }
        });
    }

    fn render_search_config(&mut self, ui: &mut egui::Ui) {
        ui.heading("Step 2: Web Search Configuration");
        ui.separator();
        ui.add_space(10.0);

        ui.label("Configure search backends for research capabilities.");
        ui.small("DuckDuckGo is always available as a fallback (no API key needed).");
        ui.add_space(15.0);

        // Serper
        egui::Frame::none()
            .fill(egui::Color32::from_rgb(40, 45, 50))
            .rounding(8.0)
            .inner_margin(10.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.enable_serper, "");
                    ui.label(egui::RichText::new("Serper").strong());
                    ui.small("(Google results via API)");
                });
                if self.enable_serper {
                    ui.horizontal(|ui| {
                        ui.label("API Key: ");
                        ui.add(egui::TextEdit::singleline(&mut self.serper_key).password(true));
                    });
                    ui.horizontal(|ui| {
                        ui.small("Get a key from ");
                        ui.hyperlink_to("serper.dev", "https://serper.dev");
                    });
                }
            });

        ui.add_space(10.0);

        // Brave
        egui::Frame::none()
            .fill(egui::Color32::from_rgb(40, 45, 50))
            .rounding(8.0)
            .inner_margin(10.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.enable_brave, "");
                    ui.label(egui::RichText::new("Brave Search").strong());
                    ui.small("(Privacy-focused API)");
                });
                if self.enable_brave {
                    ui.horizontal(|ui| {
                        ui.label("API Key: ");
                        ui.add(egui::TextEdit::singleline(&mut self.brave_key).password(true));
                    });
                    ui.horizontal(|ui| {
                        ui.small("Get a key from ");
                        ui.hyperlink_to("brave.com/search/api", "https://brave.com/search/api");
                    });
                }
            });

        ui.add_space(20.0);

        // Navigation
        ui.horizontal(|ui| {
            if ui.button("<< Back").clicked() {
                self.step = SetupStep::LlmConfig;
            }
            ui.add_space(20.0);
            if ui.button("Skip").clicked() {
                self.step = SetupStep::TelegramConfig;
            }
            if ui.button("Next >>").clicked() {
                self.step = SetupStep::TelegramConfig;
            }
        });
    }

    fn render_telegram_config(&mut self, ui: &mut egui::Ui) {
        ui.heading("Step 3: Telegram Configuration (Optional)");
        ui.separator();
        ui.add_space(10.0);

        ui.label("Connect your agent to Telegram for mobile access.");
        ui.add_space(15.0);

        egui::Frame::none()
            .fill(egui::Color32::from_rgb(40, 45, 50))
            .rounding(8.0)
            .inner_margin(15.0)
            .show(ui, |ui| {
                ui.label("How to set up:");
                ui.add_space(5.0);
                ui.label("1. Open Telegram and search for @BotFather");
                ui.label("2. Send /newbot and follow the prompts");
                ui.label("3. Copy the bot token below");
                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    ui.label("Bot Token: ");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.telegram_token)
                            .password(true)
                            .hint_text("123456789:ABC...")
                            .desired_width(300.0),
                    );
                });

                ui.add_space(10.0);

                ui.label("Allowed User IDs (optional, comma-separated):");
                ui.add(
                    egui::TextEdit::singleline(&mut self.telegram_users)
                        .hint_text("123456789, 987654321")
                        .desired_width(300.0),
                );
                ui.small("Leave empty to use pairing mode (first user to message gets paired)");
                ui.small("Get your ID from @userinfobot on Telegram");
            });

        ui.add_space(20.0);

        // Navigation
        ui.horizontal(|ui| {
            if ui.button("<< Back").clicked() {
                self.step = SetupStep::SearchConfig;
            }
            ui.add_space(20.0);
            if ui.button("Skip").clicked() {
                self.telegram_token.clear();
                self.step = SetupStep::Review;
            }
            if ui.button("Next >>").clicked() {
                self.step = SetupStep::Review;
            }
        });
    }

    fn render_review(&mut self, ui: &mut egui::Ui) {
        ui.heading("Step 4: Review Configuration");
        ui.separator();
        ui.add_space(10.0);

        // LLM Summary
        egui::Frame::none()
            .fill(egui::Color32::from_rgb(35, 50, 45))
            .rounding(8.0)
            .inner_margin(12.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("[+]").strong());
                    ui.label("LLM Provider: ");
                    match self.llm_provider {
                        LlmProviderChoice::Unset => {
                            ui.label("Not configured yet");
                        }
                        LlmProviderChoice::Anthropic => {
                            ui.label(format!("Anthropic Claude ({})", self.ollama_model));
                            if self.api_key.is_empty() {
                                ui.colored_label(egui::Color32::YELLOW, "(no API key)");
                            }
                        }
                        LlmProviderChoice::OpenAI => {
                            ui.label(format!("OpenAI GPT ({})", self.ollama_model));
                            if self.api_key.is_empty() {
                                ui.colored_label(egui::Color32::YELLOW, "(no API key)");
                            }
                        }
                        LlmProviderChoice::OpenAICompatible => {
                            ui.label(format!("OpenAI-Compatible at {}", self.openai_base_url));
                        }
                        LlmProviderChoice::Ollama => {
                            ui.label(format!(
                                "Ollama ({}) at {}",
                                self.ollama_model, self.ollama_url
                            ));
                        }
                    };
                });
            });

        ui.add_space(8.0);

        // Search Summary
        egui::Frame::none()
            .fill(egui::Color32::from_rgb(35, 45, 50))
            .rounding(8.0)
            .inner_margin(12.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("[+]").strong());
                    ui.label("Search Backends: ");
                    let mut backends = vec!["DuckDuckGo"];
                    if self.enable_serper {
                        backends.push("Serper");
                    }
                    if self.enable_brave {
                        backends.push("Brave");
                    }
                    ui.label(backends.join(", "));
                });
            });

        ui.add_space(8.0);

        // Telegram Summary
        egui::Frame::none()
            .fill(egui::Color32::from_rgb(50, 45, 35))
            .rounding(8.0)
            .inner_margin(12.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if self.telegram_token.is_empty() {
                        ui.label(egui::RichText::new("[-]").strong());
                        ui.label("Telegram: Not configured");
                    } else {
                        ui.label(egui::RichText::new("[+]").strong());
                        ui.label("Telegram: Configured");
                    }
                });
            });

        ui.add_space(8.0);

        // Identity Summary
        egui::Frame::none()
            .fill(egui::Color32::from_rgb(40, 40, 50))
            .rounding(8.0)
            .inner_margin(12.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("[+]").strong());
                    ui.label("Identity (DID): ");
                });
                ui.label(
                    egui::RichText::new(self.agent.identity.did())
                        .monospace()
                        .size(10.0),
                );
            });

        if let Some(err) = &self.error {
            ui.add_space(10.0);
            ui.colored_label(egui::Color32::RED, format!("Error: {}", err));
        }

        ui.add_space(20.0);

        // Navigation
        ui.horizontal(|ui| {
            if ui.button("<< Back").clicked() {
                self.step = SetupStep::TelegramConfig;
                self.error = None;
            }
            ui.add_space(20.0);
            if ui
                .add(
                    egui::Button::new(egui::RichText::new("  Save Configuration  ").strong())
                        .min_size(egui::vec2(150.0, 35.0)),
                )
                .clicked()
            {
                if let Err(e) = self.save_config() {
                    self.error = Some(e.to_string());
                } else {
                    self.step = SetupStep::Complete;
                }
            }
        });
    }

    fn render_complete(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(40.0);

            ui.label(
                egui::RichText::new("[OK]")
                    .size(48.0)
                    .color(egui::Color32::GREEN),
            );
            ui.add_space(10.0);
            ui.heading("Setup Complete!");

            ui.add_space(20.0);

            ui.label(format!(
                "Your {} is ready to use.",
                crate::branding::PRODUCT_NAME
            ));

            ui.add_space(30.0);

            egui::Frame::none()
                .fill(egui::Color32::from_rgb(40, 45, 50))
                .rounding(10.0)
                .inner_margin(20.0)
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("Quick Start").strong());
                    ui.add_space(10.0);

                    ui.horizontal(|ui| {
                        ui.label("GUI Mode:");
                        ui.label(egui::RichText::new("agentark").monospace());
                    });

                    ui.horizontal(|ui| {
                        ui.label("Headless:");
                        ui.label(egui::RichText::new("agentark --headless").monospace());
                    });

                    ui.horizontal(|ui| {
                        ui.label("Re-run Setup:");
                        ui.label(egui::RichText::new("agentark --setup").monospace());
                    });

                    ui.add_space(10.0);
                    ui.label("HTTP API will be available at http://127.0.0.1:8990");
                });

            ui.add_space(30.0);

            if ui
                .add(
                    egui::Button::new(egui::RichText::new("  Launch Agent  ").size(16.0))
                        .min_size(egui::vec2(150.0, 40.0)),
                )
                .clicked()
            {
                // Return to indicate launch main app
                std::process::exit(0);
            }
        });
    }

    fn save_config(&mut self) -> anyhow::Result<()> {
        let model_id = self.ollama_model.trim().to_string();
        if self.llm_provider != LlmProviderChoice::Unset && model_id.is_empty() {
            return Err(anyhow::anyhow!("Model is required"));
        }
        if self.llm_provider == LlmProviderChoice::Ollama && self.ollama_url.trim().is_empty() {
            return Err(anyhow::anyhow!("Ollama URL is required"));
        }
        if self.llm_provider == LlmProviderChoice::OpenAICompatible
            && self.openai_base_url.trim().is_empty()
        {
            return Err(anyhow::anyhow!("OpenAI-compatible base URL is required"));
        }

        // Build LLM provider config
        let llm = match self.llm_provider {
            LlmProviderChoice::Unset => LlmProvider::default(),
            LlmProviderChoice::Anthropic => LlmProvider::Anthropic {
                api_key: self.api_key.clone(),
                model: model_id.clone(),
            },
            LlmProviderChoice::OpenAI => LlmProvider::OpenAI {
                api_key: self.api_key.clone(),
                model: model_id.clone(),
                base_url: None,
            },
            LlmProviderChoice::OpenAICompatible => LlmProvider::OpenAI {
                api_key: if self.api_key.is_empty() {
                    "not-needed".to_string()
                } else {
                    self.api_key.clone()
                },
                model: model_id.clone(),
                base_url: Some(self.openai_base_url.clone()),
            },
            LlmProviderChoice::Ollama => LlmProvider::Ollama {
                base_url: self.ollama_url.clone(),
                model: model_id.clone(),
            },
        };

        // Build telegram config
        let telegram = if self.telegram_token.is_empty() {
            None
        } else {
            let allowed_users: Vec<i64> = self
                .telegram_users
                .split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect();

            Some(crate::core::config::TelegramConfig {
                bot_token: self.telegram_token.clone(),
                allowed_users,
                dm_policy: "pairing".to_string(),
            })
        };

        // Update config
        let mut config = self.agent.config.clone();
        config.name = self.agent_name.clone();
        config.llm = llm;
        config.telegram = telegram;

        // Save main config
        config.save(&self.agent.config_dir, Some(&self.agent.data_dir))?;

        // Save search config
        let search_config = SearchConfig {
            serper: if self.enable_serper {
                Some(crate::actions::SearchBackend::Serper {
                    api_key: self.serper_key.clone(),
                })
            } else {
                None
            },
            brave: if self.enable_brave {
                Some(crate::actions::SearchBackend::Brave {
                    api_key: self.brave_key.clone(),
                })
            } else {
                None
            },
            exa: None,
            tavily: None,
            perplexity: None,
            firecrawl: None,
            searxng: None,
            playwright: None, // Auto-detected at runtime via bridge health check
            primary: None,
            fallback1: None,
            fallback2: None,
            provider_order: Vec::new(),
            health: crate::actions::search::SearchBackendHealthState::default(),
        };

        crate::runtime::save_persisted_search_config(
            &self.agent.config_dir,
            Some(&self.agent.data_dir),
            &search_config,
        )?;

        Ok(())
    }
}

impl eframe::App for SetupWizard {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Progress bar at top
        egui::TopBottomPanel::top("progress").show(ctx, |ui| {
            ui.add_space(5.0);
            let progress = match self.step {
                SetupStep::Welcome => 0.0,
                SetupStep::LlmConfig => 0.25,
                SetupStep::SearchConfig => 0.5,
                SetupStep::TelegramConfig => 0.75,
                SetupStep::Review => 0.9,
                SetupStep::Complete => 1.0,
            };
            ui.add(egui::ProgressBar::new(progress).show_percentage());
            ui.add_space(5.0);
        });

        egui::CentralPanel::default().show(ctx, |ui| match self.step {
            SetupStep::Welcome => self.render_welcome(ui),
            SetupStep::LlmConfig => self.render_llm_config(ui),
            SetupStep::SearchConfig => self.render_search_config(ui),
            SetupStep::TelegramConfig => self.render_telegram_config(ui),
            SetupStep::Review => self.render_review(ui),
            SetupStep::Complete => self.render_complete(ui),
        });
    }
}
