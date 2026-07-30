pub const EVENT_NAME: &str = "setup://status";
pub const CSGO_APP_DIR: &str = "Counter-Strike Global Offensive";
pub const CSGO_EXE: &str = "csgo.exe";
pub const CSGO_APP_IDS: &[&str] = &["730", "4465480"];
pub const CSGO_PATH_HINT_FILE: &str = "csgo_path.txt";
pub const CSGO_PATH_CACHE_FILE: &str = "csgo_path_cache.txt";
pub const CSGO_PATH_ENV_VARS: &[&str] = &["LOADER_CSGO_PATH", "CSGO_PATH", "CSGO_DIR"];
pub const GAME_LIBRARY_PATH: &[&str] = &["nl_cloud", "scripts", "libraries"];
pub const MAX_USERNAME_LENGTH: usize = 13;

pub const STRUCTURAL_SCAN_MAX_DEPTH: usize = 6;
pub const STRUCTURAL_SCAN_MAX_SECONDS: u64 = 10;
pub const STRUCTURAL_SCAN_MAX_VISITED_DIRS: usize = 10_000;
pub const STRUCTURAL_SCAN_MAX_CANDIDATES: usize = 24;

pub const UTILITY_EXE: &str = "injector.exe";
pub const UTILITY_DLL: &str = "neverlose.dll";
pub const UTILITY_RUNTIME_DIR: &str = "payload";
pub const UTILITY_PAYLOAD_ARCHIVE: &str = "neverlose-payload.zip";

pub const PRIMO_EXE: &str = "primo.exe";
pub const PRIMO_DLL: &str = "primordial-csgo.dll";
pub const PRIMO_RUNTIME_DIR: &str = "primordial-payload";
pub const PRIMO_PAYLOAD_ARCHIVE: &str = "primordial-payload.zip";

pub const GAMESENSE_EXE: &str = "skeet-insecure.exe";
pub const GAMESENSE_DLL: &str = "skeet.dll";
pub const GAMESENSE_RUNTIME_DIR: &str = "gamesense-payload";
pub const GAMESENSE_PAYLOAD_ARCHIVE: &str = "gamesense-payload.zip";

pub const LUA_ARCHIVE: &str = "lua_libs.zip";
pub const PROCESS_SETTLE_DELAY_MS: u64 = 500;
pub const CREATE_NO_WINDOW: u32 = 0x08000000;
