//! Native `libmpv` (`libmpv-2.dll` / `libmpv.so.2` / `libmpv.2.dylib`) FFI bindings
//! and hardware-accelerated Win32 video surface host for Kura's built-in player.

use crate::klog;
use anyhow::{bail, Result};
use libloading::Library;
use std::ffi::{c_char, c_double, c_int, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

// ---------------------------------------------------------------------------
// libmpv C API Constants & Types (<mpv/client.h>)
// ---------------------------------------------------------------------------

pub const MPV_FORMAT_STRING: c_int = 1;
pub const MPV_FORMAT_FLAG: c_int = 3;
pub const MPV_FORMAT_INT64: c_int = 4;
pub const MPV_FORMAT_DOUBLE: c_int = 5;

pub const MPV_EVENT_NONE: c_int = 0;
pub const MPV_EVENT_SHUTDOWN: c_int = 1;
pub const MPV_EVENT_END_FILE: c_int = 7;
pub const MPV_EVENT_FILE_LOADED: c_int = 8;
pub const MPV_EVENT_PROPERTY_CHANGE: c_int = 22;

pub const MPV_END_FILE_REASON_EOF: c_int = 0;

#[repr(C)]
pub struct MpvEvent {
    pub event_id: c_int,
    pub error: c_int,
    pub reply_userdata: u64,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct MpvEventProperty {
    pub name: *const c_char,
    pub format: c_int,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct MpvEventEndFile {
    pub reason: c_int,
    pub error: c_int,
    pub playlist_entry_id: i64,
    pub playlist_insert_id: i64,
    pub playlist_insert_num_entries: c_int,
}

type FnMpvCreate = unsafe extern "C" fn() -> *mut c_void;
type FnMpvInitialize = unsafe extern "C" fn(ctx: *mut c_void) -> c_int;
type FnMpvDestroy = unsafe extern "C" fn(ctx: *mut c_void);
type FnMpvCommand = unsafe extern "C" fn(ctx: *mut c_void, args: *const *const c_char) -> c_int;
type FnMpvSetOption = unsafe extern "C" fn(
    ctx: *mut c_void,
    name: *const c_char,
    format: c_int,
    data: *const c_void,
) -> c_int;
type FnMpvSetOptionString =
    unsafe extern "C" fn(ctx: *mut c_void, name: *const c_char, data: *const c_char) -> c_int;
type FnMpvSetProperty = unsafe extern "C" fn(
    ctx: *mut c_void,
    name: *const c_char,
    format: c_int,
    data: *const c_void,
) -> c_int;
type FnMpvSetPropertyString =
    unsafe extern "C" fn(ctx: *mut c_void, name: *const c_char, data: *const c_char) -> c_int;
type FnMpvGetProperty = unsafe extern "C" fn(
    ctx: *mut c_void,
    name: *const c_char,
    format: c_int,
    data: *mut c_void,
) -> c_int;
type FnMpvGetPropertyString =
    unsafe extern "C" fn(ctx: *mut c_void, name: *const c_char) -> *mut c_char;
type FnMpvObserveProperty = unsafe extern "C" fn(
    ctx: *mut c_void,
    reply_userdata: u64,
    name: *const c_char,
    format: c_int,
) -> c_int;
type FnMpvWaitEvent = unsafe extern "C" fn(ctx: *mut c_void, timeout: c_double) -> *mut MpvEvent;
type FnMpvWakeup = unsafe extern "C" fn(ctx: *mut c_void);
type FnMpvFree = unsafe extern "C" fn(data: *mut c_void);

pub struct MpvApi {
    _lib: Library,
    pub path: PathBuf,
    create: FnMpvCreate,
    initialize: FnMpvInitialize,
    destroy: FnMpvDestroy,
    command: FnMpvCommand,
    set_option: FnMpvSetOption,
    set_option_string: FnMpvSetOptionString,
    set_property: FnMpvSetProperty,
    set_property_string: FnMpvSetPropertyString,
    get_property: FnMpvGetProperty,
    get_property_string: FnMpvGetPropertyString,
    observe_property: FnMpvObserveProperty,
    wait_event: FnMpvWaitEvent,
    wakeup: FnMpvWakeup,
    free: FnMpvFree,
}

unsafe impl Send for MpvApi {}
unsafe impl Sync for MpvApi {}

static MPV_API: OnceLock<Result<MpvApi, String>> = OnceLock::new();

fn candidate_libmpv_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let lib_names: &[&str] = if cfg!(windows) {
        &["libmpv-2.dll", "mpv-2.dll", "mpv-1.dll"]
    } else if cfg!(target_os = "macos") {
        &["libmpv.2.dylib", "libmpv.dylib"]
    } else {
        &["libmpv.so.2", "libmpv.so"]
    };

    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let mut dirs = vec![
                exe_dir.join("resources").join("mpv"),
                exe_dir.join("mpv"),
                exe_dir.to_path_buf(),
            ];
            if let Some(p1) = exe_dir.parent() {
                dirs.push(p1.join("Resources").join("resources").join("mpv"));
                dirs.push(p1.join("lib").join("kura").join("resources").join("mpv"));
                dirs.push(p1.join("resources").join("mpv"));
                if let Some(p2) = p1.parent() {
                    // Matches `src-tauri/target/debug/kura.exe` -> `src-tauri/resources/mpv`
                    dirs.push(p2.join("resources").join("mpv"));
                }
            }
            for d in dirs {
                for name in lib_names {
                    out.push(d.join(name));
                }
            }
        }
    }

    #[cfg(windows)]
    {
        if let Some(mpv_exe) = crate::player::detect(crate::player::Kind::Mpv) {
            if let Some(dir) = mpv_exe.parent() {
                for name in lib_names {
                    out.push(dir.join(name));
                }
            }
        }
        let win_dirs = [
            r"C:\Program Files\Jellyfin\Jellyfin Media Player",
            r"C:\Program Files\MPV Player",
            r"C:\Program Files\mpv",
            r"C:\Program Files\mpv.net",
        ];
        for d in win_dirs {
            let dir = PathBuf::from(d);
            for name in lib_names {
                out.push(dir.join(name));
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        for d in ["/opt/homebrew/lib", "/usr/local/lib"] {
            for name in lib_names {
                out.push(PathBuf::from(d).join(name));
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        for d in [
            "/usr/lib/x86_64-linux-gnu",
            "/usr/lib64",
            "/usr/lib",
            "/usr/local/lib",
        ] {
            for name in lib_names {
                out.push(PathBuf::from(d).join(name));
            }
        }
    }

    out
}

fn find_shaders_dir() -> Option<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            dirs.push(exe_dir.join("resources").join("shaders"));
            dirs.push(exe_dir.join("shaders"));
            if let Some(p1) = exe_dir.parent() {
                dirs.push(p1.join("Resources").join("resources").join("shaders"));
                dirs.push(p1.join("lib").join("kura").join("resources").join("shaders"));
                dirs.push(p1.join("resources").join("shaders"));
                if let Some(p2) = p1.parent() {
                    dirs.push(p2.join("resources").join("shaders"));
                }
            }
        }
    }
    #[cfg(debug_assertions)]
    {
        dirs.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources").join("shaders"));
    }

    dirs.into_iter()
        .find(|d| d.join("Anime4K_Clamp_Highlights.glsl").is_file())
}

/// Resolve a named Anime4K v4 preset (`"off"`, `"a_fast"`, `"a_hq"`, `"b_soft"`, `"c_upscale"`)
/// into a platform-joined `glsl-shaders` string (`;` on Windows, `:` on Unix) for `libmpv`.
pub fn resolve_shader_preset_paths(preset: &str) -> String {
    let files: &[&str] = match preset.trim().to_ascii_lowercase().as_str() {
        "a_fast" => &[
            "Anime4K_Clamp_Highlights.glsl",
            "Anime4K_Restore_CNN_M.glsl",
            "Anime4K_Upscale_CNN_x2_M.glsl",
            "Anime4K_AutoDownscalePre_x2.glsl",
            "Anime4K_AutoDownscalePre_x4.glsl",
        ],
        "a_hq" => &[
            "Anime4K_Clamp_Highlights.glsl",
            "Anime4K_Restore_CNN_VL.glsl",
            "Anime4K_Upscale_CNN_x2_VL.glsl",
            "Anime4K_AutoDownscalePre_x2.glsl",
            "Anime4K_AutoDownscalePre_x4.glsl",
        ],
        "b_soft" => &[
            "Anime4K_Clamp_Highlights.glsl",
            "Anime4K_Restore_CNN_Soft_M.glsl",
            "Anime4K_Upscale_CNN_x2_M.glsl",
            "Anime4K_AutoDownscalePre_x2.glsl",
            "Anime4K_AutoDownscalePre_x4.glsl",
        ],
        "c_upscale" => &[
            "Anime4K_Clamp_Highlights.glsl",
            "Anime4K_Upscale_CNN_x2_M.glsl",
            "Anime4K_AutoDownscalePre_x2.glsl",
            "Anime4K_AutoDownscalePre_x4.glsl",
        ],
        _ => return String::new(),
    };

    let Some(dir) = find_shaders_dir() else {
        klog!("[libmpv] Anime4K shaders directory not found for preset '{preset}'");
        return String::new();
    };

    let sep = if cfg!(windows) { ";" } else { ":" };
    let resolved: Vec<String> = files
        .iter()
        .map(|f| dir.join(f))
        .filter(|p| p.is_file())
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    resolved.join(sep)
}

#[cfg(windows)]
unsafe fn load_library_isolated(cand: &Path) -> std::result::Result<Library, String> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryExW(
            lp_lib_file_name: *const u16,
            h_file: *mut c_void,
            dw_flags: u32,
        ) -> *mut c_void;
        fn GetLastError() -> u32;
    }
    // LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR (0x0100) | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS (0x1000)
    // Resolves transitive dependencies strictly from the DLL's own directory and System32,
    // without mutating process-wide DLL search state or searching CWD (`H-15`).
    const FLAGS: u32 = 0x0000_1100;
    let wide: Vec<u16> = cand
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let handle = LoadLibraryExW(wide.as_ptr(), std::ptr::null_mut(), FLAGS);
    if handle.is_null() {
        return Err(format!("LoadLibraryExW failed (win32 error {})", GetLastError()));
    }
    let os_lib = libloading::os::windows::Library::from_raw(handle as _);
    Ok(Library::from(os_lib))
}

