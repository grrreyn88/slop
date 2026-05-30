#![cfg_attr(windows, windows_subsystem = "windows")]

use serde::Serialize;
use std::{
    fs::{self, File},
    io::{self, Cursor},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tauri::{http, path::BaseDirectory, AppHandle, Emitter, Manager};
use zip::ZipArchive;

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine};

#[cfg(windows)]
use std::os::windows::process::CommandExt;
const EVENT_NAME: &str = "setup://status";
const UTILITY_EXE: &str = "injector.exe";
const UTILITY_DLL: &str = "neverlose.dll";
const PRIMO_EXE: &str = "primo.exe";
const PRIMO_DLL: &str = "primordial-csgo.dll";
const GAMESENSE_EXE: &str = "skeet2.exe";
const GAMESENSE_DLL: &str = "skeet.dll";
const LUA_ARCHIVE: &str = "lua_libs.zip";
const UTILITY_RUNTIME_DIR: &str = "payload";
const PRIMO_RUNTIME_DIR: &str = "primordial-payload";
const GAMESENSE_RUNTIME_DIR: &str = "gamesense-payload";
const TELEGRAM_URL: &str = "https://t.me/nlcsgofix";
const CSGO_APP_DIR: &str = "Counter-Strike Global Offensive";
const CSGO_EXE: &str = "csgo.exe";
const GAME_LIBRARY_PATH: &[&str] = &["nl_cloud", "scripts", "libraries"];
const FRONTEND_INDEX: &[u8] = include_bytes!("../../frontend/index.html");
const FRONTEND_STYLES: &[u8] = include_bytes!("../../frontend/styles.css");
const FRONTEND_APP: &[u8] = include_bytes!("../../frontend/app.js");
const FRONTEND_GS_ICON: &[u8] = include_bytes!("../../frontend/assets/images/gs.png");
const FRONTEND_PRIMO_ICON: &[u8] = include_bytes!("../../frontend/assets/images/primo.png");
const FRONTEND_NL_ICON: &[u8] = include_bytes!("../../frontend/assets/images/nl.png");
const UTILITY_EXE_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/injector.exe"
));
const UTILITY_DLL_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/neverlose.dll"
));
const PRIMO_EXE_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/primordial/primo.exe"
));
const PRIMO_DLL_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/primordial/primordial-csgo.dll"
));
const GAMESENSE_EXE_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/gamesense/skeet2.exe"
));
const GAMESENSE_DLL_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/gamesense/skeet.dll"
));
const LUA_ARCHIVE_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/lua_libs.zip"
));

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
    inner: Arc<AppStateInner>,
}

#[derive(Default)]
struct AppStateInner {
    setup_started: AtomicBool,
}

fn emit_status(app: &AppHandle, stage: &str, message: impl Into<String>, percent: u8, level: &str) {
    let payload = SetupEvent {
        stage: stage.to_string(),
        message: message.into(),
        percent: percent.min(100),
        level: level.to_string(),
    };
    let _ = app.emit(EVENT_NAME, payload);
}

fn frontend_response(request: http::Request<Vec<u8>>) -> http::Response<&'static [u8]> {
    let (content_type, body) = match request.uri().path().trim_start_matches('/') {
        "" | "index.html" => ("text/html; charset=utf-8", FRONTEND_INDEX),
        "styles.css" => ("text/css; charset=utf-8", FRONTEND_STYLES),
        "app.js" => ("application/javascript; charset=utf-8", FRONTEND_APP),
        "assets/images/gs.png" => ("image/png", FRONTEND_GS_ICON),
        "assets/images/primo.png" => ("image/png", FRONTEND_PRIMO_ICON),
        "assets/images/nl.png" => ("image/png", FRONTEND_NL_ICON),
        _ => ("text/html; charset=utf-8", FRONTEND_INDEX),
    };

    http::Response::builder()
        .header(http::header::CONTENT_TYPE, content_type)
        .body(body)
        .expect("failed to build frontend response")
}

