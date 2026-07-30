use std::{
    env, fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tauri::{path::BaseDirectory, AppHandle, Manager};

use crate::config::{
    CSGO_APP_DIR, CSGO_APP_IDS, CSGO_EXE, CSGO_PATH_CACHE_FILE, CSGO_PATH_ENV_VARS,
    CSGO_PATH_HINT_FILE, GAME_LIBRARY_PATH, STRUCTURAL_SCAN_MAX_CANDIDATES,
    STRUCTURAL_SCAN_MAX_DEPTH, STRUCTURAL_SCAN_MAX_SECONDS, STRUCTURAL_SCAN_MAX_VISITED_DIRS,
};

fn path_key(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', r"\")
        .trim_end_matches('\\')
        .to_lowercase()
}

fn push_unique(paths: &mut Vec<PathBuf>, path: impl Into<PathBuf>) {
    let path = path.into();
    if path.as_os_str().is_empty() {
        return;
    }

    let key = path_key(&path);
    if !paths.iter().any(|existing| path_key(existing) == key) {
        paths.push(path);
    }
}

fn normalize_candidate(path: impl Into<PathBuf>) -> PathBuf {
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

fn push_candidate(paths: &mut Vec<PathBuf>, path: impl Into<PathBuf>) {
    push_unique(paths, normalize_candidate(path));
}

fn parse_vdf_key_value(line: &str) -> Option<(String, String)> {
    let mut values = line
        .split('"')
        .skip(1)
        .step_by(2)
        .map(|value| value.replace(r"\\", r"\"));

    Some((values.next()?, values.next()?))
}

fn looks_like_path(value: &str) -> bool {
    let value = value.trim();
    value.contains(r":\")
        || value.contains(":/")
        || value.starts_with(r"\\")
        || value.starts_with('/')
}

fn parse_library_path(line: &str) -> Option<PathBuf> {
    let (key, value) = parse_vdf_key_value(line)?;
    (key.eq_ignore_ascii_case("path") || (key.parse::<u32>().is_ok() && looks_like_path(&value)))
        .then(|| PathBuf::from(value))
}

fn parse_manifest_install_dir(manifest_path: &Path) -> Option<PathBuf> {
    let contents = fs::read_to_string(manifest_path).ok()?;
    contents.lines().find_map(|line| {
        let (key, value) = parse_vdf_key_value(line.trim())?;
        (key.eq_ignore_ascii_case("installdir") && !value.trim().is_empty())
            .then(|| PathBuf::from(value))
    })
}

fn read_path_file(path: &Path) -> Option<PathBuf> {
    let contents = fs::read_to_string(path).ok()?;
    let value = contents
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))?;
    Some(normalize_candidate(value.trim_matches('"')))
}

fn explicit_install_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    for variable in CSGO_PATH_ENV_VARS {
        if let Ok(value) = env::var(variable) {
            push_candidate(&mut paths, value);
        }
    }

    if let Ok(exe_path) = env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            if let Some(path) = read_path_file(&exe_dir.join(CSGO_PATH_HINT_FILE)) {
                push_candidate(&mut paths, path);
            }
        }
    }

    if let Ok(current_dir) = env::current_dir() {
        if let Some(path) = read_path_file(&current_dir.join(CSGO_PATH_HINT_FILE)) {
            push_candidate(&mut paths, path);
        }
    }

    paths
}

fn cached_install_path(app: &AppHandle) -> Option<PathBuf> {
    let cache_path = app
        .path()
        .resolve(CSGO_PATH_CACHE_FILE, BaseDirectory::AppLocalData)
        .ok()?;
    let cached_path = read_path_file(&cache_path)?;

    if cached_path.join(CSGO_EXE).is_file() {
        Some(cached_path)
    } else {
        let _ = fs::remove_file(cache_path);
        None
    }
}

