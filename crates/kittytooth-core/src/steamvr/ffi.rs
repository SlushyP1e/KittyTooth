//! Minimal OpenVR client, loaded at runtime from the user's SteamVR install.
//!
//! This does what `openvr_api.dll` does, without needing the OpenVR SDK at build time:
//! read `openvrpaths.vrpath` to find the runtime, load `vrclient`, ask its
//! `VRClientCoreFactory` for `IVRClientCore_003`, then fetch the plain C function tables
//! (`FnTable:IVRSystem_026`, `FnTable:IVROverlay_028`). Layouts and indices are taken from
//! OpenVR 2.15 (`openvr_capi.h`, `src/ivrclientcore.h`); only the entries we call are named,
//! the rest are padding. Everything here is only ever touched from the SteamVR thread.

use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, TryLockError};

use libloading::Library;

pub type TrackedDeviceIndex = u32;
pub type OverlayHandle = u64;

pub const TRACKED_DEVICE_HMD: TrackedDeviceIndex = 0;
pub const TRACKED_DEVICE_INVALID: TrackedDeviceIndex = 0xFFFF_FFFF;

pub const APP_OVERLAY: i32 = 2;
pub const APP_BACKGROUND: i32 = 3;

pub const INIT_OK: i32 = 0;
/// `VRInitError_Init_NoServerForBackgroundApp`: SteamVR is installed but not running.
pub const INIT_NO_SERVER_FOR_BACKGROUND_APP: i32 = 121;

pub const ROLE_LEFT_HAND: i32 = 1;
pub const ROLE_RIGHT_HAND: i32 = 2;

pub const EVENT_QUIT: u32 = 700;
pub const OVERLAY_ERROR_KEY_IN_USE: i32 = 17;

const CLIENT_CORE_VERSION: &CStr = c"IVRClientCore_003";
const SYSTEM_VERSION: &CStr = c"IVRSystem_026";
const OVERLAY_VERSION: &CStr = c"IVROverlay_028";
const SYSTEM_FNTABLE: &CStr = c"FnTable:IVRSystem_026";
const OVERLAY_FNTABLE: &CStr = c"FnTable:IVROverlay_028";
/// SDK version these layouts come from (reported to SteamVR like `VR_Init` does).
const SDK_VERSION: (u32, u32, u32) = (2, 15, 6);

/// Row-major 3x4 transform (`HmdMatrix34_t`).
pub type Matrix34 = [[f32; 4]; 3];

/// `IVRClientCore` is a C++ interface. On 64-bit Windows and on the Itanium ABI (Linux) a
/// virtual call is a plain C call with `this` first, and this interface has no virtual
/// destructor, so the slots start at 0.
#[repr(C)]
struct ClientCore {
    vtbl: *const ClientCoreVtbl,
}

#[repr(C)]
struct ClientCoreVtbl {
    init: unsafe extern "C" fn(*mut ClientCore, i32, *const c_char) -> i32,
    cleanup: unsafe extern "C" fn(*mut ClientCore),
    is_interface_version_valid: unsafe extern "C" fn(*mut ClientCore, *const c_char) -> i32,
    get_generic_interface: unsafe extern "C" fn(*mut ClientCore, *const c_char, *mut i32) -> *mut c_void,
    b_is_hmd_present: unsafe extern "C" fn(*mut ClientCore) -> bool,
    get_english_string_for_hmd_error: unsafe extern "C" fn(*mut ClientCore, i32) -> *const c_char,
    get_id_for_vr_init_error: unsafe extern "C" fn(*mut ClientCore, i32) -> *const c_char,
}

type Pad = *const c_void;

/// `VR_IVRSystem_FnTable` (IVRSystem_026), entries 0..=50.
#[repr(C)]
struct SystemFn {
    _p0: [Pad; 12],
    get_device_to_absolute_tracking_pose: unsafe extern "C" fn(i32, f32, *mut TrackedDevicePose, u32), // 12
    _p0b: [Pad; 5],
    get_tracked_device_index_for_controller_role: unsafe extern "C" fn(i32) -> TrackedDeviceIndex, // 18
    _p1: [Pad; 11],
    poll_next_event: unsafe extern "C" fn(*mut VrEvent, u32) -> bool, // 30
    _p2: [Pad; 8],
    trigger_haptic_pulse: unsafe extern "C" fn(TrackedDeviceIndex, u32, u16), // 39
    _p3: [Pad; 7],
    acknowledge_quit_exiting: unsafe extern "C" fn(), // 47
    _p4: [Pad; 1],
    get_runtime_version: unsafe extern "C" fn() -> *const c_char, // 49
    set_sdk_version: unsafe extern "C" fn(u32, u32, u32) -> i32,  // 50
}

