//! rakukan-engine DLL の動的ローダー
//!
//! `DynEngine` は `RakunEngine` と同じ API を持ち、実行時に選択された
//! バックエンチEDLL�E�Euda/vulkan/cpu�E�に処琁E��委譲する、E//!
//! # バックエンド選択頁E//! 1. `config.toml` の `gpu_backend` キー�E�Ecuda` / `vulkan` / `cpu` / `auto`�E�E//! 2. キー未持E��また�E `auto` の場合�E、インスト�Eル済みの DLL めE//!    `cuda` ↁE`vulkan` ↁE`cpu` の頁E��探索して採用する、E//!
//! # DLL ファイル吁E//! `rakukan_engine_<backend>.dll` がインスト�EルチE��レクトリに存在すること、E
use std::ffi::{CStr, CString, c_char, c_void};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use libloading::{Library, Symbol};

const EXPECTED_ENGINE_ABI_VERSION: u32 = 9;

// ─── Segments モチE���E�EONVERTER_REDESIGN Phase A�E�E────────────────────────────

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum CandidateSource {
    Llm,
    Dict,
    History,
    Digit,
    Literal,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Candidate {
    pub surface: String,
    pub source: CandidateSource,
    pub annotation: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Segment {
    pub reading: String,
    pub candidates: Vec<Candidate>,
    pub selected: usize,
    pub fixed: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Segments {
    pub segments: Vec<Segment>,
    pub history_size: usize,
    pub focused: usize,
}

impl Segments {
    pub fn compose_surface(&self) -> String {
        self.segments
            .iter()
            .map(|s| {
                s.candidates
                    .get(s.selected)
                    .map(|c| c.surface.as_str())
                    .unwrap_or("")
            })
            .collect()
    }

    pub fn compose_reading(&self) -> String {
        self.segments.iter().map(|s| s.reading.as_str()).collect()
    }

    pub fn empty() -> Self {
        Segments {
            segments: vec![],
            history_size: 0,
            focused: 0,
        }
    }
}

// ─── EngineVTable ──────────────────────────────────────────────────────────────
// DLL からロードした関数ポインタのコレクション

struct EngineVTable {
    // ライフサイクル
    create: unsafe extern "C" fn(*const c_char) -> *mut c_void,
    destroy: unsafe extern "C" fn(*mut c_void),
    free_string: unsafe extern "C" fn(*mut c_char),

    // 入力
    push_char: unsafe extern "C" fn(*mut c_void, u32) -> u8,
    push_raw: unsafe extern "C" fn(*mut c_void, u32),
    push_fullwidth_alpha: unsafe extern "C" fn(*mut c_void, u32),
    backspace: unsafe extern "C" fn(*mut c_void) -> bool,
    flush_n: unsafe extern "C" fn(*mut c_void) -> bool,

    // プリエディット状態
    preedit_display: unsafe extern "C" fn(*mut c_void) -> *mut c_char,
    preedit_is_empty: unsafe extern "C" fn(*mut c_void) -> bool,
    hiragana_text: unsafe extern "C" fn(*mut c_void) -> *mut c_char,
    romaji_log_str: unsafe extern "C" fn(*mut c_void) -> *mut c_char,
    hiragana_from_romaji_log: unsafe extern "C" fn(*mut c_void) -> *mut c_char,
    committed_text: unsafe extern "C" fn(*mut c_void) -> *mut c_char,

    // BG 変換
    bg_start: unsafe extern "C" fn(*mut c_void, u32) -> bool,
    bg_status: unsafe extern "C" fn(*mut c_void) -> *const c_char,
    bg_take_candidates: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_char,
    bg_peek_top_candidate: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_char,
    bg_reclaim: unsafe extern "C" fn(*mut c_void),
    bg_wait_ms: unsafe extern "C" fn(*mut c_void, u64) -> u8,

    // 確定/リセット
    commit: unsafe extern "C" fn(*mut c_void, *const c_char),
    commit_as_hiragana: unsafe extern "C" fn(*mut c_void),
    reset_preedit: unsafe extern "C" fn(*mut c_void),
    force_preedit: unsafe extern "C" fn(*mut c_void, *const c_char),
    reset_all: unsafe extern "C" fn(*mut c_void),

    // 変換/同期
    convert_sync: unsafe extern "C" fn(*mut c_void) -> *mut c_char,
    merge_candidates: unsafe extern "C" fn(*mut c_void, *const c_char, u32) -> *mut c_char,
    merge_candidates_for_reading:
        unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char, u32) -> *mut c_char,

    // 非同期初期化
    start_load_model: unsafe extern "C" fn(*mut c_void),
    poll_model_ready: unsafe extern "C" fn(*mut c_void) -> bool,
    start_load_dict: unsafe extern "C" fn(*mut c_void),
    poll_dict_ready: unsafe extern "C" fn(*mut c_void) -> bool,

    // ステータス
    is_kanji_ready: unsafe extern "C" fn(*mut c_void) -> bool,
    is_dict_ready: unsafe extern "C" fn(*mut c_void) -> bool,
    backend_label: unsafe extern "C" fn(*mut c_void) -> *mut c_char,
    n_gpu_layers: unsafe extern "C" fn(*mut c_void) -> u32,
    main_gpu: unsafe extern "C" fn(*mut c_void) -> i32,

    // Static
    available_models_json: unsafe extern "C" fn() -> *mut c_char,

    // 学習
    learn: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char),
    learn_force: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char),

    // 診断
    last_error: unsafe extern "C" fn() -> *mut c_char,
    dict_status: unsafe extern "C" fn() -> *mut c_char,
}

