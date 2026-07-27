#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

//! Bony Build native desktop client (egui + ACP stdio).

mod agent_bridge;
mod app;
mod bevy;
mod charts;
mod config_io;
mod events;
mod fonts;
mod git_workspace;
mod i18n;
mod markdown;
mod model;
mod npc_ai;
mod openmontage;
mod process;
mod task;
mod unity;
mod usage;

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use clap::Parser;
use eframe::egui;

use crate::agent_bridge::{BridgeConfig, resolve_grok_bin};
use crate::app::BonyBuildApp;
use crate::usage::load_plugin_prefs;

#[derive(Debug, Parser)]
#[command(name = "bony-build", about = "Native desktop client for Bony Build")]
struct Args {
    /// Working directory for the agent session (default: current directory).
    #[arg(long)]
    cwd: Option<PathBuf>,

    /// Path to the `grok` binary (default: search PATH).
    #[arg(long)]
    grok_bin: Option<PathBuf>,

    /// Require manual approval for every tool (overrides saved Full Control).
    #[arg(long = "ask-permissions")]
    ask_permissions: bool,
}

fn main() -> eframe::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let args = Args::parse();
    let cwd = args
        .cwd
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let grok_bin = resolve_grok_bin(args.grok_bin.as_deref());
    let prefs = load_plugin_prefs();
    let always_approve = !args.ask_permissions && prefs.auto_approve_tools;

    let config = BridgeConfig {
        grok_bin,
        cwd,
        always_approve: Arc::new(AtomicBool::new(always_approve)),
        resume_session_id: None,
    };

    let app_icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/bony-build.png"))
        .expect("embedded Bony Build icon must be a valid PNG");

    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 920.0])
            .with_min_inner_size([1100.0, 720.0])
            .with_title("Bony Build")
            .with_icon(app_icon)
            // Codex-style: single custom title bar (menus + window controls).
            .with_decorations(false),
        vsync: true,
        hardware_acceleration: eframe::HardwareAcceleration::Preferred,
        ..Default::default()
    };

    eframe::run_native(
        "Bony Build",
        native,
        Box::new(move |cc| Ok(Box::new(BonyBuildApp::new(cc, config)))),
    )
}