/// `VR_IVROverlay_FnTable` (IVROverlay_028), entries 0..=62.
#[repr(C)]
struct OverlayFn {
    find_overlay: unsafe extern "C" fn(*const c_char, *mut OverlayHandle) -> i32, // 0
    create_overlay: unsafe extern "C" fn(*const c_char, *const c_char, *mut OverlayHandle) -> i32, // 1
    _p0: [Pad; 1],
    destroy_overlay: unsafe extern "C" fn(OverlayHandle) -> i32, // 3
    _p1: [Pad; 12],
    set_overlay_alpha: unsafe extern "C" fn(OverlayHandle, f32) -> i32, // 16
    _p2: [Pad; 3],
    set_overlay_sort_order: unsafe extern "C" fn(OverlayHandle, u32) -> i32, // 20
    _p3: [Pad; 1],
    set_overlay_width_in_meters: unsafe extern "C" fn(OverlayHandle, f32) -> i32, // 22
    _p4: [Pad; 12],
    set_overlay_transform_tracked_device_relative:
        unsafe extern "C" fn(OverlayHandle, TrackedDeviceIndex, *const Matrix34) -> i32, // 35
    _p5: [Pad; 7],
    show_overlay: unsafe extern "C" fn(OverlayHandle) -> i32, // 43
    hide_overlay: unsafe extern "C" fn(OverlayHandle) -> i32, // 44
    _p6: [Pad; 17],
    set_overlay_raw: unsafe extern "C" fn(OverlayHandle, *mut c_void, u32, u32, u32) -> i32, // 62
}

/// `VREvent_t`. The data union is 48 bytes (six u64). On Windows it is 8-aligned (64 bytes
/// total); on Linux the struct is packed to 4 for historical reasons (60 bytes).
#[cfg_attr(windows, repr(C))]
#[cfg_attr(not(windows), repr(C, packed(4)))]
#[derive(Clone, Copy)]
pub struct VrEvent {
    pub event_type: u32,
    pub tracked_device_index: TrackedDeviceIndex,
    pub event_age_seconds: f32,
    data: [u64; 6],
}

#[cfg(windows)]
const _: () = assert!(std::mem::size_of::<VrEvent>() == 64);
#[cfg(not(windows))]
const _: () = assert!(std::mem::size_of::<VrEvent>() == 60);

/// `TrackedDevicePose_t` (natural alignment, 80 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TrackedDevicePose {
    pub device_to_absolute: Matrix34,
    pub velocity: [f32; 3],
    pub angular_velocity: [f32; 3],
    pub tracking_result: i32,
    pub pose_is_valid: bool,
    pub device_is_connected: bool,
}

const _: () = assert!(std::mem::size_of::<TrackedDevicePose>() == 80);

/// `TrackingUniverseStanding`.
pub const UNIVERSE_STANDING: i32 = 1;

/// Why connecting failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectError {
    /// No OpenVR runtime registered on this PC.
    NotInstalled,
    /// Installed, but SteamVR is not running.
    NotRunning,
    /// Anything else, in plain words.
    Failed(String),
}

/// Default location of `openvrpaths.vrpath` (honours `VR_PATHREG_OVERRIDE`).
pub fn vrpath_file() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("VR_PATHREG_OVERRIDE").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "linux") {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    } else {
        None // SteamVR no longer supports macOS
    }?;
    Some(base.join("openvr").join("openvrpaths.vrpath"))
}

/// Runtime directories listed in an `openvrpaths.vrpath` file.
pub fn parse_runtime_paths(text: &str) -> Vec<PathBuf> {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| v.get("runtime").and_then(|r| r.as_array()).cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|p| p.as_str())
        .filter(|p| !p.trim().is_empty())
        .map(PathBuf::from)
        .collect()
}

