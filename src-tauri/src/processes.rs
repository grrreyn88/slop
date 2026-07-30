use std::{
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
use tauri::AppHandle;

use crate::config::{CREATE_NO_WINDOW, CSGO_EXE};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
pub fn is_running(file_name: &str) -> bool {
    let filter = format!("IMAGENAME eq {file_name}");
    let mut command = Command::new("tasklist.exe");
    command.args(["/FI", &filter, "/NH"]);
    command.creation_flags(CREATE_NO_WINDOW);

    let Ok(output) = command.output() else {
        return false;
    };
    if !output.status.success() {
        return false;
    }

    let expected_name = file_name.to_ascii_lowercase();
    String::from_utf8_lossy(&output.stdout).lines().any(|line| {
        line.trim_start()
            .to_ascii_lowercase()
            .starts_with(&expected_name)
    })
}

#[cfg(not(windows))]
pub fn is_running(_file_name: &str) -> bool {
    false
}

pub fn close_loader_when_csgo_starts(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            if is_running(CSGO_EXE) {
                app.exit(0);
                return;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
}

#[cfg(windows)]
pub fn terminate(file_name: &str) {
    let mut command = Command::new("taskkill.exe");
    command.args(["/IM", file_name, "/F", "/T"]);
    command.creation_flags(CREATE_NO_WINDOW);
    let _ = command.output();
}

#[cfg(not(windows))]
pub fn terminate(_file_name: &str) {}

pub fn canonical_runtime_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path)
        .map(|path| clean_windows_path(&path))
        .unwrap_or_else(|_| clean_windows_path(path))
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

pub fn launch_hidden(exe_path: &Path, working_dir: &Path) -> Result<(), String> {
    let mut command = Command::new(exe_path);
    command.current_dir(working_dir);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("System launch error (code/text): {error:?}"))
}