// ─── DLL ローチE────────────────────────────────────────────────────────────────

macro_rules! load_sym {
    ($lib:expr, $name:literal) => {{
        let sym: Symbol<_> = unsafe {
            $lib.get($name)
                .context(concat!("symbol not found: ", stringify!($name)))?
        };
        *sym
    }};
}

macro_rules! load_sym_opt {
    ($lib:expr, $name:literal) => {{
        let sym = unsafe { $lib.get($name) };
        sym.ok().map(|sym: Symbol<_>| *sym)
    }};
}

impl EngineVTable {
    unsafe fn load(lib: &Library) -> Result<Self> {
        let abi_version: Option<unsafe extern "C" fn() -> u32> =
            load_sym_opt!(lib, b"engine_abi_version\0");
        let Some(abi_version) = abi_version else {
            bail!(
                "installed engine DLL is outdated: missing engine_abi_version; run `cargo make full-install`"
            );
        };
        let actual = unsafe { abi_version() };
        if actual != EXPECTED_ENGINE_ABI_VERSION {
            bail!(
                "installed engine DLL ABI mismatch: expected {}, got {}; run `cargo make full-install`",
                EXPECTED_ENGINE_ABI_VERSION,
                actual
            );
        }

        Ok(EngineVTable {
            create: load_sym!(lib, b"engine_create\0"),
            destroy: load_sym!(lib, b"engine_destroy\0"),
            free_string: load_sym!(lib, b"engine_free_string\0"),
            push_char: load_sym!(lib, b"engine_push_char\0"),
            push_raw: load_sym!(lib, b"engine_push_raw\0"),
            push_fullwidth_alpha: load_sym!(lib, b"engine_push_fullwidth_alpha\0"),
            backspace: load_sym!(lib, b"engine_backspace\0"),
            flush_n: load_sym!(lib, b"engine_flush_n\0"),
            preedit_display: load_sym!(lib, b"engine_preedit_display\0"),
            preedit_is_empty: load_sym!(lib, b"engine_preedit_is_empty\0"),
            hiragana_text: load_sym!(lib, b"engine_hiragana_text\0"),
            romaji_log_str: load_sym!(lib, b"engine_romaji_log_str\0"),
            hiragana_from_romaji_log: load_sym!(lib, b"engine_hiragana_from_romaji_log\0"),
            committed_text: load_sym!(lib, b"engine_committed_text\0"),
            bg_start: load_sym!(lib, b"engine_bg_start\0"),
            bg_status: load_sym!(lib, b"engine_bg_status\0"),
            bg_take_candidates: load_sym!(lib, b"engine_bg_take_candidates\0"),
            bg_peek_top_candidate: load_sym!(lib, b"engine_bg_peek_top_candidate\0"),
            bg_reclaim: load_sym!(lib, b"engine_bg_reclaim\0"),
            bg_wait_ms: load_sym!(lib, b"engine_bg_wait_ms\0"),
            commit: load_sym!(lib, b"engine_commit\0"),
            commit_as_hiragana: load_sym!(lib, b"engine_commit_as_hiragana\0"),
            reset_preedit: load_sym!(lib, b"engine_reset_preedit\0"),
            force_preedit: load_sym!(lib, b"engine_force_preedit\0"),
            reset_all: load_sym!(lib, b"engine_reset_all\0"),
            convert_sync: load_sym!(lib, b"engine_convert_sync\0"),
            merge_candidates: load_sym!(lib, b"engine_merge_candidates\0"),
            merge_candidates_for_reading: load_sym!(lib, b"engine_merge_candidates_for_reading\0"),
            start_load_model: load_sym!(lib, b"engine_start_load_model\0"),
            poll_model_ready: load_sym!(lib, b"engine_poll_model_ready\0"),
            start_load_dict: load_sym!(lib, b"engine_start_load_dict\0"),
            poll_dict_ready: load_sym!(lib, b"engine_poll_dict_ready\0"),
            is_kanji_ready: load_sym!(lib, b"engine_is_kanji_ready\0"),
            is_dict_ready: load_sym!(lib, b"engine_is_dict_ready\0"),
            backend_label: load_sym!(lib, b"engine_backend_label\0"),
            n_gpu_layers: load_sym!(lib, b"engine_n_gpu_layers\0"),
            main_gpu: load_sym!(lib, b"engine_main_gpu\0"),
            available_models_json: load_sym!(lib, b"engine_available_models_json\0"),
            learn: load_sym!(lib, b"engine_learn\0"),
            learn_force: load_sym!(lib, b"engine_learn_force\0"),
            last_error: load_sym!(lib, b"engine_last_error\0"),
            dict_status: load_sym!(lib, b"engine_dict_status\0"),
        })
    }
}