/// Where `vrclient` lives inside a runtime directory, most likely first.
pub fn client_candidates(runtime: &Path) -> Vec<PathBuf> {
    let bin = runtime.join("bin");
    if cfg!(windows) {
        vec![
            bin.join("vrclient_x64.dll"),
            bin.join("win64").join("vrclient.dll"),
        ]
    } else {
        vec![bin.join("linux64").join("vrclient.so")]
    }
}

/// Finds `vrclient` the way `openvr_api` does (`VR_OVERRIDE`, then the path registry).
/// `vrpath` is the registry file to read (`None` means there is none).
pub fn find_client(vrpath: Option<&Path>) -> Result<PathBuf, ConnectError> {
    if !cfg!(target_pointer_width = "64") {
        return Err(ConnectError::NotInstalled);
    }
    let runtimes = match std::env::var_os("VR_OVERRIDE").filter(|p| !p.is_empty()) {
        Some(p) => vec![PathBuf::from(p)],
        None => {
            let Some(text) = vrpath.and_then(|p| std::fs::read_to_string(p).ok()) else {
                return Err(ConnectError::NotInstalled);
            };
            parse_runtime_paths(&text)
        }
    };
    runtimes
        .iter()
        .flat_map(|r| client_candidates(r))
        .find(|p| p.is_file())
        .ok_or(ConnectError::NotInstalled)
}

/// A loaded `vrclient` with an initialised client core.
struct Runtime {
    core: *mut ClientCore,
    /// `Init` succeeded, so `Cleanup` must run before unloading.
    initialised: bool,
    // Dropped (unloaded) after `core` has been cleaned up in `Drop for Runtime`.
    _lib: Library,
    // Released last, once the library is unloaded.
    _guard: MutexGuard<'static, ()>,
}

/// vrclient keeps process-wide state: loading, initialising or unloading it from two threads at
/// once corrupts the heap (openvr_api has a global mutex for the same reason). Only one
/// `Runtime` may exist at a time; the SteamVR thread is the only user in the app.
static VRCLIENT: Mutex<()> = Mutex::new(());

impl Runtime {
    fn load(path: &Path) -> Result<Self, ConnectError> {
        let guard = match VRCLIENT.try_lock() {
            Ok(g) => g,
            Err(TryLockError::Poisoned(p)) => p.into_inner(),
            Err(TryLockError::WouldBlock) => {
                return Err(ConnectError::Failed(
                    "SteamVR is already in use by KittyTooth".into(),
                ))
            }
        };
        let lib = load_library(path)
            .map_err(|e| ConnectError::Failed(format!("could not load {}: {e}", path.display())))?;
        type Factory = unsafe extern "C" fn(*const c_char, *mut i32) -> *mut c_void;
        // SAFETY: `VRClientCoreFactory` has had this signature since the first OpenVR release.
        let core = unsafe {
            let factory = lib
                .get::<Factory>(b"VRClientCoreFactory\0")
                .map_err(|_| ConnectError::Failed("vrclient has no VRClientCoreFactory".into()))?;
            let mut rc = 0i32;
            factory(CLIENT_CORE_VERSION.as_ptr(), &mut rc) as *mut ClientCore
        };
        if core.is_null() {
            return Err(ConnectError::Failed(
                "this SteamVR version does not offer IVRClientCore_003".into(),
            ));
        }
        Ok(Self {
            core,
            initialised: false,
            _lib: lib,
            _guard: guard,
        })
    }

    fn vtbl(&self) -> &ClientCoreVtbl {
        // SAFETY: `core` is a live C++ object returned by the factory.
        unsafe { &*(*self.core).vtbl }
    }

    fn init(&mut self, app_type: i32) -> i32 {
        // SAFETY: valid object; a null startup-info string is allowed.
        let rc = unsafe { (self.vtbl().init)(self.core, app_type, std::ptr::null()) };
        self.initialised = rc == INIT_OK;
        rc
    }

    fn version_valid(&self, version: &CStr) -> bool {
        // SAFETY: valid object and NUL-terminated string.
        unsafe { (self.vtbl().is_interface_version_valid)(self.core, version.as_ptr()) == INIT_OK }
    }

