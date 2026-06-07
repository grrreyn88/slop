#![cfg_attr(windows, windows_subsystem = "windows")]

use serde::Serialize;
use std::{
    env,
    fs::{self, File},
    io::{self, Cursor, Read},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
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
const GAMESENSE_EXE: &str = "skeet-insecure.exe";
const GAMESENSE_DLL: &str = "skeet.dll";
const LUA_ARCHIVE: &str = "lua_libs.zip";
const UTILITY_RUNTIME_DIR: &str = "payload";
const PRIMO_RUNTIME_DIR: &str = "primordial-payload";
const GAMESENSE_RUNTIME_DIR: &str = "gamesense-payload";
const UTILITY_PAYLOAD_ARCHIVE: &str = "neverlose-payload.zip";
const PRIMO_PAYLOAD_ARCHIVE: &str = "primordial-payload.zip";
const GAMESENSE_PAYLOAD_ARCHIVE: &str = "gamesense-payload.zip";
const TELEGRAM_URL: &str = "https://t.me/nlcsgofix";
const CSGO_APP_DIR: &str = "Counter-Strike Global Offensive";
const CSGO_EXE: &str = "csgo.exe";
const CSGO_APP_IDS: &[&str] = &["730", "4465480"];
const CSGO_PATH_HINT_FILE: &str = "csgo_path.txt";
const CSGO_PATH_CACHE_FILE: &str = "csgo_path_cache.txt";
const CSGO_PATH_ENV_VARS: &[&str] = &["LOADER_CSGO_PATH", "CSGO_PATH", "CSGO_DIR"];
const STRUCTURAL_SCAN_MAX_DEPTH: usize = 6;
const STRUCTURAL_SCAN_MAX_SECONDS: u64 = 10;
const STRUCTURAL_SCAN_MAX_VISITED_DIRS: usize = 10_000;
const STRUCTURAL_SCAN_MAX_CANDIDATES: usize = 24;
const GAME_LIBRARY_PATH: &[&str] = &["nl_cloud", "scripts", "libraries"];
const FRONTEND_INDEX: &[u8] = include_bytes!("../../frontend/index.html");
const FRONTEND_STYLES: &[u8] = include_bytes!("../../frontend/styles.css");
const FRONTEND_APP: &[u8] = include_bytes!("../../frontend/app.js");
const FRONTEND_GS_ICON: &[u8] = include_bytes!("../../frontend/assets/images/gs.png");
const FRONTEND_PRIMO_ICON: &[u8] = include_bytes!("../../frontend/assets/images/primo.png");
const FRONTEND_NL_ICON: &[u8] = include_bytes!("../../frontend/assets/images/nl.png");
const UTILITY_PAYLOAD_ARCHIVE_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/neverlose-payload.zip"
));
const PRIMO_PAYLOAD_ARCHIVE_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/primordial-payload.zip"
));
const GAMESENSE_PAYLOAD_ARCHIVE_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/gamesense-payload.zip"
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

#[cfg(windows)]
fn is_process_running_by_image(file_name: &str) -> bool {
    let mut cmd = Command::new("tasklist.exe");
    let filter = format!("IMAGENAME eq {file_name}");
    cmd.args(["/FI", &filter, "/NH"]);
    cmd.creation_flags(0x08000000);

    let Ok(output) = cmd.output() else {
        return false;
    };
    if !output.status.success() {
        return false;
    }

    let file_name = file_name.to_ascii_lowercase();
    String::from_utf8_lossy(&output.stdout).lines().any(|line| {
        line.trim_start()
            .to_ascii_lowercase()
            .starts_with(&file_name)
    })
}

#[cfg(not(windows))]
fn is_process_running_by_image(_file_name: &str) -> bool {
    false
}