// ─── DynEngine ────────────────────────────────────────────────────────────────

/// 動的にロードされた rakukan-engine DLL。
/// `RakunEngine` と同じ API を提供する。
pub struct DynEngine {
    handle: *mut c_void,
    vtable: EngineVTable,
    _lib: Arc<Library>,
}

unsafe impl Send for DynEngine {}
unsafe impl Sync for DynEngine {}

impl DynEngine {
    /// DLL パスからエンジンを生成する。
    pub fn from_dll(dll_path: &Path, config_json: Option<&str>) -> Result<Self> {
        tracing::info!("Loading engine DLL: {}", dll_path.display());
        let lib = unsafe { Library::new(dll_path) }
            .with_context(|| format!("DLL load failed: {}", dll_path.display()))?;
        let vtable = unsafe { EngineVTable::load(&lib) }?;

        let handle = unsafe {
            let cfg = config_json.and_then(|s| CString::new(s).ok());
            let ptr = cfg.as_ref().map(|c| c.as_ptr()).unwrap_or(std::ptr::null());
            (vtable.create)(ptr)
        };
        if handle.is_null() {
            bail!("engine_create returned null");
        }

        Ok(DynEngine {
            handle,
            vtable,
            _lib: Arc::new(lib),
        })
    }

    /// バックエンドを自動検�Eして適刁E�� DLL をロードする、E    ///
    /// `install_dir`: rakukan DLL が�E置されてぁE��チE��レクトリ
    /// `config_json`: EngineConfig JSON�E�Eull の場合�EチE��ォルト！E    
    pub fn load_auto(install_dir: &Path, config_json: Option<&str>) -> Result<Self> {
        let backend = match detect_backend() {
            BackendSelection::Explicit(b) => {
                tracing::info!("Selected backend (explicit): {b}");
                b
            }
            BackendSelection::Auto => {
                let b = detect_best_installed_backend(install_dir);
                tracing::info!("Selected backend (auto): {b}");
                b
            }
        };
        Self::load_backend(install_dir, &backend, config_json)
    }

    /// 持E��バチE��エンド名の DLL をロードする、E    
    pub fn load_backend(
        install_dir: &Path,
        backend: &str,
        config_json: Option<&str>,
    ) -> Result<Self> {
        let dll_name = format!("rakukan_engine_{}.dll", backend);
        let dll_path = install_dir.join(&dll_name);
        if !dll_path.exists() {
            // フォールバック: cpu
            if backend != "cpu" {
                tracing::warn!("{} not found, falling back to cpu", dll_name);
                return Self::load_backend(install_dir, "cpu", config_json);
            }
            bail!("engine DLL not found: {}", dll_path.display());
        }
        Self::from_dll(&dll_path, config_json)
    }

    // ── ヘルパ�E ───────────────────────────────────────────────────────────