    fn interface<T>(&self, name: &CStr) -> Result<*const T, i32> {
        let mut err = 0i32;
        // SAFETY: valid object and NUL-terminated string.
        let p = unsafe { (self.vtbl().get_generic_interface)(self.core, name.as_ptr(), &mut err) };
        if p.is_null() || err != INIT_OK {
            Err(err)
        } else {
            Ok(p as *const T)
        }
    }

    fn error_text(&self, err: i32) -> String {
        // SAFETY: valid object; the returned string is static inside vrclient.
        let p = unsafe { (self.vtbl().get_english_string_for_hmd_error)(self.core, err) };
        if p.is_null() {
            format!("SteamVR error {err}")
        } else {
            // SAFETY: NUL-terminated string owned by vrclient.
            let s = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
            format!("{s} ({err})")
        }
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        if self.initialised {
            // SAFETY: valid object; Cleanup is what VR_Shutdown calls before unloading.
            unsafe { (self.vtbl().cleanup)(self.core) }
        }
    }
}

#[cfg(windows)]
fn load_library(path: &Path) -> Result<Library, libloading::Error> {
    use libloading::os::windows::{Library as WinLib, LOAD_WITH_ALTERED_SEARCH_PATH};
    // SAFETY: loading SteamVR's own client library, as openvr_api does (same flag, so its
    // dependencies resolve from its own folder).
    unsafe { WinLib::load_with_flags(path, LOAD_WITH_ALTERED_SEARCH_PATH).map(Library::from) }
}

#[cfg(not(windows))]
fn load_library(path: &Path) -> Result<Library, libloading::Error> {
    // SAFETY: loading SteamVR's own client library, as openvr_api does.
    unsafe { Library::new(path) }
}

/// Checks whether SteamVR is running without starting it: a background app's init fails with
/// `NoServerForBackgroundApp` instead of launching SteamVR.
pub fn probe(client: &Path) -> Result<(), ConnectError> {
    let mut rt = Runtime::load(client)?;
    match rt.init(APP_BACKGROUND) {
        INIT_OK => Ok(()), // dropping `rt` cleans up and unloads
        INIT_NO_SERVER_FOR_BACKGROUND_APP => Err(ConnectError::NotRunning),
        e => Err(ConnectError::Failed(rt.error_text(e))),
    }
}

/// A live connection to SteamVR as an overlay application.
pub struct Session {
    system: *const SystemFn,
    overlay: *const OverlayFn,
    pub runtime_version: Option<String>,
    // Last: dropped after everything that uses it.
    _rt: Runtime,
}

impl Session {
    /// Connects only if SteamVR is already running (never launches it).
    pub fn connect(client: &Path) -> Result<Self, ConnectError> {
        probe(client)?;
        let mut rt = Runtime::load(client)?;
        match rt.init(APP_OVERLAY) {
            INIT_OK => {}
            e => return Err(ConnectError::Failed(rt.error_text(e))),
        }
        for v in [SYSTEM_VERSION, OVERLAY_VERSION] {
            if !rt.version_valid(v) {
                return Err(ConnectError::Failed(format!(
                    "this SteamVR is too old for {} (update SteamVR)",
                    v.to_string_lossy()
                )));
            }
        }
        let system = rt
            .interface::<SystemFn>(SYSTEM_FNTABLE)
            .map_err(|e| ConnectError::Failed(rt.error_text(e)))?;
        let overlay = rt
            .interface::<OverlayFn>(OVERLAY_FNTABLE)
            .map_err(|e| ConnectError::Failed(rt.error_text(e)))?;
        let mut s = Self {
            system,
            overlay,
            runtime_version: None,
            _rt: rt,
        };
        let (a, b, c) = SDK_VERSION;
        // SAFETY: function tables returned by vrclient stay valid until Cleanup.
        unsafe {
            let _ = (s.sys().set_sdk_version)(a, b, c);
            let v = (s.sys().get_runtime_version)();
            if !v.is_null() {
                s.runtime_version = Some(CStr::from_ptr(v).to_string_lossy().into_owned());
            }
        }
        Ok(s)
    }

    fn sys(&self) -> &SystemFn {
        // SAFETY: valid until the runtime is dropped, which only happens with `self`.
        unsafe { &*self.system }
    }