#[cfg(not(windows))]
unsafe fn load_library_isolated(cand: &Path) -> std::result::Result<Library, String> {
    Library::new(cand).map_err(|e| e.to_string())
}

impl MpvApi {
    pub fn get() -> Result<&'static MpvApi> {
        match MPV_API.get_or_init(Self::load) {
            Ok(api) => Ok(api),
            Err(e) => bail!("{e}"),
        }
    }

    fn load() -> std::result::Result<MpvApi, String> {
        let mut last_err = String::from("No verified libmpv candidates found");
        for cand in candidate_libmpv_paths() {
            if !cand.is_absolute() || !cand.is_file() {
                continue;
            }
            unsafe {
                match load_library_isolated(&cand) {
                    Ok(lib) => {
                        let res = (|| -> Result<MpvApi> {
                            let create = *lib.get::<FnMpvCreate>(b"mpv_create\0")?;
                            let initialize = *lib.get::<FnMpvInitialize>(b"mpv_initialize\0")?;
                            let destroy = *lib.get::<FnMpvDestroy>(b"mpv_destroy\0")?;
                            let command = *lib.get::<FnMpvCommand>(b"mpv_command\0")?;
                            let set_option = *lib.get::<FnMpvSetOption>(b"mpv_set_option\0")?;
                            let set_option_string =
                                *lib.get::<FnMpvSetOptionString>(b"mpv_set_option_string\0")?;
                            let set_property =
                                *lib.get::<FnMpvSetProperty>(b"mpv_set_property\0")?;
                            let set_property_string =
                                *lib.get::<FnMpvSetPropertyString>(b"mpv_set_property_string\0")?;
                            let get_property =
                                *lib.get::<FnMpvGetProperty>(b"mpv_get_property\0")?;
                            let get_property_string =
                                *lib.get::<FnMpvGetPropertyString>(b"mpv_get_property_string\0")?;
                            let observe_property =
                                *lib.get::<FnMpvObserveProperty>(b"mpv_observe_property\0")?;
                            let wait_event = *lib.get::<FnMpvWaitEvent>(b"mpv_wait_event\0")?;
                            let wakeup = *lib.get::<FnMpvWakeup>(b"mpv_wakeup\0")?;
                            let free = *lib.get::<FnMpvFree>(b"mpv_free\0")?;

                            klog!("[libmpv] Loaded native libmpv from {}", cand.display());
                            Ok(MpvApi {
                                _lib: lib,
                                path: cand.clone(),
                                create,
                                initialize,
                                destroy,
                                command,
                                set_option,
                                set_option_string,
                                set_property,
                                set_property_string,
                                get_property,
                                get_property_string,
                                observe_property,
                                wait_event,
                                wakeup,
                                free,
                            })
                        })();
                        match res {
                            Ok(api) => return Ok(api),
                            Err(e) => last_err = format!("{}: {e}", cand.display()),
                        }
                    }
                    Err(e) => {
                        last_err = format!("{}: {e}", cand.display());
                    }
                }
            }
        }
        Err(format!("Failed to load libmpv: {last_err}"))
    }
}