fn schedule_loader_exit_when_csgo_running(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            if is_process_running_by_image(CSGO_EXE) {
                app.exit(0);
                break;
            }

            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
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

fn path_dedupe_key(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', r"\")
        .trim_end_matches('\\')
        .to_lowercase()
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: impl Into<PathBuf>) {
    let path = path.into();
    if path.as_os_str().is_empty() {
        return;
    }

    let key = path_dedupe_key(&path);
    if !paths
        .iter()
        .any(|existing| path_dedupe_key(existing) == key)
    {
        paths.push(path);
    }
}

fn parse_vdf_key_value(line: &str) -> Option<(String, String)> {
    let mut quoted_values = line
        .split('"')
        .skip(1)
        .step_by(2)
        .map(|value| value.replace(r"\\", r"\"));

    match (quoted_values.next(), quoted_values.next()) {
        (Some(key), Some(value)) => Some((key, value)),
        _ => None,
    }
}

fn looks_like_filesystem_path(value: &str) -> bool {
    let value = value.trim();
    value.contains(r":\")
        || value.contains(":/")
        || value.starts_with(r"\\")
        || value.starts_with('/')
}

fn parse_vdf_path_value(line: &str) -> Option<PathBuf> {
    let (key, value) = parse_vdf_key_value(line)?;
    if key.eq_ignore_ascii_case("path")
        || (key.parse::<u32>().is_ok() && looks_like_filesystem_path(&value))
    {
        Some(PathBuf::from(value))
    } else {
        None
    }
}

fn parse_app_manifest_install_dir(manifest_path: &Path) -> Option<PathBuf> {
    let contents = fs::read_to_string(manifest_path).ok()?;
    for line in contents.lines() {
        if let Some((key, value)) = parse_vdf_key_value(line.trim()) {
            if key.eq_ignore_ascii_case("installdir") && !value.trim().is_empty() {
                return Some(PathBuf::from(value));
            }
        }
    }

    None
}

fn normalize_csgo_candidate_path(path: impl Into<PathBuf>) -> PathBuf {
    let path = path.into();
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(CSGO_EXE))
    {
        return path.parent().unwrap_or(path.as_path()).to_path_buf();
    }

    path
}

fn push_unique_csgo_candidate(paths: &mut Vec<PathBuf>, path: impl Into<PathBuf>) {
    push_unique_path(paths, normalize_csgo_candidate_path(path));
}

fn read_csgo_path_hint(path: &Path) -> Option<PathBuf> {
    let contents = fs::read_to_string(path).ok()?;
    let value = contents
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))?;

    Some(normalize_csgo_candidate_path(value.trim_matches('"')))
}

fn manual_csgo_install_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    for var_name in CSGO_PATH_ENV_VARS {
        if let Ok(value) = env::var(var_name) {
            push_unique_csgo_candidate(&mut paths, value);
        }
    }

    if let Ok(exe_path) = env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            if let Some(path) = read_csgo_path_hint(&exe_dir.join(CSGO_PATH_HINT_FILE)) {
                push_unique_csgo_candidate(&mut paths, path);
            }
        }
    }

    if let Ok(current_dir) = env::current_dir() {
        if let Some(path) = read_csgo_path_hint(&current_dir.join(CSGO_PATH_HINT_FILE)) {
            push_unique_csgo_candidate(&mut paths, path);
        }
    }

    paths
}

fn cached_csgo_install_path(app: &AppHandle) -> Option<PathBuf> {
    let cache_path = app
        .path()
        .resolve(CSGO_PATH_CACHE_FILE, BaseDirectory::AppLocalData)
        .ok()?;
    let cached_path = read_csgo_path_hint(&cache_path)?;

    if cached_path.join(CSGO_EXE).is_file() {
        Some(cached_path)
    } else {
        let _ = fs::remove_file(cache_path);
        None
    }
}

fn remember_csgo_install_path(app: &AppHandle, csgo_dir: &Path) {
    let Ok(cache_path) = app
        .path()
        .resolve(CSGO_PATH_CACHE_FILE, BaseDirectory::AppLocalData)
    else {
        return;
    };

    if let Some(parent) = cache_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let _ = fs::write(cache_path, csgo_dir.to_string_lossy().as_bytes());
}

fn steam_library_roots(steam_path: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    push_unique_path(&mut roots, steam_path);
    let libraryfolders_path = steam_path.join("steamapps").join("libraryfolders.vdf");

    if let Ok(contents) = fs::read_to_string(libraryfolders_path) {
        for line in contents.lines() {
            if let Some(path) = parse_vdf_path_value(line.trim()) {
                push_unique_path(&mut roots, path);
            }
        }
    }

    roots
}