    /// DLL が返しぁEC 斁E���EめERust String に変換して解放する
    unsafe fn take_cstr(&self, ptr: *mut c_char) -> Option<String> {
        if ptr.is_null() {
            return None;
        }
        let s = unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() };
        unsafe { (self.vtable.free_string)(ptr) };
        Some(s)
    }

    /// Rust &str から CString への一時的な変換。
    fn to_cstring(s: &str) -> CString {
        CString::new(s.replace('\0', "")).unwrap_or_default()
    }

    // ── 斁E���E劁E────────────────────────────────────────────────────────────

    pub fn push_char(&mut self, c: char) {
        unsafe {
            (self.vtable.push_char)(self.handle, c as u32);
        }
    }

    pub fn push_raw(&mut self, c: char) {
        unsafe {
            (self.vtable.push_raw)(self.handle, c as u32);
        }
    }

    pub fn push_fullwidth_alpha(&mut self, c: char) {
        unsafe {
            (self.vtable.push_fullwidth_alpha)(self.handle, c as u32);
        }
    }

    pub fn backspace(&mut self) -> bool {
        unsafe { (self.vtable.backspace)(self.handle) }
    }

    pub fn flush_pending_n(&mut self) -> bool {
        unsafe { (self.vtable.flush_n)(self.handle) }
    }

    // ── プリエチE��チE��状慁E──────────────────────────────────────────────────

    pub fn preedit_display(&self) -> String {
        unsafe {
            let ptr = (self.vtable.preedit_display)(self.handle);
            self.take_cstr(ptr).unwrap_or_default()
        }
    }

    pub fn preedit_is_empty(&self) -> bool {
        unsafe { (self.vtable.preedit_is_empty)(self.handle) }
    }

    pub fn hiragana_text(&self) -> String {
        unsafe {
            let ptr = (self.vtable.hiragana_text)(self.handle);
            self.take_cstr(ptr).unwrap_or_default()
        }
    }

    pub fn romaji_log_str(&self) -> String {
        unsafe {
            let ptr = (self.vtable.romaji_log_str)(self.handle);
            self.take_cstr(ptr).unwrap_or_default()
        }
    }

    pub fn hiragana_from_romaji_log(&self) -> String {
        unsafe {
            let ptr = (self.vtable.hiragana_from_romaji_log)(self.handle);
            self.take_cstr(ptr).unwrap_or_default()
        }
    }

    pub fn committed_text(&self) -> String {
        unsafe {
            let ptr = (self.vtable.committed_text)(self.handle);
            self.take_cstr(ptr).unwrap_or_default()
        }
    }

    // ── BG 変換 ─────────────────────────────────────────────────────────────

    /// BG 変換を起動する。true = 起動しぁE    
    pub fn bg_start(&mut self, n_cands: usize) -> bool {
        unsafe { (self.vtable.bg_start)(self.handle, n_cands as u32) }
    }

    /// BG 状態文字�E�E�診断用�E�E    
    pub fn bg_status(&self) -> &'static str {
        unsafe {
            let ptr = (self.vtable.bg_status)(self.handle);
            CStr::from_ptr(ptr).to_str().unwrap_or("unknown")
        }
    }

    /// key が一致する BG 変換結果を取得する、E    
    pub fn bg_take_candidates(&mut self, key: &str) -> Option<Vec<String>> {
        let ckey = Self::to_cstring(key);
        unsafe {
            let ptr = (self.vtable.bg_take_candidates)(self.handle, ckey.as_ptr());
            let json = self.take_cstr(ptr)?;
            serde_json::from_str(&json).ok()
        }
    }

    /// M2 §5.2: ライブ変換 preview 用、トチE�E候補だけを覗き見る (cache を進めなぁE、E    
    pub fn bg_peek_top_candidate(&self, key: &str) -> Option<String> {
        let ckey = Self::to_cstring(key);
        unsafe {
            let ptr = (self.vtable.bg_peek_top_candidate)(self.handle, ckey.as_ptr());
            self.take_cstr(ptr)
        }
    }

    /// Done 状態�E converter めEengine に戻ぁE    
    pub fn bg_reclaim(&mut self) {
        unsafe {
            (self.vtable.bg_reclaim)(self.handle);
        }
    }

    /// BG 変換完亁E��最大 `timeout_ms` ミリ秒ブロチE��征E��する、E    /// Done になれ�E `true`、タイムアウトなめE`false`、E    
    pub fn bg_wait_ms(&mut self, timeout_ms: u64) -> bool {
        unsafe { (self.vtable.bg_wait_ms)(self.handle, timeout_ms) != 0 }
    }

    // ── 確定�EリセチE�� ──────────────────────────────────────────────────────

    pub fn commit(&mut self, text: &str) {
        let cs = Self::to_cstring(text);
        unsafe {
            (self.vtable.commit)(self.handle, cs.as_ptr());
        }
    }

    pub fn commit_as_hiragana(&mut self) {
        unsafe {
            (self.vtable.commit_as_hiragana)(self.handle);
        }
    }

    pub fn reset_preedit(&mut self) {
        unsafe {
            (self.vtable.reset_preedit)(self.handle);
        }
    }

    pub fn force_preedit(&mut self, text: String) {
        let c = std::ffi::CString::new(text.replace('\0', "")).unwrap_or_default();
        unsafe {
            (self.vtable.force_preedit)(self.handle, c.as_ptr());
        }
    }

    pub fn reset_all(&mut self) {
        unsafe {
            (self.vtable.reset_all)(self.handle);
        }
    }

    // ── 変換�E�同期フォールバック�E�──────────────────────────────────────────

    pub fn convert_sync(&mut self) -> Vec<String> {
        unsafe {
            let ptr = (self.vtable.convert_sync)(self.handle);
            match self.take_cstr(ptr) {
                Some(json) => serde_json::from_str(&json).unwrap_or_default(),
                None => vec![],
            }
        }
    }

    pub fn merge_candidates(&self, llm_cands: Vec<String>, limit: usize) -> Vec<String> {
        let json = serde_json::to_string(&llm_cands).unwrap_or_else(|_| "[]".into());
        let cjson = Self::to_cstring(&json);
        unsafe {
            let ptr = (self.vtable.merge_candidates)(self.handle, cjson.as_ptr(), limit as u32);
            match self.take_cstr(ptr) {
                Some(s) => serde_json::from_str(&s).unwrap_or_default(),
                None => vec![],
            }
        }
    }

    pub fn merge_candidates_for_reading(
        &self,
        reading: &str,
        llm_cands: Vec<String>,
        limit: usize,
    ) -> Vec<String> {
        let creading = Self::to_cstring(reading);
        let json = serde_json::to_string(&llm_cands).unwrap_or_else(|_| "[]".into());
        let cjson = Self::to_cstring(&json);
        unsafe {
            let ptr = (self.vtable.merge_candidates_for_reading)(
                self.handle,
                creading.as_ptr(),
                cjson.as_ptr(),
                limit as u32,
            );
            match self.take_cstr(ptr) {
                Some(s) => serde_json::from_str(&s).unwrap_or_default(),
                None => vec![],
            }
        }
    }

    // ── 非同期�E期化 ────────────────────────────────────────────────────────

    pub fn start_load_model(&mut self) {
        unsafe {
            (self.vtable.start_load_model)(self.handle);
        }
    }

    /// true = モチE��が新たに利用可能になった！Eangbar 更新トリガー�E�E    
    pub fn poll_model_ready(&mut self) -> bool {
        unsafe { (self.vtable.poll_model_ready)(self.handle) }
    }

    pub fn start_load_dict(&mut self) {
        unsafe {
            (self.vtable.start_load_dict)(self.handle);
        }
    }

    /// true = 辞書が新たに利用可能になっぁE    
    pub fn poll_dict_ready(&mut self) -> bool {
        unsafe { (self.vtable.poll_dict_ready)(self.handle) }
    }

    // ── スチE�Eタス ──────────────────────────────────────────────────────────

    pub fn is_kanji_ready(&self) -> bool {
        unsafe { (self.vtable.is_kanji_ready)(self.handle) }
    }

    pub fn is_dict_ready(&self) -> bool {
        unsafe { (self.vtable.is_dict_ready)(self.handle) }
    }

    pub fn backend_label(&self) -> String {
        unsafe {
            let ptr = (self.vtable.backend_label)(self.handle);
            self.take_cstr(ptr).unwrap_or_else(|| "unknown".into())
        }
    }

    pub fn n_gpu_layers(&self) -> u32 {
        unsafe { (self.vtable.n_gpu_layers)(self.handle) }
    }

    pub fn main_gpu(&self) -> i32 {
        unsafe { (self.vtable.main_gpu)(self.handle) }
    }

    pub fn available_models_json(&self) -> String {
        unsafe {
            let ptr = (self.vtable.available_models_json)();
            self.take_cstr(ptr).unwrap_or_else(|| "[]".into())
        }
    }

    pub fn learn(&mut self, reading: &str, surface: &str) {
        let r = Self::to_cstring(reading);
        let s = Self::to_cstring(surface);
        unsafe {
            (self.vtable.learn)(self.handle, r.as_ptr(), s.as_ptr());
        }
    }

    pub fn learn_force(&mut self, reading: &str, surface: &str) {
        let r = Self::to_cstring(reading);
        let s = Self::to_cstring(surface);
        unsafe {
            (self.vtable.learn_force)(self.handle, r.as_ptr(), s.as_ptr());
        }
    }

    /// エンジン DLL 側の最後�Eエラー/スチE�EタスメチE��ージを返す�E�診断用�E�E    
    pub fn last_error(&self) -> String {
        let ptr = unsafe { (self.vtable.last_error)() };
        if ptr.is_null() {
            return String::new();
        }
        let s = unsafe { std::ffi::CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned();
        unsafe { (self.vtable.free_string)(ptr) };
        s
    }

    pub fn dict_status(&self) -> String {
        let ptr = unsafe { (self.vtable.dict_status)() };
        if ptr.is_null() {
            return String::new();
        }
        let s = unsafe { std::ffi::CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned();
        unsafe { (self.vtable.free_string)(ptr) };
        s
    }
}

impl Drop for DynEngine {
    fn drop(&mut self) {
        unsafe {
            (self.vtable.destroy)(self.handle);
        }
    }
}

// ─── バックエンド�E動検�E ──────────────────────────────────────────────────────

/// config.toml の gpu_backend ��をどぁE��釈するか、E
enum BackendSelection {
    /// `cuda` / `vulkan` / `cpu` のぁE��れかが�E示されてぁE��
    Explicit(String),
    Auto,
}

/// config.toml の `gpu_backend` キーを読み取り、�E示持E��E/ auto を区別して返す、E
fn detect_backend() -> BackendSelection {
    match read_config_toml_backend() {
        Some(b) if matches!(b.as_str(), "cuda" | "vulkan" | "cpu") => {
            tracing::debug!("backend::select: from config.toml={b}");
            BackendSelection::Explicit(b)
        }
        Some(b) => {
            // "auto" もしく�E想定外文字�E ↁE自動検�Eにフォールバック
            tracing::debug!("backend::select: config.toml={b} -> auto");
            BackendSelection::Auto
        }
        None => {
            tracing::debug!("backend::select: gpu_backend not set -> auto");
            BackendSelection::Auto
        }
    }
}

/// インスト�Eル済みの DLL めE`cuda` ↁE`vulkan` ↁE`cpu` の頁E��探索して
/// 最良のバックエンド名を返す。どれも見つからなければ `cpu` を返す
/// �E�Eload_backend` 側の最終フォールバックで適刁E��エラーになる）、E
fn detect_best_installed_backend(install_dir: &Path) -> String {
    for backend in ["cuda", "vulkan", "cpu"] {
        let dll = install_dir.join(format!("rakukan_engine_{}.dll", backend));
        if dll.exists() {
            return backend.to_string();
        }
    }
    "cpu".to_string()
}

fn appdata_rakukan() -> Option<PathBuf> {
    let appdata = std::env::var("APPDATA").ok()?;
    Some(PathBuf::from(appdata).join("rakukan"))
}

fn read_config_toml_backend() -> Option<String> {
    let path = appdata_rakukan()?.join("config.toml");
    let text = match std::fs::read(&path) {
        Ok(bytes) => {
            String::from_utf8_lossy(&bytes).to_string()
        }
        Err(_e) => {
            return None;
        }
    };
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("gpu_backend") {
            let rest = rest.trim().trim_start_matches('=').trim();
            let val = rest
                .split('#')
                .next()
                .unwrap_or("")
                .trim()
                .trim_matches('"')
                .trim_matches('\'');
            if matches!(val, "cuda" | "vulkan" | "cpu" | "auto") {
                return Some(val.to_string());
            }
        }
    }
    None
}

// ─── DLL ディレクトリ検出 ──────────────────────────────────────────────────────

#[cfg(target_os = "windows")]
pub fn install_dir() -> Option<PathBuf> {
    let appdata = std::env::var("LOCALAPPDATA").ok()?;
    Some(PathBuf::from(appdata).join("rakukan"))
}

#[cfg(not(target_os = "windows"))]
pub fn install_dir() -> Option<PathBuf> {
    Some(PathBuf::from("/usr/local/lib/rakukan"))
}