// ---------------------------------------------------------------------------
// Safe Wrapper Around `mpv_handle*`
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub struct MpvInstance {
    ctx: *mut c_void,
    api: &'static MpvApi,
}

unsafe impl Send for MpvInstance {}
unsafe impl Sync for MpvInstance {}

impl MpvInstance {
    pub fn new(wid: Option<i64>) -> Result<Self> {
        let api = MpvApi::get()?;
        let ctx = unsafe { (api.create)() };
        if ctx.is_null() {
            bail!("mpv_create() returned NULL");
        }
        let inst = Self { ctx, api };

        if let Some(w) = wid {
            inst.set_option_i64("wid", w)?;
        }

        // Configure hardware decoding & clean embedded rendering.
        // On Windows, pin gpu-api=d3d11 + d3d11va so mpv skips slow OpenGL WGL driver
        // probing (which can stall 400-900ms waking hybrid GPUs) and uses Direct3D 11 directly.
        #[cfg(windows)]
        {
            let _ = inst.set_option_string("vo", "gpu,gpu-next,direct3d");
            let _ = inst.set_option_string("gpu-api", "d3d11");
            let _ = inst.set_option_string("hwdec", "d3d11va,auto-safe");
        }
        #[cfg(not(windows))]
        {
            let _ = inst.set_option_string("vo", "gpu,gpu-next,xv,x11");
            let _ = inst.set_option_string("hwdec", "auto-safe");
        }
        let _ = inst.set_option_string("keep-open", "no");
        let _ = inst.set_option_string("idle", "yes");
        let _ = inst.set_option_string("osc", "no");
        let _ = inst.set_option_string("osd-level", "0");
        let _ = inst.set_option_string("osd-bar", "no");
        let _ = inst.set_option_string("input-default-bindings", "no");
        let _ = inst.set_option_string("input-vo-keyboard", "no");
        let _ = inst.set_option_string("cursor-autohide", "no");
        let _ = inst.set_option_string("sub-visibility", "no");
        let _ = inst.set_option_string("sub-auto", "exact");
        let _ = inst.set_option_string("audio-client-name", "Kura");

        let rc = unsafe { (api.initialize)(ctx) };
        if rc < 0 {
            unsafe { (api.destroy)(ctx) };
            bail!("mpv_initialize() failed with error code {rc}");
        }

        // Observe key playback & subtitle properties
        let _ = inst.observe_property(1, "time-pos", MPV_FORMAT_DOUBLE);
        let _ = inst.observe_property(2, "duration", MPV_FORMAT_DOUBLE);
        let _ = inst.observe_property(3, "pause", MPV_FORMAT_FLAG);
        let _ = inst.observe_property(4, "sub-text", MPV_FORMAT_STRING);
        let _ = inst.observe_property(5, "aid", MPV_FORMAT_INT64);

        Ok(inst)
    }