#[cfg(windows)]
fn registry_string(
    root: &windows_registry::Key,
    key_path: &str,
    value_name: &str,
) -> Option<String> {
    root.open(key_path)
        .ok()?
        .get_string(value_name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(windows)]
fn registry_path(
    root: &windows_registry::Key,
    key_path: &str,
    value_name: &str,
) -> Option<PathBuf> {
    registry_string(root, key_path, value_name).map(PathBuf::from)
}

#[cfg(windows)]
fn steam_install_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Ok(path) = find_steam_path() {
        push_unique_path(&mut paths, path);
    }

    for (root, key_path, value_name) in [
        (
            &windows_registry::LOCAL_MACHINE,
            r"SOFTWARE\WOW6432Node\Valve\Steam",
            "InstallPath",
        ),
        (
            &windows_registry::LOCAL_MACHINE,
            r"SOFTWARE\Valve\Steam",
            "InstallPath",
        ),
        (
            &windows_registry::CURRENT_USER,
            r"Software\Valve\Steam",
            "SteamPath",
        ),
    ] {
        if let Some(path) = registry_path(root, key_path, value_name) {
            push_unique_path(&mut paths, path);
        }
    }

    for var_name in [
        "STEAM",
        "STEAM_HOME",
        "PROGRAMFILES(X86)",
        "PROGRAMFILES",
        "PROGRAMW6432",
    ] {
        if let Ok(value) = env::var(var_name) {
            let path = PathBuf::from(value);
            if var_name.starts_with("PROGRAM") {
                push_unique_path(&mut paths, path.join("Steam"));
            } else {
                push_unique_path(&mut paths, path);
            }
        }
    }

    for path in [
        PathBuf::from(r"C:\Steam"),
        PathBuf::from(r"C:\Program Files (x86)\Steam"),
        PathBuf::from(r"C:\Program Files\Steam"),
    ] {
        push_unique_path(&mut paths, path);
    }

    paths
}

#[cfg(not(windows))]
fn steam_install_paths() -> Vec<PathBuf> {
    find_steam_path().ok().into_iter().collect()
}

#[cfg(windows)]
fn registry_csgo_install_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    for app_id in CSGO_APP_IDS {
        for (root, key_path) in [
            (
                &windows_registry::LOCAL_MACHINE,
                format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Steam App {app_id}"),
            ),
            (
                &windows_registry::LOCAL_MACHINE,
                format!(
                    r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Steam App {app_id}"
                ),
            ),
            (
                &windows_registry::CURRENT_USER,
                format!(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Steam App {app_id}"),
            ),
        ] {
            if let Some(path) = registry_path(root, &key_path, "InstallLocation") {
                push_unique_path(&mut paths, path);
            }
        }
    }

    paths
}

#[cfg(not(windows))]
fn registry_csgo_install_paths() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(windows)]
fn likely_steam_library_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    for drive in b'C'..=b'Z' {
        let root = format!("{}:\\", drive as char);
        if !Path::new(&root).exists() {
            continue;
        }

        for relative in [
            "SteamLibrary",
            "Steam",
            r"Games\SteamLibrary",
            r"Games\Steam",
            r"Program Files (x86)\Steam",
            r"Program Files\Steam",
        ] {
            push_unique_path(&mut roots, PathBuf::from(&root).join(relative));
        }
    }

    roots
}