fn remember_install_path(app: &AppHandle, csgo_dir: &Path) {
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

    for (root, key_path, value_name) in [
        (
            &windows_registry::CURRENT_USER,
            r"Software\Valve\Steam",
            "SteamPath",
        ),
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
    ] {
        if let Some(path) = registry_path(root, key_path, value_name) {
            push_unique(&mut paths, path);
        }
    }

    for variable in [
        "STEAM",
        "STEAM_HOME",
        "PROGRAMFILES(X86)",
        "PROGRAMFILES",
        "PROGRAMW6432",
    ] {
        if let Ok(value) = env::var(variable) {
            let path = PathBuf::from(value);
            push_unique(
                &mut paths,
                if variable.starts_with("PROGRAM") {
                    path.join("Steam")
                } else {
                    path
                },
            );
        }
    }

    for path in [
        PathBuf::from(r"C:\Steam"),
        PathBuf::from(r"C:\Program Files (x86)\Steam"),
        PathBuf::from(r"C:\Program Files\Steam"),
    ] {
        push_unique(&mut paths, path);
    }

    paths
}

#[cfg(not(windows))]
fn steam_install_paths() -> Vec<PathBuf> {
    Vec::new()
}

fn steam_library_roots(steam_path: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    push_unique(&mut roots, steam_path);

    let library_file = steam_path.join("steamapps").join("libraryfolders.vdf");
    if let Ok(contents) = fs::read_to_string(library_file) {
        for line in contents.lines() {
            if let Some(path) = parse_library_path(line.trim()) {
                push_unique(&mut roots, path);
            }
        }
    }
    roots
}

#[cfg(windows)]
fn registry_install_paths() -> Vec<PathBuf> {
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
                push_candidate(&mut paths, path);
            }
        }
    }

    paths
}

#[cfg(not(windows))]
fn registry_install_paths() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(windows)]
fn likely_steam_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for drive in b'C'..=b'Z' {
        let drive_root = format!("{}:\\", drive as char);
        if !Path::new(&drive_root).exists() {
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
            push_unique(&mut roots, PathBuf::from(&drive_root).join(relative));
        }
    }
    roots
}

#[cfg(not(windows))]
fn likely_steam_roots() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(windows)]
fn scan_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Ok(current_dir) = env::current_dir() {
        push_unique(&mut roots, current_dir);
    }
    if let Ok(exe_path) = env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            push_unique(&mut roots, exe_dir);
        }
    }
    if let Ok(user_profile) = env::var("USERPROFILE") {
        let user_profile = PathBuf::from(user_profile);
        for relative in ["Desktop", "Downloads", "Documents", "Games"] {
            push_unique(&mut roots, user_profile.join(relative));
        }
    }
    for variable in ["PROGRAMFILES(X86)", "PROGRAMFILES", "PROGRAMW6432"] {
        if let Ok(value) = env::var(variable) {
            push_unique(&mut roots, PathBuf::from(value));
        }
    }

    for drive in b'C'..=b'Z' {
        let drive_root = format!("{}:\\", drive as char);
        if !Path::new(&drive_root).exists() {
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
            push_unique(&mut roots, PathBuf::from(&drive_root).join(relative));
        }
    }
    roots
}

#[cfg(not(windows))]
fn scan_roots() -> Vec<PathBuf> {
    Vec::new()
}

fn should_skip_directory(path: &Path) -> bool {
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

fn scan_limit_reached(started_at: &Instant, visited_dirs: usize, candidates: usize) -> bool {
    started_at.elapsed() >= Duration::from_secs(STRUCTURAL_SCAN_MAX_SECONDS)
        || visited_dirs >= STRUCTURAL_SCAN_MAX_VISITED_DIRS
        || candidates >= STRUCTURAL_SCAN_MAX_CANDIDATES
}

fn collect_installations(
    root: &Path,
    candidates: &mut Vec<PathBuf>,
    started_at: &Instant,
    visited_dirs: &mut usize,
) {
    if !root.exists() {
        return;
    }

    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((current, depth)) = stack.pop() {
        if scan_limit_reached(started_at, *visited_dirs, candidates.len()) {
            return;
        }

        *visited_dirs += 1;
        if current.join(CSGO_EXE).is_file() {
            push_candidate(candidates, current);
            continue;
        }
        if depth >= STRUCTURAL_SCAN_MAX_DEPTH {
            continue;
        }

        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_dir() || should_skip_directory(&entry.path()) {
                continue;
            }
            stack.push((entry.path(), depth + 1));
        }
    }
}