#[cfg(windows)]
fn find_steam_path() -> Result<PathBuf, String> {
    let key = windows_registry::CURRENT_USER
        .open(r"Software\Valve\Steam")
        .map_err(|error| format!("Steam registry key was not found: {error}"))?;
    let steam_path = key
        .get_string("SteamPath")
        .map_err(|error| format!("SteamPath registry value was not found: {error}"))?;
    let steam_path = steam_path.trim();

    if steam_path.is_empty() {
        return Err("SteamPath registry value is empty.".to_string());
    }

    Ok(PathBuf::from(steam_path))
}

#[cfg(not(windows))]
fn find_steam_path() -> Result<PathBuf, String> {
    Err("Steam registry lookup is only available on Windows.".to_string())
}

fn parse_vdf_path_value(line: &str) -> Option<PathBuf> {
    let mut quoted_values = line
        .split('"')
        .skip(1)
        .step_by(2)
        .map(|value| value.replace(r"\\", r"\"));

    match (quoted_values.next(), quoted_values.next()) {
        (Some(key), Some(value)) if key.eq_ignore_ascii_case("path") => Some(PathBuf::from(value)),
        _ => None,
    }
}

fn steam_library_roots(steam_path: &Path) -> Vec<PathBuf> {
    let mut roots = vec![steam_path.to_path_buf()];
    let libraryfolders_path = steam_path.join("steamapps").join("libraryfolders.vdf");

    if let Ok(contents) = fs::read_to_string(libraryfolders_path) {
        for line in contents.lines() {
            if let Some(path) = parse_vdf_path_value(line.trim()) {
                roots.push(path);
            }
        }
    }

    let mut unique_roots = Vec::new();
    for root in roots {
        let key = root.to_string_lossy().to_lowercase();
        if !unique_roots
            .iter()
            .any(|existing: &PathBuf| existing.to_string_lossy().to_lowercase() == key)
        {
            unique_roots.push(root);
        }
    }

    unique_roots
}

fn find_csgo_dir() -> Result<PathBuf, String> {
    let steam_path = find_steam_path()?;

    for library_root in steam_library_roots(&steam_path) {
        let candidate = library_root
            .join("steamapps")
            .join("common")
            .join(CSGO_APP_DIR);
        if candidate.join(CSGO_EXE).is_file() {
            return Ok(candidate);
        }
    }

    Err("Could not find Counter-Strike Global Offensive/csgo.exe in Steam libraries.".to_string())
}

fn game_libraries_dir(csgo_dir: &Path) -> PathBuf {
    let mut libraries_dir = csgo_dir.to_path_buf();
    for segment in GAME_LIBRARY_PATH {
        libraries_dir.push(segment);
    }
    libraries_dir
}

fn bundled_payload_bytes(file_name: &str) -> Result<&'static [u8], String> {
    if file_name.eq_ignore_ascii_case(UTILITY_EXE) {
        return Ok(UTILITY_EXE_BYTES);
    }
    if file_name.eq_ignore_ascii_case(UTILITY_DLL) {
        return Ok(UTILITY_DLL_BYTES);
    }
    if file_name.eq_ignore_ascii_case(PRIMO_EXE) {
        return Ok(PRIMO_EXE_BYTES);
    }
    if file_name.eq_ignore_ascii_case(PRIMO_DLL) {
        return Ok(PRIMO_DLL_BYTES);
    }
    if file_name.eq_ignore_ascii_case(GAMESENSE_EXE) {
        return Ok(GAMESENSE_EXE_BYTES);
    }
    if file_name.eq_ignore_ascii_case(GAMESENSE_DLL) {
        return Ok(GAMESENSE_DLL_BYTES);
    }
    if file_name.eq_ignore_ascii_case(LUA_ARCHIVE) {
        return Ok(LUA_ARCHIVE_BYTES);
    }

    Err(format!("Unknown bundled payload file: {file_name}"))
}

#[cfg(windows)]
fn powershell_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(windows)]
fn extract_exe_icon_to_png(exe_path: &Path, output_path: &Path) -> Result<(), String> {
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create icon cache directory {}: {error}",
                parent.display()
            )
        })?;
    }

    let exe_path_arg = powershell_string(&exe_path.to_string_lossy());
    let output_path_arg = powershell_string(&output_path.to_string_lossy());
    let script = format!(
        r#"
Add-Type -AssemblyName System.Drawing
$icon = [System.Drawing.Icon]::ExtractAssociatedIcon({exe_path_arg})
if ($null -eq $icon) {{ exit 2 }}
$bitmap = $icon.ToBitmap()
$bitmap.Save({output_path_arg}, [System.Drawing.Imaging.ImageFormat]::Png)
$bitmap.Dispose()
$icon.Dispose()
"#
    );

    let mut command = Command::new("powershell.exe");
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        &script,
    ]);
    command.creation_flags(0x08000000);

    let output = command.output().map_err(|error| {
        format!(
            "Failed to start PowerShell for icon extraction from {}: {error:?}",
            exe_path.display()
        )
    })?;

    if output.status.success() && output_path.is_file() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if stderr.is_empty() { stdout } else { stderr };

    Err(format!(
        "Failed to extract CS:GO icon from {}. {}",
        exe_path.display(),
        detail
    ))
}