#[cfg(not(windows))]
fn likely_steam_library_roots() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(windows)]
fn likely_game_search_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Ok(current_dir) = env::current_dir() {
        push_unique_path(&mut roots, current_dir);
    }

    if let Ok(exe_path) = env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            push_unique_path(&mut roots, exe_dir);
        }
    }

    if let Ok(user_profile) = env::var("USERPROFILE") {
        let user_profile = PathBuf::from(user_profile);
        for relative in ["Desktop", "Downloads", "Documents", "Games"] {
            push_unique_path(&mut roots, user_profile.join(relative));
        }
    }

    for var_name in ["PROGRAMFILES(X86)", "PROGRAMFILES", "PROGRAMW6432"] {
        if let Ok(value) = env::var(var_name) {
            push_unique_path(&mut roots, PathBuf::from(value));
        }
    }

    for drive in b'C'..=b'Z' {
        let root = format!("{}:\\", drive as char);
        if !Path::new(&root).exists() {
            continue;
        }

        for relative in [
            "",
            "Games",
            "Game",
            "Downloads",
            "CSGO",
            "Counter-Strike",
            "Counter-Strike Global Offensive",
            "csgo legacy",
            "SteamLibrary",
            "Steam",
            r"Games\CSGO",
            r"Games\Counter-Strike Global Offensive",
            r"Program Files (x86)\Steam\steamapps\common",
            r"Program Files\Steam\steamapps\common",
        ] {
            push_unique_path(&mut roots, PathBuf::from(&root).join(relative));
        }
    }

    roots
}

#[cfg(not(windows))]
fn likely_game_search_roots() -> Vec<PathBuf> {
    Vec::new()
}

fn should_skip_scan_dir(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };

    matches!(
        name.to_ascii_lowercase().as_str(),
        "$recycle.bin"
            | "system volume information"
            | "windows"
            | "winnt"
            | "windows.old"
            | "programdata"
            | "appdata"
            | "node_modules"
            | "target"
            | ".git"
            | ".svn"
            | ".hg"
    )
}

fn structural_scan_exhausted(started_at: &Instant, visited_dirs: usize, candidates: usize) -> bool {
    started_at.elapsed() >= Duration::from_secs(STRUCTURAL_SCAN_MAX_SECONDS)
        || visited_dirs >= STRUCTURAL_SCAN_MAX_VISITED_DIRS
        || candidates >= STRUCTURAL_SCAN_MAX_CANDIDATES
}

fn collect_csgo_dirs_under(
    root: &Path,
    max_depth: usize,
    candidates: &mut Vec<PathBuf>,
    started_at: &Instant,
    visited_dirs: &mut usize,
) {
    if !root.exists() {
        return;
    }

    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((current, depth)) = stack.pop() {
        if structural_scan_exhausted(started_at, *visited_dirs, candidates.len()) {
            return;
        }

        *visited_dirs += 1;

        if current.join(CSGO_EXE).is_file() {
            push_unique_csgo_candidate(candidates, current);
            continue;
        }

        if depth >= max_depth {
            continue;
        }

        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };

        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_dir() {
                continue;
            }

            let entry_path = entry.path();
            if should_skip_scan_dir(&entry_path) {
                continue;
            }

            stack.push((entry_path, depth + 1));
        }
    }
}

fn structural_csgo_install_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let started_at = Instant::now();
    let mut visited_dirs = 0usize;

    for root in likely_game_search_roots() {
        if structural_scan_exhausted(&started_at, visited_dirs, paths.len()) {
            break;
        }

        collect_csgo_dirs_under(
            &root,
            STRUCTURAL_SCAN_MAX_DEPTH,
            &mut paths,
            &started_at,
            &mut visited_dirs,
        );
    }

    paths
}

fn candidate_csgo_dirs_from_library_root(library_root: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let steamapps_dir = library_root.join("steamapps");
    let common_dir = steamapps_dir.join("common");

    push_unique_csgo_candidate(&mut candidates, common_dir.join(CSGO_APP_DIR));

    for app_id in ["730", "4465480"] {
        let manifest_path = steamapps_dir.join(format!("appmanifest_{app_id}.acf"));
        if let Some(install_dir) = parse_app_manifest_install_dir(&manifest_path) {
            push_unique_csgo_candidate(&mut candidates, common_dir.join(install_dir));
        }
    }

    candidates
}

fn csgo_candidate_score(path: &Path) -> u8 {
    let path_text = path_dedupe_key(path);
    let dir_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();

    if dir_name.eq_ignore_ascii_case(CSGO_APP_DIR) {
        return 0;
    }

    if path_text.contains(r"\counter-strike global offensive") {
        return 1;
    }

    if dir_name.eq_ignore_ascii_case("csgo legacy") {
        return 2;
    }

    3
}

