use std::{
    fs::{self, File},
    io::{self, Cursor, Read},
    path::{Path, PathBuf},
    thread,
    time::Duration,
};
use tauri::{path::BaseDirectory, AppHandle, Manager};
use zip::ZipArchive;

use crate::{
    config::{
        GAMESENSE_DLL, GAMESENSE_EXE, GAMESENSE_PAYLOAD_ARCHIVE, GAMESENSE_RUNTIME_DIR,
        LUA_ARCHIVE, PRIMO_DLL, PRIMO_EXE, PRIMO_PAYLOAD_ARCHIVE, PRIMO_RUNTIME_DIR,
        PROCESS_SETTLE_DELAY_MS, UTILITY_DLL, UTILITY_EXE, UTILITY_PAYLOAD_ARCHIVE,
        UTILITY_RUNTIME_DIR,
    },
    processes,
};

const NEVERLOSE_ARCHIVE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/neverlose-payload.zip"
));
const PRIMORDIAL_ARCHIVE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/primordial-payload.zip"
));
const GAMESENSE_ARCHIVE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/gamesense-payload.zip"
));
const LUA_LIBRARIES_ARCHIVE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/lua_libs.zip"
));

#[derive(Clone, Copy, Debug)]
pub enum Product {
    Neverlose,
    Primordial,
    Gamesense,
}

struct ProductFiles {
    executable: &'static str,
    library: &'static str,
    runtime_dir: &'static str,
    archive_name: &'static str,
    archive: &'static [u8],
}

impl Product {
    fn files(self) -> ProductFiles {
        match self {
            Self::Neverlose => ProductFiles {
                executable: UTILITY_EXE,
                library: UTILITY_DLL,
                runtime_dir: UTILITY_RUNTIME_DIR,
                archive_name: UTILITY_PAYLOAD_ARCHIVE,
                archive: NEVERLOSE_ARCHIVE,
            },
            Self::Primordial => ProductFiles {
                executable: PRIMO_EXE,
                library: PRIMO_DLL,
                runtime_dir: PRIMO_RUNTIME_DIR,
                archive_name: PRIMO_PAYLOAD_ARCHIVE,
                archive: PRIMORDIAL_ARCHIVE,
            },
            Self::Gamesense => ProductFiles {
                executable: GAMESENSE_EXE,
                library: GAMESENSE_DLL,
                runtime_dir: GAMESENSE_RUNTIME_DIR,
                archive_name: GAMESENSE_PAYLOAD_ARCHIVE,
                archive: GAMESENSE_ARCHIVE,
            },
        }
    }
}

fn settle_process_handles() {
    thread::sleep(Duration::from_millis(PROCESS_SETTLE_DELAY_MS));
}

pub fn terminate(product: Product) {
    processes::terminate(product.files().executable);
}

pub fn terminate_all() {
    for product in [Product::Neverlose, Product::Primordial, Product::Gamesense] {
        terminate(product);
    }
}

