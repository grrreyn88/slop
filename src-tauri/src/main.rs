#![cfg_attr(windows, windows_subsystem = "windows")]

mod config;
mod game;
mod payload;
mod processes;
mod profile;
mod web;

use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::{AppHandle, Emitter, Manager};

use crate::{config::EVENT_NAME, payload::Product, profile::NeverloseProfile};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SetupEvent {
    stage: String,
    message: String,
    percent: u8,
    level: String,
}

#[derive(Clone, Default)]
struct AppState {
    setup_started: Arc<AtomicBool>,
}

fn emit_status(app: &AppHandle, stage: &str, message: impl Into<String>, percent: u8, level: &str) {
    let event = SetupEvent {
        stage: stage.to_string(),
        message: message.into(),
        percent: percent.min(100),
        level: level.to_string(),
    };
    let _ = app.emit(EVENT_NAME, event);
}

async fn run_product_sequence(app: AppHandle, product: Product) -> Result<(), String> {
    if matches!(product, Product::Neverlose) {
        emit_status(&app, "steam", "Finding CS:GO installation...", 25, "info");
        let csgo_dir = game::find_csgo_dir(&app)?;

        emit_status(&app, "archive", "Installing Lua libraries...", 50, "info");
        payload::extract_lua_libraries(&game::libraries_dir(&csgo_dir))?;
    }

    let (stage, message) = match product {
        Product::Neverlose => ("utility", "Starting bundled utility..."),
        Product::Primordial => ("primordial", "Starting Primordial..."),
        Product::Gamesense => ("gamesense", "Starting Gamesense..."),
    };
    emit_status(&app, stage, message, 75, "info");
    payload::launch(&app, product)?;

    emit_status(&app, "done", "Done! Open CS:GO", 100, "success");
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_focus();
    }
    processes::close_loader_when_csgo_starts(&app);
    Ok(())
}

fn begin_product_setup(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    product: Product,
) -> Result<(), String> {
    if state.setup_started.swap(true, Ordering::SeqCst) {
        return Ok(());
    }

    let setup_started = state.setup_started.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = run_product_sequence(app.clone(), product).await {
            emit_status(&app, "error", error, 75, "error");
        }
        setup_started.store(false, Ordering::SeqCst);
    });
    Ok(())
}

#[tauri::command]
async fn start_setup(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    begin_product_setup(app, state, Product::Neverlose)
}

#[tauri::command]
async fn start_primo_setup(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    begin_product_setup(app, state, Product::Primordial)
}

#[tauri::command]
async fn start_gamesense_setup(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    begin_product_setup(app, state, Product::Gamesense)
}

#[tauri::command]
fn get_neverlose_profile(app: AppHandle) -> Result<Option<NeverloseProfile>, String> {
    profile::load(&app)
}

#[tauri::command]
fn save_neverlose_username(app: AppHandle, username: String) -> Result<(), String> {
    profile::save_username(&app, username)
}

#[tauri::command]
fn save_neverlose_expiration(app: AppHandle, expiration_date: i64) -> Result<(), String> {
    profile::save_expiration(&app, expiration_date)
}

#[tauri::command]
fn save_neverlose_avatar(app: AppHandle, avatar_data: String) -> Result<(), String> {
    profile::save_avatar(&app, avatar_data)
}

#[tauri::command]
fn exit_app(app: AppHandle) {
    payload::terminate_all();
    app.exit(0);
}

#[tauri::command]
fn minimize_app(app: AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.minimize();
    }
}

#[tauri::command]
fn drag_app(app: AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.start_dragging();
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .register_uri_scheme_protocol("nl", |_context, request| web::response(request))
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            start_setup,
            start_primo_setup,
            start_gamesense_setup,
            exit_app,
            minimize_app,
            drag_app,
            get_neverlose_profile,
            save_neverlose_username,
            save_neverlose_expiration,
            save_neverlose_avatar,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