    fn ovl(&self) -> &OverlayFn {
        // SAFETY: as above.
        unsafe { &*self.overlay }
    }

    /// Drains pending events. Returns true when SteamVR asked us to quit (it is shutting down);
    /// the quit is acknowledged and the caller should drop the session.
    pub fn poll_quit(&self) -> bool {
        // SAFETY: plain-old-data out parameter of the size SteamVR expects for this SDK.
        let mut ev: VrEvent = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<VrEvent>() as u32;
        for _ in 0..64 {
            // SAFETY: valid table and out pointer.
            if !unsafe { (self.sys().poll_next_event)(&mut ev, size) } {
                break;
            }
            let kind = ev.event_type;
            if kind == EVENT_QUIT {
                // SAFETY: valid table.
                unsafe { (self.sys().acknowledge_quit_exiting)() };
                return true;
            }
        }
        false
    }

    /// Current poses of devices `0..count` (standing universe). Invalid poses have
    /// `pose_is_valid == false`.
    pub fn poses(&self, count: u32) -> Vec<TrackedDevicePose> {
        let count = count.clamp(1, 64);
        // SAFETY: plain-old-data; SteamVR fills `count` entries.
        let mut out = vec![unsafe { std::mem::zeroed::<TrackedDevicePose>() }; count as usize];
        // SAFETY: valid table and a buffer of `count` poses.
        unsafe {
            (self.sys().get_device_to_absolute_tracking_pose)(UNIVERSE_STANDING, 0.0, out.as_mut_ptr(), count)
        };
        out
    }

    /// Device index of the left (`ROLE_LEFT_HAND`) or right controller, if one is connected.
    pub fn controller(&self, role: i32) -> Option<TrackedDeviceIndex> {
        // SAFETY: valid table.
        let i = unsafe { (self.sys().get_tracked_device_index_for_controller_role)(role) };
        (i != TRACKED_DEVICE_INVALID).then_some(i)
    }

    /// One legacy haptic pulse (max 3999 µs; SteamVR accepts one per controller every 5 ms).
    pub fn haptic_pulse(&self, device: TrackedDeviceIndex, micros: u16) {
        // SAFETY: valid table; axis 0 is the main haptic actuator.
        unsafe { (self.sys().trigger_haptic_pulse)(device, 0, micros.min(3999)) }
    }

    pub fn create_overlay(&self, key: &str, name: &str) -> Result<OverlayHandle, String> {
        let key = CString::new(key).map_err(|e| e.to_string())?;
        let name = CString::new(name).map_err(|e| e.to_string())?;
        let mut h: OverlayHandle = 0;
        // SAFETY: valid table and NUL-terminated strings.
        let mut rc = unsafe { (self.ovl().create_overlay)(key.as_ptr(), name.as_ptr(), &mut h) };
        if rc == OVERLAY_ERROR_KEY_IN_USE {
            // SAFETY: as above.
            rc = unsafe { (self.ovl().find_overlay)(key.as_ptr(), &mut h) };
        }
        if rc == 0 {
            Ok(h)
        } else {
            Err(format!("SteamVR overlay error {rc}"))
        }
    }

    pub fn destroy_overlay(&self, h: OverlayHandle) {
        // SAFETY: valid table.
        unsafe { (self.ovl().destroy_overlay)(h) };
    }

    pub fn set_alpha(&self, h: OverlayHandle, alpha: f32) {
        // SAFETY: valid table.
        unsafe { (self.ovl().set_overlay_alpha)(h, alpha) };
    }

    pub fn set_sort_order(&self, h: OverlayHandle, order: u32) {
        // SAFETY: valid table.
        unsafe { (self.ovl().set_overlay_sort_order)(h, order) };
    }

    pub fn set_width(&self, h: OverlayHandle, meters: f32) {
        // SAFETY: valid table.
        unsafe { (self.ovl().set_overlay_width_in_meters)(h, meters) };
    }

    pub fn set_transform_relative(&self, h: OverlayHandle, device: TrackedDeviceIndex, m: &Matrix34) {
        // SAFETY: valid table; SteamVR copies the matrix.
        unsafe { (self.ovl().set_overlay_transform_tracked_device_relative)(h, device, m) };
    }