fn read_archive_file(files: &ProductFiles, file_name: &str) -> Result<Vec<u8>, String> {
    let mut archive = ZipArchive::new(Cursor::new(files.archive)).map_err(|error| {
        format!(
            "Failed to read embedded payload archive {}: {error}",
            files.archive_name
        )
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
        .ok_or_else(|| {
            format!(
                "Embedded payload {file_name} was not found in {}",
                files.archive_name
            )
        })?;

    let mut entry = archive.by_index(entry_index).map_err(|error| {
        format!(
            "Failed to read {file_name} from {}: {error}",
            files.archive_name
        )
    })?;
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut bytes).map_err(|error| {
        format!(
            "Failed to decompress {file_name} from {}: {error}",
            files.archive_name
        )
    })?;
    Ok(bytes)
}

fn write_runtime_file(
    product: Product,
    files: &ProductFiles,
    file_name: &str,
    runtime_dir: &Path,
) -> Result<PathBuf, String> {
    let bytes = read_archive_file(files, file_name)?;
    let destination = runtime_dir.join(file_name);

    let write_result = fs::write(&destination, &bytes).or_else(|_| {
        terminate(product);
        settle_process_handles();
        fs::write(&destination, &bytes)
    });

    write_result.map_err(|error| match error.raw_os_error() {
        Some(32) => {
            format!("Не удалось обновить {file_name}: файл занят запущенным процессом (код 32).")
        }
        Some(code) => format!(
            "Не удалось записать {file_name} в {}: {error} (код {code}).",
            destination.display()
        ),
        None => format!(
            "Не удалось записать {file_name} в {}: {error}.",
            destination.display()
        ),
    })?;

    destination
        .is_file()
        .then_some(destination.clone())
        .ok_or_else(|| format!("Runtime file {} was not created.", destination.display()))
}

fn prepare_runtime(
    app: &AppHandle,
    product: Product,
) -> Result<(PathBuf, PathBuf, PathBuf), String> {
    let files = product.files();
    terminate(product);
    settle_process_handles();

    let runtime_dir = app
        .path()
        .resolve(files.runtime_dir, BaseDirectory::AppLocalData)
        .map_err(|error| format!("Failed to resolve runtime payload directory: {error}"))?;
    fs::create_dir_all(&runtime_dir).map_err(|error| {
        format!(
            "Failed to create runtime payload directory {}: {error}",
            runtime_dir.display()
        )
    })?;

    let executable = write_runtime_file(product, &files, files.executable, &runtime_dir)?;
    let library = write_runtime_file(product, &files, files.library, &runtime_dir)?;

    let runtime_dir = processes::canonical_runtime_path(&runtime_dir);
    let executable = processes::canonical_runtime_path(&executable);
    let library = processes::canonical_runtime_path(&library);
    if executable.parent() != Some(runtime_dir.as_path())
        || library.parent() != Some(runtime_dir.as_path())
    {
        return Err(format!(
            "Payload files must be in the same runtime directory. exe: {}, dll: {}, dir: {}",
            executable.display(),
            library.display(),
            runtime_dir.display()
        ));
    }

    Ok((runtime_dir, executable, library))
}

pub fn launch(app: &AppHandle, product: Product) -> Result<(), String> {
    let (runtime_dir, executable, _library) = prepare_runtime(app, product)?;
    settle_process_handles();
    processes::launch_hidden(&executable, &runtime_dir)
}

pub fn extract_lua_libraries(libraries_dir: &Path) -> Result<(), String> {
    fs::create_dir_all(libraries_dir).map_err(|error| {
        format!(
            "Failed to create libraries directory {}: {error}",
            libraries_dir.display()
        )
    })?;

    let mut archive = ZipArchive::new(Cursor::new(LUA_LIBRARIES_ARCHIVE))
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

        let mut output = File::create(&output_path)
            .map_err(|error| format!("Failed to create {}: {error}", output_path.display()))?;
        io::copy(&mut entry, &mut output)
            .map_err(|error| format!("Failed to extract {}: {error}", output_path.display()))?;
        extracted_files += 1;
    }

    if extracted_files == 0 {
        return Err(format!(
            "Embedded ZIP archive {LUA_ARCHIVE} did not contain any files to extract."
        ));
    }

    let installed_files = count_files(libraries_dir).map_err(|error| {
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

fn count_files(path: &Path) -> io::Result<usize> {
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

#[cfg(test)]
mod tests {
    use super::{read_archive_file, Product, LUA_LIBRARIES_ARCHIVE};
    use std::io::Cursor;
    use zip::ZipArchive;

    #[test]
    fn every_product_archive_contains_its_executable_and_library() {
        for product in [Product::Neverlose, Product::Primordial, Product::Gamesense] {
            let files = product.files();
            assert!(!read_archive_file(&files, files.executable)
                .unwrap()
                .is_empty());
            assert!(!read_archive_file(&files, files.library).unwrap().is_empty());
        }
    }

    #[test]
    fn lua_archive_contains_files() {
        let mut archive = ZipArchive::new(Cursor::new(LUA_LIBRARIES_ARCHIVE)).unwrap();
        let files = (0..archive.len())
            .filter(|index| archive.by_index(*index).is_ok_and(|entry| entry.is_file()))
            .count();
        assert!(files > 0);
    }
}