#[cfg(not(windows))]
fn extract_exe_icon_to_png(_exe_path: &Path, _output_path: &Path) -> Result<(), String> {
    Err("EXE icon extraction is only available on Windows.".to_string())
}

fn csgo_icon_data_url(app: &AppHandle) -> Result<String, String> {
    let csgo_exe_path = find_csgo_dir()?.join(CSGO_EXE);
    let icon_path = app
        .path()
        .resolve("csgo-icon.png", BaseDirectory::AppLocalData)
        .map_err(|error| format!("Failed to resolve CS:GO icon cache path: {error}"))?;

    extract_exe_icon_to_png(&csgo_exe_path, &icon_path)?;

    let icon_bytes = fs::read(&icon_path)
        .map_err(|error| format!("Failed to read CS:GO icon {}: {error}", icon_path.display()))?;

    Ok(format!(
        "data:image/png;base64,{}",
        BASE64_STANDARD.encode(icon_bytes)
    ))
}

fn extract_lua_libraries(libraries_dir: &Path) -> Result<(), String> {
    let archive_bytes = bundled_payload_bytes(LUA_ARCHIVE)?;
    fs::create_dir_all(libraries_dir).map_err(|error| {
        format!(
            "Failed to create libraries directory {}: {error}",
            libraries_dir.display()
        )
    })?;

    let mut archive = ZipArchive::new(Cursor::new(archive_bytes))
        .map_err(|error| format!("Failed to read embedded ZIP archive {LUA_ARCHIVE}: {error}"))?;

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("Failed to read ZIP entry #{index}: {error}"))?;
        let entry_name = entry.name().to_string();
        let enclosed_name = entry
            .enclosed_name()
            .ok_or_else(|| format!("ZIP entry has unsafe path: {entry_name}"))?;
        if enclosed_name.as_os_str().is_empty() {
            continue;
        }

        let output_path = libraries_dir.join(enclosed_name);
        if entry.is_dir() {
            fs::create_dir_all(&output_path).map_err(|error| {
                format!(
                    "Failed to create directory {}: {error}",
                    output_path.display()
                )
            })?;
            continue;
        }

        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!("Failed to create directory {}: {error}", parent.display())
            })?;
        }

        let mut output_file = File::create(&output_path)
            .map_err(|error| format!("Failed to create {}: {error}", output_path.display()))?;
        io::copy(&mut entry, &mut output_file)
            .map_err(|error| format!("Failed to extract {}: {error}", output_path.display()))?;
    }

    Ok(())
}

fn clean_windows_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let path_text = path.to_string_lossy();
        if let Some(stripped) = path_text.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{stripped}"));
        }
        if let Some(stripped) = path_text.strip_prefix(r"\\?\") {
            return PathBuf::from(stripped);
        }
    }

    path.to_path_buf()
}

fn canonical_runtime_path(path: &Path) -> PathBuf {
    fs::canonicalize(path)
        .map(|path| clean_windows_path(&path))
        .unwrap_or_else(|_| clean_windows_path(path))
}

#[cfg(windows)]
fn terminate_process_by_image(file_name: &str) {
    let mut cmd = Command::new("taskkill.exe");
    cmd.args(["/IM", file_name, "/F", "/T"]);
    cmd.creation_flags(0x08000000);

    let _ = cmd.output();
}

#[cfg(not(windows))]
fn terminate_process_by_image(_file_name: &str) {}