fn find_csgo_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let mut candidates = Vec::new();
    let mut library_roots = Vec::new();

    if let Some(cached_path) = cached_csgo_install_path(app) {
        push_unique_csgo_candidate(&mut candidates, cached_path);
    }

    for candidate in manual_csgo_install_paths() {
        if candidate.join(CSGO_EXE).is_file() {
            remember_csgo_install_path(app, &candidate);
            return Ok(candidate);
        }
        push_unique_csgo_candidate(&mut candidates, candidate);
    }

    for steam_path in steam_install_paths() {
        for library_root in steam_library_roots(&steam_path) {
            push_unique_path(&mut library_roots, library_root);
        }
    }

    for library_root in likely_steam_library_roots() {
        push_unique_path(&mut library_roots, library_root);
    }

    for library_root in library_roots {
        for candidate in candidate_csgo_dirs_from_library_root(&library_root) {
            push_unique_path(&mut candidates, candidate);
        }
    }

    for path in structural_csgo_install_paths() {
        push_unique_csgo_candidate(&mut candidates, path);
    }

    for path in registry_csgo_install_paths() {
        push_unique_csgo_candidate(&mut candidates, path);
    }

    candidates.sort_by_key(|path| csgo_candidate_score(path));

    let checked_count = candidates.len();
    for candidate in candidates {
        if candidate.join(CSGO_EXE).is_file() {
            remember_csgo_install_path(app, &candidate);
            return Ok(candidate);
        }
    }

    Err(format!(
        "Could not find Counter-Strike Global Offensive/csgo.exe. Checked {checked_count} likely Steam locations."
    ))
}

fn game_libraries_dir(csgo_dir: &Path) -> PathBuf {
    let mut libraries_dir = csgo_dir.to_path_buf();
    for segment in GAME_LIBRARY_PATH {
        libraries_dir.push(segment);
    }
    libraries_dir
}

fn payload_archive_for_file(file_name: &str) -> Result<(&'static [u8], &'static str), String> {
    if file_name.eq_ignore_ascii_case(UTILITY_EXE) || file_name.eq_ignore_ascii_case(UTILITY_DLL) {
        return Ok((UTILITY_PAYLOAD_ARCHIVE_BYTES, UTILITY_PAYLOAD_ARCHIVE));
    }
    if file_name.eq_ignore_ascii_case(PRIMO_EXE) || file_name.eq_ignore_ascii_case(PRIMO_DLL) {
        return Ok((PRIMO_PAYLOAD_ARCHIVE_BYTES, PRIMO_PAYLOAD_ARCHIVE));
    }
    if file_name.eq_ignore_ascii_case(GAMESENSE_EXE)
        || file_name.eq_ignore_ascii_case(GAMESENSE_DLL)
    {
        return Ok((GAMESENSE_PAYLOAD_ARCHIVE_BYTES, GAMESENSE_PAYLOAD_ARCHIVE));
    }

    Err(format!("Unknown bundled payload file: {file_name}"))
}

fn bundled_payload_bytes(file_name: &str) -> Result<Vec<u8>, String> {
    let (archive_bytes, archive_name) = payload_archive_for_file(file_name)?;
    let mut archive = ZipArchive::new(Cursor::new(archive_bytes)).map_err(|error| {
        format!("Failed to read embedded payload archive {archive_name}: {error}")
    })?;

    let entry_index = (0..archive.len())
        .find(|index| {
            archive
                .by_index(*index)
                .ok()
                .and_then(|entry| {
                    entry
                        .enclosed_name()
                        .and_then(|path| path.file_name().map(|name| name.to_os_string()))
                })
                .and_then(|name| {
                    name.to_str()
                        .map(|name| name.eq_ignore_ascii_case(file_name))
                })
                .unwrap_or(false)
        })
        .ok_or_else(|| format!("Embedded payload {file_name} was not found in {archive_name}"))?;

    let mut entry = archive
        .by_index(entry_index)
        .map_err(|error| format!("Failed to read {file_name} from {archive_name}: {error}"))?;
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut bytes).map_err(|error| {
        format!("Failed to decompress {file_name} from {archive_name}: {error}")
    })?;

    Ok(bytes)
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
    let csgo_exe_path = find_csgo_dir(app)?.join(CSGO_EXE);
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
    fs::create_dir_all(libraries_dir).map_err(|error| {
        format!(
            "Failed to create libraries directory {}: {error}",
            libraries_dir.display()
        )
    })?;

    let mut archive = ZipArchive::new(Cursor::new(LUA_ARCHIVE_BYTES))
        .map_err(|error| format!("Failed to read embedded ZIP archive {LUA_ARCHIVE}: {error}"))?;

    let mut extracted_files = 0usize;
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
        extracted_files += 1;
    }

    if extracted_files == 0 {
        return Err(format!(
            "Embedded ZIP archive {LUA_ARCHIVE} did not contain any files to extract."
        ));
    }

    let installed_files = count_files_recursively(libraries_dir).map_err(|error| {
        format!(
            "Failed to verify extracted Lua libraries in {}: {error}",
            libraries_dir.display()
        )
    })?;

    if installed_files == 0 {
        return Err(format!(
            "Lua libraries were not installed into {}. Extracted {extracted_files} files from archive, but target directory is empty.",
            libraries_dir.display()
        ));
    }

    Ok(())
}