    pub fn set_option_i64(&self, name: &str, val: i64) -> Result<()> {
        let cname = CString::new(name)?;
        let rc = unsafe {
            (self.api.set_option)(
                self.ctx,
                cname.as_ptr(),
                MPV_FORMAT_INT64,
                &val as *const i64 as *const c_void,
            )
        };
        if rc < 0 {
            bail!("mpv_set_option({name}) failed: {rc}");
        }
        Ok(())
    }

    pub fn set_option_string(&self, name: &str, val: &str) -> Result<()> {
        let cname = CString::new(name)?;
        let cval = CString::new(val)?;
        let rc = unsafe { (self.api.set_option_string)(self.ctx, cname.as_ptr(), cval.as_ptr()) };
        if rc < 0 {
            bail!("mpv_set_option_string({name}={val}) failed: {rc}");
        }
        Ok(())
    }

    pub fn command(&self, args: &[&str]) -> Result<()> {
        let cstrings: Vec<CString> = args
            .iter()
            .map(|s| CString::new(*s).unwrap_or_default())
            .collect();
        let mut ptrs: Vec<*const c_char> = cstrings.iter().map(|cs| cs.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        let rc = unsafe { (self.api.command)(self.ctx, ptrs.as_ptr()) };
        if rc < 0 {
            bail!("mpv_command({args:?}) failed: {rc}");
        }
        Ok(())
    }

    pub fn set_property_string(&self, name: &str, val: &str) -> Result<()> {
        let cname = CString::new(name)?;
        let cval = CString::new(val)?;
        let rc = unsafe { (self.api.set_property_string)(self.ctx, cname.as_ptr(), cval.as_ptr()) };
        if rc < 0 {
            bail!("mpv_set_property_string({name}={val}) failed: {rc}");
        }
        Ok(())
    }

    pub fn set_property_bool(&self, name: &str, val: bool) -> Result<()> {
        let cname = CString::new(name)?;
        let flag: c_int = if val { 1 } else { 0 };
        let rc = unsafe {
            (self.api.set_property)(
                self.ctx,
                cname.as_ptr(),
                MPV_FORMAT_FLAG,
                &flag as *const c_int as *const c_void,
            )
        };
        if rc < 0 {
            bail!("mpv_set_property_bool({name}={val}) failed: {rc}");
        }
        Ok(())
    }

    pub fn set_property_i64(&self, name: &str, val: i64) -> Result<()> {
        let cname = CString::new(name)?;
        let rc = unsafe {
            (self.api.set_property)(
                self.ctx,
                cname.as_ptr(),
                MPV_FORMAT_INT64,
                &val as *const i64 as *const c_void,
            )
        };
        if rc < 0 {
            bail!("mpv_set_property_i64({name}={val}) failed: {rc}");
        }
        Ok(())
    }

    pub fn set_property_double(&self, name: &str, val: f64) -> Result<()> {
        let cname = CString::new(name)?;
        let rc = unsafe {
            (self.api.set_property)(
                self.ctx,
                cname.as_ptr(),
                MPV_FORMAT_DOUBLE,
                &val as *const f64 as *const c_void,
            )
        };
        if rc < 0 {
            bail!("mpv_set_property_double({name}={val}) failed: {rc}");
        }
        Ok(())
    }

    pub fn get_property_string(&self, name: &str) -> Option<String> {
        let cname = CString::new(name).ok()?;
        unsafe {
            let ptr = (self.api.get_property_string)(self.ctx, cname.as_ptr());
            if ptr.is_null() {
                return None;
            }
            let s = CStr::from_ptr(ptr).to_string_lossy().into_owned();
            (self.api.free)(ptr as *mut c_void);
            Some(s)
        }
    }

    pub fn get_property_i64(&self, name: &str) -> Option<i64> {
        let cname = CString::new(name).ok()?;
        let mut out: i64 = 0;
        let rc = unsafe {
            (self.api.get_property)(
                self.ctx,
                cname.as_ptr(),
                MPV_FORMAT_INT64,
                &mut out as *mut i64 as *mut c_void,
            )
        };
        if rc >= 0 {
            Some(out)
        } else {
            None
        }
    }

    pub fn get_property_double(&self, name: &str) -> Option<f64> {
        let cname = CString::new(name).ok()?;
        let mut out: f64 = 0.0;
        let rc = unsafe {
            (self.api.get_property)(
                self.ctx,
                cname.as_ptr(),
                MPV_FORMAT_DOUBLE,
                &mut out as *mut f64 as *mut c_void,
            )
        };
        if rc >= 0 {
            Some(out)
        } else {
            None
        }
    }

    pub fn get_property_bool(&self, name: &str) -> Option<bool> {
        let cname = CString::new(name).ok()?;
        let mut out: c_int = 0;
        let rc = unsafe {
            (self.api.get_property)(
                self.ctx,
                cname.as_ptr(),
                MPV_FORMAT_FLAG,
                &mut out as *mut c_int as *mut c_void,
            )
        };
        if rc >= 0 {
            Some(out != 0)
        } else {
            None
        }
    }

    pub fn observe_property(&self, userdata: u64, name: &str, format: c_int) -> Result<()> {
        let cname = CString::new(name)?;
        let rc = unsafe { (self.api.observe_property)(self.ctx, userdata, cname.as_ptr(), format) };
        if rc < 0 {
            bail!("mpv_observe_property({name}) failed: {rc}");
        }
        Ok(())
    }

    pub fn wait_event(&self, timeout_sec: f64) -> *mut MpvEvent {
        unsafe { (self.api.wait_event)(self.ctx, timeout_sec) }
    }

    pub fn wakeup(&self) {
        unsafe { (self.api.wakeup)(self.ctx) }
    }
}

// ---------------------------------------------------------------------------
// Win32 Hardware Video Surface Host Window (`--wid`)
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod win_surface {
    use super::*;
    use std::sync::mpsc;

    type HWND = isize;
    type HINSTANCE = isize;
    type HMENU = isize;
    type HICON = isize;
    type HCURSOR = isize;
    type HBRUSH = isize;
    type LRESULT = isize;
    type WPARAM = usize;
    type LPARAM = isize;
    type UINT = u32;
    type DWORD = u32;
    type BOOL = i32;

    const WS_POPUP: DWORD = 0x8000_0000;
    const WS_CLIPCHILDREN: DWORD = 0x0200_0000;
    const WS_CLIPSIBLINGS: DWORD = 0x0400_0000;
    const WS_DISABLED: DWORD = 0x0800_0000;

    const WS_EX_TOOLWINDOW: DWORD = 0x0000_0080;
    const WS_EX_NOACTIVATE: DWORD = 0x0800_0000;
    const WS_EX_APPWINDOW: isize = 0x0004_0000;

    const GWL_EXSTYLE: i32 = -20;
    const GWLP_HWNDPARENT: i32 = -8;

    const SWP_NOSIZE: UINT = 0x0001;
    const SWP_NOMOVE: UINT = 0x0002;
    const SWP_NOZORDER: UINT = 0x0004;
    const SWP_NOACTIVATE: UINT = 0x0010;
    const SWP_SHOWWINDOW: UINT = 0x0040;
    const SWP_HIDEWINDOW: UINT = 0x0080;
    const SWP_NOOWNERZORDER: UINT = 0x0200;
    const SWP_ASYNCWINDOWPOS: UINT = 0x4000;

    const BLACK_BRUSH: i32 = 4;
    const WM_ERASEBKGND: UINT = 0x0014;

    #[repr(C)]
    struct POINT {
        x: i32,
        y: i32,
    }

    #[repr(C)]
    struct RECT {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[repr(C)]
    struct WNDCLASSEXW {
        cb_size: UINT,
        style: UINT,
        lpfn_wnd_proc: unsafe extern "system" fn(HWND, UINT, WPARAM, LPARAM) -> LRESULT,
        cb_cls_extra: i32,
        cb_wnd_extra: i32,
        h_instance: HINSTANCE,
        h_icon: HICON,
        h_cursor: HCURSOR,
        hbr_background: HBRUSH,
        lpsz_menu_name: *const u16,
        lpsz_class_name: *const u16,
        h_icon_sm: HICON,
    }

    #[repr(C)]
    struct MSG {
        hwnd: HWND,
        message: UINT,
        w_param: WPARAM,
        l_param: LPARAM,
        time: DWORD,
        pt: POINT,
    }

    #[link(name = "user32")]
    extern "system" {
        fn RegisterClassExW(lpwcx: *const WNDCLASSEXW) -> u16;
        fn CreateWindowExW(
            dw_ex_style: DWORD,
            lp_class_name: *const u16,
            lp_window_name: *const u16,
            dw_style: DWORD,
            x: i32,
            y: i32,
            n_width: i32,
            n_height: i32,
            h_wnd_parent: HWND,
            h_menu: HMENU,
            h_instance: HINSTANCE,
            lp_param: *const c_void,
        ) -> HWND;
        fn DefWindowProcW(hwnd: HWND, msg: UINT, w_param: WPARAM, l_param: LPARAM) -> LRESULT;
        fn GetMessageW(lp_msg: *mut MSG, h_wnd: HWND, w_msg_filter_min: UINT, w_msg_filter_max: UINT) -> BOOL;
        fn TranslateMessage(lp_msg: *const MSG) -> BOOL;
        fn DispatchMessageW(lp_msg: *const MSG) -> LRESULT;
        fn SetWindowPos(
            h_wnd: HWND,
            h_wnd_insert_after: HWND,
            x: i32,
            y: i32,
            cx: i32,
            cy: i32,
            u_flags: UINT,
        ) -> BOOL;
        fn GetClientRect(h_wnd: HWND, lp_rect: *mut RECT) -> BOOL;
        fn ClientToScreen(h_wnd: HWND, lp_point: *mut POINT) -> BOOL;
        fn IsIconic(h_wnd: HWND) -> BOOL;
        fn IsWindowVisible(h_wnd: HWND) -> BOOL;
        fn GetWindowLongPtrW(h_wnd: HWND, n_index: i32) -> isize;
        fn SetWindowLongPtrW(h_wnd: HWND, n_index: i32, dw_new_long: isize) -> isize;
    }

    #[link(name = "gdi32")]
    extern "system" {
        fn GetStockObject(i: i32) -> HBRUSH;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(lp_module_name: *const u16) -> HINSTANCE;
    }

    unsafe extern "system" fn surface_wnd_proc(
        hwnd: HWND,
        msg: UINT,
        w_param: WPARAM,
        l_param: LPARAM,
    ) -> LRESULT {
        if msg == WM_ERASEBKGND {
            return 1;
        }
        DefWindowProcW(hwnd, msg, w_param, l_param)
    }

    static VIDEO_HWND: OnceLock<HWND> = OnceLock::new();
    static LAST_BOUNDS: Mutex<(i32, i32, i32, i32, bool)> = Mutex::new((0, 0, 0, 0, false));

    pub fn ensure_video_hwnd() -> Result<i64> {
        let hwnd = *VIDEO_HWND.get_or_init(|| {
            let (tx, rx) = mpsc::channel::<HWND>();
            std::thread::Builder::new()
                .name("kura-libmpv-surface".to_string())
                .spawn(move || unsafe {
                    let h_instance = GetModuleHandleW(std::ptr::null());
                    let class_name: Vec<u16> = "KuraLibmpvSurfaceClass\0".encode_utf16().collect();
                    let wnd_name: Vec<u16> = "KuraVideoSurface\0".encode_utf16().collect();

                    let wc = WNDCLASSEXW {
                        cb_size: std::mem::size_of::<WNDCLASSEXW>() as UINT,
                        style: 0,
                        lpfn_wnd_proc: surface_wnd_proc,
                        cb_cls_extra: 0,
                        cb_wnd_extra: 0,
                        h_instance,
                        h_icon: 0,
                        h_cursor: 0,
                        hbr_background: GetStockObject(BLACK_BRUSH),
                        lpsz_menu_name: std::ptr::null(),
                        lpsz_class_name: class_name.as_ptr(),
                        h_icon_sm: 0,
                    };
                    RegisterClassExW(&wc);

                    let hwnd = CreateWindowExW(
                        WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                        class_name.as_ptr(),
                        wnd_name.as_ptr(),
                        WS_POPUP | WS_CLIPCHILDREN | WS_CLIPSIBLINGS | WS_DISABLED,
                        0,
                        0,
                        1280,
                        720,
                        0,
                        0,
                        h_instance,
                        std::ptr::null(),
                    );
                    let _ = tx.send(hwnd);

                    let mut msg: MSG = std::mem::zeroed();
                    while GetMessageW(&mut msg, 0, 0, 0) > 0 {
                        TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                })
                .ok();
            rx.recv().unwrap_or(0)
        });

        if hwnd == 0 {
            bail!("Failed to create Win32 libmpv video surface window");
        }
        Ok(hwnd as i64)
    }

    pub fn is_surface_visible() -> bool {
        LAST_BOUNDS.lock().unwrap_or_else(|e| e.into_inner()).4
    }

    pub fn sync_surface_with_main(main_hwnd: isize, active_and_visible: bool) {
        let Some(&video_hwnd) = VIDEO_HWND.get() else {
            return;
        };
        if video_hwnd == 0 || main_hwnd == 0 {
            return;
        }

        unsafe {
            let hide_async_flags = SWP_HIDEWINDOW
                | SWP_NOMOVE
                | SWP_NOSIZE
                | SWP_NOZORDER
                | SWP_NOACTIVATE
                | SWP_NOOWNERZORDER
                | SWP_ASYNCWINDOWPOS;

            if !active_and_visible {
                let mut g = LAST_BOUNDS.lock().unwrap_or_else(|e| e.into_inner());
                if g.4 {
                    g.4 = false;
                    SetWindowLongPtrW(main_hwnd, GWLP_HWNDPARENT, 0);
                    SetWindowPos(video_hwnd, 0, 0, 0, 0, 0, hide_async_flags);
                }
                return;
            }

            if IsIconic(main_hwnd) != 0 || IsWindowVisible(main_hwnd) == 0 {
                let mut g = LAST_BOUNDS.lock().unwrap_or_else(|e| e.into_inner());
                if g.4 {
                    g.4 = false;
                    SetWindowPos(video_hwnd, 0, 0, 0, 0, 0, hide_async_flags);
                }
                return;
            }

            let mut client_rc = RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            if GetClientRect(main_hwnd, &mut client_rc) == 0 {
                return;
            }
            let mut pt = POINT {
                x: client_rc.left,
                y: client_rc.top,
            };
            ClientToScreen(main_hwnd, &mut pt);
            let w = (client_rc.right - client_rc.left).max(1);
            let h = (client_rc.bottom - client_rc.top).max(1);

            let mut g = LAST_BOUNDS.lock().unwrap_or_else(|e| e.into_inner());
            let bounds_changed = g.0 != pt.x || g.1 != pt.y || g.2 != w || g.3 != h || !g.4;

            if !g.4 {
                // Ensure main_hwnd keeps its Taskbar button even while owned by video_hwnd
                let ex_style = GetWindowLongPtrW(main_hwnd, GWL_EXSTYLE);
                if (ex_style & WS_EX_APPWINDOW) == 0 {
                    SetWindowLongPtrW(main_hwnd, GWL_EXSTYLE, ex_style | WS_EX_APPWINDOW);
                }
                // Make main_hwnd owned by video_hwnd so DWM guarantees main_hwnd (WebView2 HUD)
                // is always composited above video_hwnd (libmpv D3D11 swapchain).
                SetWindowLongPtrW(main_hwnd, GWLP_HWNDPARENT, video_hwnd);
            }

            if bounds_changed {
                *g = (pt.x, pt.y, w, h, true);
                SetWindowPos(
                    video_hwnd,
                    main_hwnd,
                    pt.x,
                    pt.y,
                    w,
                    h,
                    SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_SHOWWINDOW | SWP_ASYNCWINDOWPOS,
                );
            }
        }
    }
}

pub fn ensure_video_surface() -> Result<Option<i64>> {
    #[cfg(windows)]
    {
        Ok(Some(win_surface::ensure_video_hwnd()?))
    }
    #[cfg(not(windows))]
    {
        Ok(None)
    }
}

/// Keep the libmpv video surface glued behind Kura's main window.
///
/// Requests are coalesced: at most one job is queued on the main thread at a time, and
/// `SetWindowPos` only executes when `(x, y, w, h, visible)` actually changes (`H-23`).
pub fn sync_surface_bounds(app: &tauri::AppHandle, active_and_visible: bool) {
    #[cfg(windows)]
    {
        use std::sync::atomic::{AtomicBool, Ordering};
        use tauri::Manager;

        static JOB_QUEUED: AtomicBool = AtomicBool::new(false);
        static WANT_ACTIVE: AtomicBool = AtomicBool::new(false);

        if !active_and_visible
            && !WANT_ACTIVE.load(Ordering::SeqCst)
            && !win_surface::is_surface_visible()
        {
            return;
        }

        WANT_ACTIVE.store(active_and_visible, Ordering::SeqCst);
        if JOB_QUEUED.swap(true, Ordering::SeqCst) {
            return;
        }

        let app_main = app.clone();
        let queued = app.run_on_main_thread(move || {
            JOB_QUEUED.store(false, Ordering::SeqCst);
            let active = WANT_ACTIVE.load(Ordering::SeqCst);
            if let Some(win) = app_main.get_webview_window("main") {
                if let Ok(hwnd) = win.hwnd() {
                    win_surface::sync_surface_with_main(hwnd.0 as isize, active);
                }
            }
        });
        if queued.is_err() {
            JOB_QUEUED.store(false, Ordering::SeqCst);
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (app, active_and_visible);
    }
}