fn terminate_existing_utility_process() {
    terminate_process_by_image(UTILITY_EXE);
}

fn terminate_existing_primo_process() {
    terminate_process_by_image(PRIMO_EXE);
}

fn terminate_existing_gamesense_process() {
    terminate_process_by_image(GAMESENSE_EXE);
}

fn copy_payload_to_runtime(file_name: &str, runtime_dir: &Path) -> Result<PathBuf, String> {
    let payload_bytes = bundled_payload_bytes(file_name)?;
    let destination_path = runtime_dir.join(file_name);

    let write_result = fs::write(&destination_path, payload_bytes).or_else(|first_error| {
        if file_name.eq_ignore_ascii_case(UTILITY_EXE)
            || file_name.eq_ignore_ascii_case(PRIMO_EXE)
            || file_name.eq_ignore_ascii_case(GAMESENSE_EXE)
            || file_name.eq_ignore_ascii_case(GAMESENSE_DLL)
        {
            if file_name.eq_ignore_ascii_case(PRIMO_EXE) {
                terminate_existing_primo_process();
            } else if file_name.eq_ignore_ascii_case(GAMESENSE_EXE)
                || file_name.eq_ignore_ascii_case(GAMESENSE_DLL)
            {
                terminate_existing_gamesense_process();
            } else {
                terminate_existing_utility_process();
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
            fs::write(&destination_path, payload_bytes)
        } else {
            Err(first_error)
        }
    });

    write_result.map_err(|error| {
        format!(
            "Failed to write embedded {file_name} to {}: {error:?}",
            destination_path.display()
        )
    })?;

    if !destination_path.is_file() {
        return Err(format!(
            "Runtime file {} was not created.",
            destination_path.display()
        ));
    }

    Ok(destination_path)
}

fn prepare_utility_runtime(app: &AppHandle) -> Result<(PathBuf, PathBuf, PathBuf), String> {
    terminate_existing_utility_process();
    std::thread::sleep(std::time::Duration::from_millis(500));

    let runtime_dir = app
        .path()
        .resolve(UTILITY_RUNTIME_DIR, BaseDirectory::AppLocalData)
        .map_err(|error| format!("Failed to resolve runtime payload directory: {error}"))?;

    fs::create_dir_all(&runtime_dir).map_err(|error| {
        format!(
            "Failed to create runtime payload directory {}: {error}",
            runtime_dir.display()
        )
    })?;

    let exe_path = copy_payload_to_runtime(UTILITY_EXE, &runtime_dir)?;
    let dll_path = copy_payload_to_runtime(UTILITY_DLL, &runtime_dir)?;

    let runtime_dir = canonical_runtime_path(&runtime_dir);
    let exe_path = canonical_runtime_path(&exe_path);
    let dll_path = canonical_runtime_path(&dll_path);

    if exe_path.parent() != Some(runtime_dir.as_path())
        || dll_path.parent() != Some(runtime_dir.as_path())
    {
        return Err(format!(
            "Utility files must be in the same runtime directory. exe: {}, dll: {}, dir: {}",
            exe_path.display(),
            dll_path.display(),
            runtime_dir.display()
        ));
    }

    Ok((runtime_dir, exe_path, dll_path))
}

fn prepare_primo_runtime(app: &AppHandle) -> Result<(PathBuf, PathBuf, PathBuf), String> {
    terminate_existing_primo_process();
    std::thread::sleep(std::time::Duration::from_millis(500));

    let runtime_dir = app
        .path()
        .resolve(PRIMO_RUNTIME_DIR, BaseDirectory::AppLocalData)
        .map_err(|error| format!("Failed to resolve Primordial payload directory: {error}"))?;

    fs::create_dir_all(&runtime_dir).map_err(|error| {
        format!(
            "Failed to create Primordial payload directory {}: {error}",
            runtime_dir.display()
        )
    })?;

    let exe_path = copy_payload_to_runtime(PRIMO_EXE, &runtime_dir)?;
    let dll_path = copy_payload_to_runtime(PRIMO_DLL, &runtime_dir)?;

    let runtime_dir = canonical_runtime_path(&runtime_dir);
    let exe_path = canonical_runtime_path(&exe_path);
    let dll_path = canonical_runtime_path(&dll_path);

    if exe_path.parent() != Some(runtime_dir.as_path())
        || dll_path.parent() != Some(runtime_dir.as_path())
    {
        return Err(format!(
            "Primordial files must be in the same runtime directory. exe: {}, dll: {}, dir: {}",
            exe_path.display(),
            dll_path.display(),
            runtime_dir.display()
        ));
    }

    Ok((runtime_dir, exe_path, dll_path))
}

fn prepare_gamesense_runtime(app: &AppHandle) -> Result<(PathBuf, PathBuf, PathBuf), String> {
    terminate_existing_gamesense_process();
    std::thread::sleep(std::time::Duration::from_millis(500));

    let runtime_dir = app
        .path()
        .resolve(GAMESENSE_RUNTIME_DIR, BaseDirectory::AppLocalData)
        .map_err(|error| format!("Failed to resolve Gamesense payload directory: {error}"))?;

    fs::create_dir_all(&runtime_dir).map_err(|error| {
        format!(
            "Failed to create Gamesense payload directory {}: {error}",
            runtime_dir.display()
        )
    })?;

    let exe_path = copy_payload_to_runtime(GAMESENSE_EXE, &runtime_dir)?;
    let dll_path = copy_payload_to_runtime(GAMESENSE_DLL, &runtime_dir)?;

    let runtime_dir = canonical_runtime_path(&runtime_dir);
    let exe_path = canonical_runtime_path(&exe_path);
    let dll_path = canonical_runtime_path(&dll_path);

    if exe_path.parent() != Some(runtime_dir.as_path())
        || dll_path.parent() != Some(runtime_dir.as_path())
    {
        return Err(format!(
            "Gamesense files must be in the same runtime directory. exe: {}, dll: {}, dir: {}",
            exe_path.display(),
            dll_path.display(),
            runtime_dir.display()
        ));
    }

    Ok((runtime_dir, exe_path, dll_path))
}

fn launch_bundled_utility(app: &AppHandle) -> Result<(), String> {
    let (runtime_dir, exe_path, _dll_path) = prepare_utility_runtime(app)?;

    std::thread::sleep(std::time::Duration::from_millis(500));

    let mut cmd = Command::new(&exe_path);
    cmd.current_dir(&runtime_dir);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);

    match cmd.spawn() {
        Ok(_) => Ok(()),
        Err(error) => Err(format!("System launch error (code/text): {:?}", error)),
    }
}