    pub fn show(&self, h: OverlayHandle) {
        // SAFETY: valid table.
        unsafe { (self.ovl().show_overlay)(h) };
    }

    pub fn hide(&self, h: OverlayHandle) {
        // SAFETY: valid table.
        unsafe { (self.ovl().hide_overlay)(h) };
    }

    /// Uploads straight-alpha RGBA pixels. SteamVR copies the buffer before returning.
    pub fn set_raw(&self, h: OverlayHandle, rgba: &mut [u8], width: u32, height: u32) -> Result<(), String> {
        if rgba.len() != (width * height * 4) as usize {
            return Err("bad overlay image size".into());
        }
        // SAFETY: valid table; the buffer is width*height*4 bytes and outlives the call.
        let rc = unsafe { (self.ovl().set_overlay_raw)(h, rgba.as_mut_ptr().cast(), width, height, 4) };
        if rc == 0 {
            Ok(())
        } else {
            Err(format!("SteamVR overlay error {rc}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json_path(p: &Path) -> String {
        serde_json::to_string(&p.display().to_string()).unwrap()
    }

    /// Loads this PC's real SteamVR client and probes it as a background app (which never
    /// launches SteamVR). Run by hand: `cargo test -p kittytooth-core real_probe -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_probe() {
        let client = find_client(vrpath_file().as_deref());
        println!("vrclient: {client:?}");
        if let Ok(c) = client {
            // Loading and unloading repeatedly must stay stable (KittyTooth probes every 5 s).
            for _ in 0..20 {
                assert_eq!(probe(&c), Err(ConnectError::NotRunning));
            }
            let r = probe(&c);
            println!("probe: {r:?}");
            if r.is_ok() {
                let s = Session::connect(&c).expect("connect");
                println!("SteamVR {:?}", s.runtime_version);
                println!(
                    "left {:?} right {:?}",
                    s.controller(ROLE_LEFT_HAND),
                    s.controller(ROLE_RIGHT_HAND)
                );
                println!("quit pending: {}", s.poll_quit());
            }
        }
    }

    #[test]
    fn parses_vrpath() {
        let text = r#"{
            "config": ["C:\\Program Files (x86)\\Steam\\config"],
            "external_drivers": null,
            "jsonid": "vrpathreg",
            "runtime": ["C:\\Steam\\steamapps\\common\\SteamVR", ""],
            "version": 1
        }"#;
        assert_eq!(
            parse_runtime_paths(text),
            vec![PathBuf::from("C:\\Steam\\steamapps\\common\\SteamVR")]
        );
        assert!(parse_runtime_paths("not json").is_empty());
        assert!(parse_runtime_paths(r#"{"runtime": 5}"#).is_empty());
    }

    #[test]
    fn missing_registry_means_not_installed() {
        if std::env::var_os("VR_OVERRIDE").is_some() {
            return;
        }
        assert_eq!(find_client(None), Err(ConnectError::NotInstalled));
        let dir = std::env::temp_dir().join(format!("kt-vrpath-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("openvrpaths.vrpath");
        // A registered runtime folder without vrclient in it is not an install.
        let rt = json_path(&dir.join("SteamVR"));
        std::fs::write(&file, format!(r#"{{"runtime":[{rt}]}}"#)).unwrap();
        assert_eq!(find_client(Some(&file)), Err(ConnectError::NotInstalled));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn finds_client_in_runtime_folder() {
        if std::env::var_os("VR_OVERRIDE").is_some() || !cfg!(target_pointer_width = "64") {
            return;
        }
        let dir = std::env::temp_dir().join(format!("kt-vrpath-{}", uuid::Uuid::new_v4()));
        let runtime = dir.join("SteamVR");
        let client = client_candidates(&runtime).remove(0);
        std::fs::create_dir_all(client.parent().unwrap()).unwrap();
        std::fs::write(&client, b"").unwrap();
        let file = dir.join("openvrpaths.vrpath");
        std::fs::write(&file, format!(r#"{{"runtime":[{}]}}"#, json_path(&runtime))).unwrap();
        assert_eq!(find_client(Some(&file)), Ok(client));
        std::fs::remove_dir_all(dir).ok();
    }
}
