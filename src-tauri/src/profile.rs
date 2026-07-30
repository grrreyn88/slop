use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine};
use serde::Serialize;
use serde_json::Value;
use std::{fs, path::Path};
use tauri::AppHandle;

use crate::{config::MAX_USERNAME_LENGTH, game};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NeverloseProfile {
    username: String,
    expiration_date: i64,
    avatar_data_url: String,
}

fn paths(app: &AppHandle) -> Result<(std::path::PathBuf, std::path::PathBuf), String> {
    let cloud_dir = game::find_csgo_dir(app)?.join("nl_cloud");
    Ok((
        cloud_dir.join("global_data.json"),
        cloud_dir.join("avatar.png"),
    ))
}

fn read_document(path: &Path) -> Result<Value, String> {
    let data = fs::read(path)
        .map_err(|error| format!("Не удалось прочитать {}: {error}", path.display()))?;
    serde_json::from_slice(&data)
        .map_err(|error| format!("Поврежден JSON {}: {error}", path.display()))
}

fn write_document(path: &Path, document: &Value) -> Result<(), String> {
    let data = serde_json::to_vec_pretty(document)
        .map_err(|error| format!("Не удалось собрать JSON профиля: {error}"))?;
    fs::write(path, data)
        .map_err(|error| format!("Не удалось записать {}: {error}", path.display()))
}

fn document_object(document: &mut Value) -> Result<&mut serde_json::Map<String, Value>, String> {
    document
        .as_object_mut()
        .ok_or_else(|| "global_data.json должен содержать объект JSON.".to_string())
}

pub fn load(app: &AppHandle) -> Result<Option<NeverloseProfile>, String> {
    let (data_path, avatar_path) = paths(app)?;
    if !data_path.is_file() || !avatar_path.is_file() {
        return Ok(None);
    }

    let document = read_document(&data_path)?;
    let username = document
        .get("username")
        .and_then(Value::as_str)
        .ok_or_else(|| "В global_data.json отсутствует строка username.".to_string())?
        .to_string();
    let expiration_date = document
        .get("expiration_date")
        .and_then(Value::as_i64)
        .ok_or_else(|| "В global_data.json отсутствует число expiration_date.".to_string())?;
    let avatar = fs::read(&avatar_path)
        .map_err(|error| format!("Не удалось прочитать {}: {error}", avatar_path.display()))?;

    Ok(Some(NeverloseProfile {
        username,
        expiration_date,
        avatar_data_url: format!("data:image/png;base64,{}", BASE64_STANDARD.encode(avatar)),
    }))
}

pub fn save_username(app: &AppHandle, username: String) -> Result<(), String> {
    if username.chars().count() > MAX_USERNAME_LENGTH {
        return Err(format!(
            "Ник не может быть длиннее {MAX_USERNAME_LENGTH} символов."
        ));
    }

    let (path, _) = paths(app)?;
    let mut document = read_document(&path)?;
    document_object(&mut document)?.insert("username".to_string(), Value::String(username));
    write_document(&path, &document)
}

pub fn save_expiration(app: &AppHandle, expiration_date: i64) -> Result<(), String> {
    let (path, _) = paths(app)?;
    let mut document = read_document(&path)?;
    document_object(&mut document)?.insert(
        "expiration_date".to_string(),
        Value::Number(expiration_date.into()),
    );
    write_document(&path, &document)
}

pub fn save_avatar(app: &AppHandle, avatar_data: String) -> Result<(), String> {
    let (_, avatar_path) = paths(app)?;
    let encoded = avatar_data
        .strip_prefix("data:image/png;base64,")
        .ok_or_else(|| "Аватар должен быть передан в формате PNG.".to_string())?;
    let avatar = BASE64_STANDARD
        .decode(encoded)
        .map_err(|error| format!("Не удалось декодировать аватар: {error}"))?;
    fs::write(&avatar_path, avatar)
        .map_err(|error| format!("Не удалось записать {}: {error}", avatar_path.display()))
}