fn launch_primordial(app: &AppHandle) -> Result<(), String> {
    let (runtime_dir, exe_path, _dll_path) = prepare_primo_runtime(app)?;

    std::thread::sleep(std::time::Duration::from_millis(500));

    #[cfg(windows)]
    {
        let exe_path_arg = powershell_string(&exe_path.to_string_lossy());
        let runtime_dir_arg = powershell_string(&runtime_dir.to_string_lossy());
        let script = format!(
            "Start-Process -FilePath {exe_path_arg} -WorkingDirectory {runtime_dir_arg} -Verb RunAs -WindowStyle Hidden"
        );

        let mut cmd = Command::new("powershell.exe");
        cmd.args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &script,
        ]);
        cmd.creation_flags(0x08000000);

        let output = cmd
            .output()
            .map_err(|error| format!("System elevated launch error (code/text): {:?}", error))?;

        if output.status.success() {
            return Ok(());
        }

        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if stderr.is_empty() { stdout } else { stderr };
        return Err(format!("Elevated Primordial launch failed: {detail}"));
    }

    #[cfg(not(windows))]
    {
        let mut cmd = Command::new(&exe_path);
        cmd.current_dir(&runtime_dir);

        match cmd.spawn() {
            Ok(_) => Ok(()),
            Err(error) => Err(format!("System launch error (code/text): {:?}", error)),
        }
    }
}

fn launch_gamesense(app: &AppHandle) -> Result<(), String> {
    let (runtime_dir, exe_path, _dll_path) = prepare_gamesense_runtime(app)?;

    std::thread::sleep(std::time::Duration::from_millis(500));

    let mut cmd = Command::new(&exe_path);
    cmd.current_dir(&runtime_dir);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);

    match cmd.spawn() {
        Ok(_) => Ok(()),
        Err(error) => Err(format!(
            "System Gamesense launch error (code/text): {:?}",
            error
        )),
    }
}