fn scanned_install_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let started_at = Instant::now();
    let mut visited_dirs = 0usize;

    for root in scan_roots() {
        if scan_limit_reached(&started_at, visited_dirs, paths.len()) {
            break;
        }
        collect_installations(&root, &mut paths, &started_at, &mut visited_dirs);
    }
    paths
}

fn candidates_from_library(library_root: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let steamapps_dir = library_root.join("steamapps");
    let common_dir = steamapps_dir.join("common");

    push_candidate(&mut candidates, common_dir.join(CSGO_APP_DIR));
    for app_id in CSGO_APP_IDS {
        let manifest_path = steamapps_dir.join(format!("appmanifest_{app_id}.acf"));
        if let Some(install_dir) = parse_manifest_install_dir(&manifest_path) {
            push_candidate(&mut candidates, common_dir.join(install_dir));
        }
    }
    candidates
}

fn candidate_score(path: &Path) -> u8 {
    let path_text = path_key(path);
    let dir_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();

    if dir_name.eq_ignore_ascii_case(CSGO_APP_DIR) {
        0
    } else if path_text.contains(r"\counter-strike global offensive") {
        1
    } else if dir_name.eq_ignore_ascii_case("csgo legacy") {
        2
    } else {
        3
    }
}

fn accept_candidate(app: &AppHandle, path: &Path) -> Option<PathBuf> {
    path.join(CSGO_EXE).is_file().then(|| {
        remember_install_path(app, path);
        path.to_path_buf()
    })
}

pub fn find_csgo_dir(app: &AppHandle) -> Result<PathBuf, String> {
    if let Some(path) = cached_install_path(app) {
        return Ok(path);
    }

    let mut candidates = Vec::new();
    for path in explicit_install_paths() {
        if let Some(path) = accept_candidate(app, &path) {
            return Ok(path);
        }
        push_candidate(&mut candidates, path);
    }

    let mut library_roots = Vec::new();
    for steam_path in steam_install_paths() {
        for root in steam_library_roots(&steam_path) {
            push_unique(&mut library_roots, root);
        }
    }
    for root in likely_steam_roots() {
        push_unique(&mut library_roots, root);
    }
    for root in library_roots {
        for path in candidates_from_library(&root) {
            if let Some(path) = accept_candidate(app, &path) {
                return Ok(path);
            }
            push_candidate(&mut candidates, path);
        }
    }

    for path in scanned_install_paths()
        .into_iter()
        .chain(registry_install_paths())
    {
        push_candidate(&mut candidates, path);
    }

    candidates.sort_by_key(|path| candidate_score(path));
    let checked_count = candidates.len();
    for path in candidates {
        if let Some(path) = accept_candidate(app, &path) {
            return Ok(path);
        }
    }

    Err(format!(
        "Could not find Counter-Strike Global Offensive/csgo.exe. Checked {checked_count} likely locations."
    ))
}

pub fn libraries_dir(csgo_dir: &Path) -> PathBuf {
    GAME_LIBRARY_PATH
        .iter()
        .fold(csgo_dir.to_path_buf(), |path, segment| path.join(segment))
}

#[cfg(test)]
mod tests {
    use super::{normalize_candidate, parse_library_path, parse_vdf_key_value};
    use std::path::PathBuf;

    #[test]
    fn parses_vdf_pairs() {
        assert_eq!(
            parse_vdf_key_value(r#""path"  "D:\\SteamLibrary""#),
            Some(("path".to_string(), r"D:\SteamLibrary".to_string()))
        );
    }

    #[test]
    fn parses_numbered_legacy_library_paths() {
        assert_eq!(
            parse_library_path(r#""1"  "D:\\SteamLibrary""#),
            Some(PathBuf::from(r"D:\SteamLibrary"))
        );
    }

    #[test]
    fn normalizes_executable_path_to_directory() {
        assert_eq!(
            normalize_candidate(r"D:\Games\CSGO\csgo.exe"),
            PathBuf::from(r"D:\Games\CSGO")
        );
    }
}