fn count_files_recursively(path: &Path) -> io::Result<usize> {
    let mut count = 0usize;
    let mut stack = vec![path.to_path_buf()];

    while let Some(current) = stack.pop() {
        for entry in fs::read_dir(current)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                stack.push(entry.path());
            } else if file_type.is_file() {
                count += 1;
            }
        }
    }

    Ok(count)
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

    let write_result = fs::write(&destination_path, &payload_bytes).or_else(|first_error| {
        if file_name.eq_ignore_ascii_case(UTILITY_EXE)
            || file_name.eq_ignore_ascii_case(UTILITY_DLL)
            || file_name.eq_ignore_ascii_case(PRIMO_EXE)
            || file_name.eq_ignore_ascii_case(PRIMO_DLL)
            || file_name.eq_ignore_ascii_case(GAMESENSE_EXE)
            || file_name.eq_ignore_ascii_case(GAMESENSE_DLL)
        {
            if file_name.eq_ignore_ascii_case(PRIMO_EXE)
                || file_name.eq_ignore_ascii_case(PRIMO_DLL)
            {
                terminate_existing_primo_process();
            } else if file_name.eq_ignore_ascii_case(GAMESENSE_EXE)
                || file_name.eq_ignore_ascii_case(GAMESENSE_DLL)
            {
                terminate_existing_gamesense_process();
            } else {
                terminate_existing_utility_process();
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
            fs::write(&destination_path, &payload_bytes)
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

    std::thread::sleep(Duration::from_millis(500));

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

    std::thread::sleep(Duration::from_millis(500));

    let mut cmd = Command::new(&exe_path);
    cmd.current_dir(&runtime_dir);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);

    match cmd.spawn() {
        Ok(_) => Ok(()),
        Err(error) => Err(format!("System launch error (code/text): {:?}", error)),
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
    let csgo_dir = find_csgo_dir(&app)?;
    let libraries_dir = game_libraries_dir(&csgo_dir);

    emit_status(&app, "archive", "Installing Lua libraries...", 50, "info");
    extract_lua_libraries(&libraries_dir)?;

    emit_status(&app, "utility", "Starting bundled utility...", 75, "info");
    launch_bundled_utility(&app)?;

    emit_status(&app, "done", "Done! You can open CS:GO", 100, "success");
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_focus();
    }
    schedule_loader_exit_when_csgo_running(&app);

    Ok(())
}

async fn run_gamesense_sequence(app: AppHandle) -> Result<(), String> {
    emit_status(&app, "gamesense", "Starting Gamesense...", 75, "info");
    launch_gamesense(&app)?;

    emit_status(&app, "done", "Done! Open CS:GO", 100, "success");
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_focus();
    }
    schedule_loader_exit_when_csgo_running(&app);

    Ok(())
}

async fn run_primo_sequence(app: AppHandle) -> Result<(), String> {
    emit_status(&app, "primordial", "Starting Primordial...", 75, "info");
    launch_primordial(&app)?;

    emit_status(&app, "done", "Done! Open CS:GO", 100, "success");
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_focus();
    }
    schedule_loader_exit_when_csgo_running(&app);

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