async fn run_loader_sequence(app: AppHandle) -> Result<(), String> {
    emit_status(&app, "steam", "Finding CS:GO installation...", 25, "info");
    let csgo_dir = find_csgo_dir()?;
    let libraries_dir = game_libraries_dir(&csgo_dir);

    emit_status(&app, "archive", "Installing Lua libraries...", 50, "info");
    extract_lua_libraries(&libraries_dir)?;

    emit_status(&app, "utility", "Starting bundled utility...", 75, "info");
    launch_bundled_utility(&app)?;

    emit_status(&app, "done", "Done! You can open CS:GO", 100, "success");
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_focus();
    }

    Ok(())
}

async fn run_gamesense_sequence(app: AppHandle) -> Result<(), String> {
    emit_status(&app, "gamesense", "Starting Gamesense...", 75, "info");
    launch_gamesense(&app)?;

    emit_status(&app, "done", "Done! Open CS:GO", 100, "success");
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_focus();
    }

    Ok(())
}

async fn run_primo_sequence(app: AppHandle) -> Result<(), String> {
    emit_status(&app, "primordial", "Starting Primordial...", 75, "info");
    launch_primordial(&app)?;

    emit_status(&app, "done", "Done! Open CS:GO", 100, "success");
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_focus();
    }

    Ok(())
}

#[tauri::command]
async fn start_gamesense_setup(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    if state.inner.setup_started.swap(true, Ordering::SeqCst) {
        return Ok(());
    }

    let state_inner = state.inner.clone();
    tauri::async_runtime::spawn(async move {
        match run_gamesense_sequence(app.clone()).await {
            Ok(()) => {
                state_inner.setup_started.store(false, Ordering::SeqCst);
            }
            Err(error) => {
                emit_status(&app, "error", error, 75, "error");
                state_inner.setup_started.store(false, Ordering::SeqCst);
            }
        }
    });

    Ok(())
}

#[tauri::command]
async fn start_setup(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    if state.inner.setup_started.swap(true, Ordering::SeqCst) {
        return Ok(());
    }

    let state_inner = state.inner.clone();
    tauri::async_runtime::spawn(async move {
        match run_loader_sequence(app.clone()).await {
            Ok(()) => {
                state_inner.setup_started.store(false, Ordering::SeqCst);
            }
            Err(error) => {
                emit_status(&app, "error", error, 75, "error");
                state_inner.setup_started.store(false, Ordering::SeqCst);
            }
        }
    });

    Ok(())
}

#[tauri::command]
async fn start_primo_setup(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    if state.inner.setup_started.swap(true, Ordering::SeqCst) {
        return Ok(());
    }

    let state_inner = state.inner.clone();
    tauri::async_runtime::spawn(async move {
        match run_primo_sequence(app.clone()).await {
            Ok(()) => {
                state_inner.setup_started.store(false, Ordering::SeqCst);
            }
            Err(error) => {
                emit_status(&app, "error", error, 75, "error");
                state_inner.setup_started.store(false, Ordering::SeqCst);
            }
        }
    });

    Ok(())
}

#[tauri::command]
fn exit_app(app: AppHandle) {
    terminate_existing_utility_process();
    terminate_existing_primo_process();
    terminate_existing_gamesense_process();
    app.exit(0);
}

#[tauri::command]
fn close_loader(app: AppHandle) {
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

#[tauri::command]
fn open_telegram_link(url: String) -> Result<(), String> {
    if url != TELEGRAM_URL {
        return Err("Unsupported external URL.".to_string());
    }

    Command::new("explorer.exe")
        .arg(TELEGRAM_URL)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Failed to open Telegram link: {:?}", error))
}

#[tauri::command]
fn get_csgo_icon(app: AppHandle) -> Option<String> {
    csgo_icon_data_url(&app).ok()
}

fn main() {
    tauri::Builder::default()
        .register_uri_scheme_protocol("nl", |_ctx, request| frontend_response(request))
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            start_setup,
            start_primo_setup,
            start_gamesense_setup,
            exit_app,
            close_loader,
            minimize_app,
            drag_app,
            open_telegram_link,
            get_csgo_icon
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
