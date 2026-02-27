//! Native GUI using egui

mod app;
mod views;

use anyhow::Result;

use crate::core::Agent;

pub use app::AgentArkApp;

/// Run the GUI application
pub async fn run(agent: Agent) -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([800.0, 600.0])
            .with_title("AgentArk"),
        ..Default::default()
    };

    let app = AgentArkApp::new(agent);

    eframe::run_native(
        "AgentArk",
        options,
        Box::new(|cc| {
            // Configure fonts and style
            let mut style = (*cc.egui_ctx.style()).clone();
            style.spacing.item_spacing = egui::vec2(8.0, 6.0);
            cc.egui_ctx.set_style(style);

            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow::anyhow!("GUI error: {}", e))?;

    Ok(())
}

/// Run the setup wizard
pub async fn run_setup_wizard(agent: Agent) -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([600.0, 500.0])
            .with_title("AgentArk Setup"),
        ..Default::default()
    };

    let app = views::SetupWizard::new(agent);

    eframe::run_native("AgentArk Setup", options, Box::new(|_cc| Ok(Box::new(app))))
        .map_err(|e| anyhow::anyhow!("Setup wizard error: {}", e))?;

    Ok(())
}
