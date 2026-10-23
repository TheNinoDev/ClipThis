#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod capture;
mod codec;
mod hotkey;
mod toast;

use capture::{CaptureManager, Settings};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::sync::mpsc;
use tauri::{
    Manager, PhysicalPosition, State, WebviewUrl, WebviewWindowBuilder, WindowEvent,
    tray::{TrayIconBuilder, MouseButton, MouseButtonState, TrayIconEvent},
    menu::{Menu, MenuItem},
};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_notification::NotificationExt;

#[cfg(windows)]
fn ensure_start_menu_shortcut() {
    use windows::Win32::System::Com::{CoInitializeEx, CoCreateInstance, CoTaskMemAlloc, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, IPersistFile};
    use std::os::windows::ffi::OsStrExt;
    use windows::core::{GUID, PCWSTR, Interface};
    use windows::Win32::UI::Shell::IShellLinkW;
    use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
    use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let clsid: GUID = GUID::from_u128(0x00021401_0000_0000_c000_000000000046);
        let link: IShellLinkW = match CoCreateInstance(&clsid, None, CLSCTX_INPROC_SERVER) {
            Ok(l) => l,
            Err(_) => { windows::Win32::System::Com::CoUninitialize(); return; }
        };
        let exe = match std::env::current_exe() { Ok(e) => e, Err(_) => { windows::Win32::System::Com::CoUninitialize(); return; } };
        let exe_w: Vec<u16> = exe.as_os_str().encode_wide().chain(Some(0)).collect();
        let _ = link.SetPath(PCWSTR(exe_w.as_ptr()));
        if let Some(parent) = exe.parent() {
            let dir_w: Vec<u16> = parent.as_os_str().encode_wide().chain(Some(0)).collect();
            let _ = link.SetWorkingDirectory(PCWSTR(dir_w.as_ptr()));
        }
        let _ = link.SetIconLocation(PCWSTR(exe_w.as_ptr()), 0);

        if let Ok(store) = link.cast::<IPropertyStore>() {
            let aumid = "com.clipthis.app\0";
            let aumid_w: Vec<u16> = aumid.encode_utf16().collect();
            let size = aumid_w.len() * 2;
            let mem = CoTaskMemAlloc(size);
            if !mem.is_null() {
                std::ptr::copy_nonoverlapping(aumid_w.as_ptr() as *const u8, mem as *mut u8, size);
                let mut pv = windows::core::PROPVARIANT::new();
                let raw = (&raw mut pv) as *mut u8;
                *(raw as *mut u16) = 31; // VT_LPWSTR
                *(raw.add(8) as *mut *mut u16) = mem as *mut u16;
                let _ = store.SetValue(&PKEY_AppUserModel_ID, &pv);
                let _ = store.Commit();
            }
        }

        if let Ok(persist) = link.cast::<IPersistFile>() {
            if let Some(data_dir) = dirs::data_dir() {
                let path = data_dir.join("Microsoft").join("Windows").join("Start Menu").join("Programs").join("ClipThis.lnk");
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let path_w: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
                let _ = persist.Save(PCWSTR(path_w.as_ptr()), true);
            }
        }
        windows::Win32::System::Com::CoUninitialize();
    }
}

/// Register the AppUserModelID in HKCU so Windows can render toasts for this app.
#[cfg(windows)]
fn register_aumid() {
    use windows::Win32::System::Registry::*;
    use windows::core::PCWSTR;
    unsafe {
        let subkey: Vec<u16> = "Software\\Classes\\AppUserModelId\\com.clipthis.app\0".encode_utf16().collect();
        let mut hkey = HKEY::default();
        if RegCreateKeyExW(HKEY_CURRENT_USER, PCWSTR(subkey.as_ptr()), 0, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut hkey, None).is_err() {
            return;
        }
        let set = |name: &str, value: &str| {
            let n: Vec<u16> = format!("{}\0", name).encode_utf16().collect();
            let v: Vec<u16> = format!("{}\0", value).encode_utf16().collect();
            let bytes: Vec<u8> = v.iter().flat_map(|c| c.to_le_bytes()).collect();
            let _ = RegSetValueExW(hkey, PCWSTR(n.as_ptr()), 0, REG_SZ, Some(&bytes));
        };
        set("DisplayName", "ClipThis");
        set("IconUri", "");
        let _ = RegCloseKey(hkey);
    }
}

#[derive(Clone, Serialize)]
struct ProcessInfoDto {
    pid: u32,
    name: String,
}

static APP_HANDLE: Mutex<Option<tauri::AppHandle>> = Mutex::new(None);

struct AppState {
    settings: Mutex<Settings>,
    shared_settings: Arc<Mutex<Settings>>,
    manager: Arc<CaptureManager>,
    hotkey_tx: mpsc::Sender<String>,
    toast: toast::ToastService,
}

#[tauri::command]
fn get_settings(state: State<AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
fn set_settings(state: State<AppState>, settings: Settings) -> Result<(), String> {
    capture::set_autostart(settings.autostart)?;
    state.manager.set_clip_seconds(settings.clip_seconds);
    if settings.capture_mic {
        state.manager.start_mic(settings.clip_seconds);
    } else {
        state.manager.stop_mic();
    }
    let _ = state.hotkey_tx.send(settings.hotkey.clone());
    *state.settings.lock().unwrap() = settings.clone();
    *state.shared_settings.lock().unwrap() = settings.clone();
    save_settings(&settings);
    Ok(())
}

#[tauri::command]
fn trigger_clip(state: State<AppState>) -> Result<String, String> {
    let settings = state.settings.lock().unwrap().clone();
    let folder = state.manager.clip_all(&settings)?;
    if settings.notify_clip {
        if let Some(app) = APP_HANDLE.lock().unwrap().clone() {
            let body = format!("Clip saved to {}", folder);
            let app2 = app.clone();
            let _ = app.run_on_main_thread(move || {
                let _ = app2.notification().builder().title("ClipThis").body(body).show();
            });
        }
    }
    Ok(folder)
}

#[tauri::command]
fn list_processes(state: State<AppState>) -> Vec<ProcessInfoDto> {
    let map = state.manager.buffers.lock().unwrap();
    map.values().map(|e| ProcessInfoDto { pid: e.pid, name: e.name.clone() }).collect()
}

#[tauri::command]
fn hide_to_tray(window: tauri::Window, state: State<AppState>) {
    hide_and_release(&window);
    state.toast.show("ClipThis hidden to tray");
}

#[tauri::command]
fn get_version(app: tauri::AppHandle) -> String {
    app.package_info().version.to_string()
}

#[derive(serde::Serialize)]
struct UpdateInfo {
    current: String,
    latest: String,
    url: String,
    notes: String,
}

#[tauri::command]
fn check_update(app: tauri::AppHandle) -> Result<Option<UpdateInfo>, String> {
    let current = app.package_info().version.to_string();
    let resp = ureq::get("https://api.github.com/repos/TheNinoDev/ClipThis/releases/latest")
        .set("User-Agent", "ClipThis")
        .call()
        .map_err(|e| e.to_string())?;
    let json: serde_json::Value = resp.into_json::<serde_json::Value>().map_err(|e| e.to_string())?;
    let tag = json["tag_name"].as_str().unwrap_or("").trim_start_matches('v').to_string();
    let url = json["html_url"].as_str().unwrap_or("").to_string();
    let notes = json["body"].as_str().unwrap_or("").to_string();
    let assets = json["assets"].as_array();
    let download_url = assets
        .and_then(|a| a.iter().find(|x| x["name"].as_str().map(|n| n.ends_with(".exe")).unwrap_or(false)))
        .and_then(|x| x["browser_download_url"].as_str())
        .unwrap_or(&url)
        .to_string();
    if tag == current || tag.is_empty() {
        Ok(None)
    } else {
        Ok(Some(UpdateInfo {
            current,
            latest: tag,
            url: download_url,
            notes,
        }))
    }
}

#[tauri::command]
fn apply_update(url: String) -> Result<(), String> {
    let temp = std::env::temp_dir().join("ClipThis_update.exe");
    let resp = ureq::get(&url).call().map_err(|e| e.to_string())?;
    let mut reader = resp.into_reader();
    let mut file = std::fs::File::create(&temp).map_err(|e| e.to_string())?;
    std::io::copy(&mut reader, &mut file).map_err(|e| e.to_string())?;
    // write an updater bat that swaps the exe after we exit
    let current_exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let bat = std::env::temp_dir().join("clipthis_updater.bat");
    let content = format!(
        "@echo off\r\nping -n 5 127.0.0.1 >nul\r\ntaskkill /f /im clipthis.exe >nul 2>&1\r\nping -n 2 127.0.0.1 >nul\r\ncopy /Y \"{}\" \"{}\" >nul\r\nstart \"\" \"{}\"\r\ndel \"{}\"\r\ndel \"%~f0\"\r\n",
        temp.display(),
        current_exe.display(),
        current_exe.display(),
        temp.display()
    );
    std::fs::write(&bat, content).map_err(|e| e.to_string())?;
    std::process::Command::new("cmd")
        .args(["/C", "start", "", bat.to_str().unwrap()])
        .spawn()
        .map_err(|e| e.to_string())?;
    std::process::exit(0);
}

#[tauri::command]
fn pick_folder(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let folder = app.dialog().file().blocking_pick_folder();
    Ok(folder.map(|p| p.to_string()))
}

fn config_path() -> std::path::PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    base.join("ClipThis").join("settings.json")
}

fn load_settings() -> Settings {
    let p = config_path();
    if let Ok(data) = std::fs::read_to_string(&p) {
        if let Ok(s) = serde_json::from_str::<Settings>(&data) {
            return s;
        }
    }
    Settings::default()
}

fn save_settings(s: &Settings) {
    let p = config_path();
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(data) = serde_json::to_string_pretty(s) {
        let _ = std::fs::write(&p, data);
    }
}

// ---------------------------------------------------------------------------
// Lazy UI: the webview only exists while the window is on screen.
// Hiding destroys it so the whole WebView2 process tree (60-90 MB) is freed.
// ---------------------------------------------------------------------------

/// Set while the UI is supposed to be visible. Guards against destroying a
/// window that the tray re-opened in the same instant the hide timer fired.
static WANT_VISIBLE: AtomicBool = AtomicBool::new(false);
/// Set while a forced destroy is in flight so CloseRequested is not swallowed.
static DESTROYING: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Serialize, Deserialize, Default)]
struct WindowPos {
    x: i32,
    y: i32,
}

fn window_pos_path() -> std::path::PathBuf {
    config_path().with_file_name("window.json")
}

fn load_window_pos() -> Option<WindowPos> {
    let data = std::fs::read_to_string(window_pos_path()).ok()?;
    let p: WindowPos = serde_json::from_str(&data).ok()?;
    if p.x.abs() > 20000 || p.y.abs() > 20000 {
        return None;
    }
    Some(p)
}

fn remember_position(window: &tauri::Window) {
    if let Ok(p) = window.outer_position() {
        if let Some(parent) = window_pos_path().parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(data) = serde_json::to_string(&WindowPos { x: p.x, y: p.y }) {
            let _ = std::fs::write(window_pos_path(), data);
        }
    }
}

/// Creates the webview window if it does not exist, otherwise reveals it.
fn open_window(app: &tauri::AppHandle) {
    WANT_VISIBLE.store(true, Ordering::SeqCst);
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let built = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("ClipThis")
        .inner_size(560.0, 540.0)
        .resizable(false)
        .decorations(false)
        .transparent(false)
        .shadow(true)
        .visible(false)
        .build();
    match built {
        Ok(w) => {
            if let Ok(icon) = tauri::image::Image::from_bytes(include_bytes!("../icons/128x128@2x.png")) {
                let _ = w.set_icon(icon);
            }
            match load_window_pos() {
                Some(p) => { let _ = w.set_position(PhysicalPosition::new(p.x, p.y)); }
                None => { let _ = w.center(); }
            }
            let _ = w.show();
            let _ = w.set_focus();
        }
        Err(e) => eprintln!("failed to create window: {e}"),
    }
}

/// Hides the window immediately, then tears the webview down off the IPC
/// handler so the destroy cannot deadlock the command that requested it.
fn hide_and_release(window: &tauri::Window) {
    WANT_VISIBLE.store(false, Ordering::SeqCst);
    remember_position(window);
    let app = window.app_handle().clone();
    let _ = app
        .notification()
        .builder()
        .title("ClipThis")
        .body("Running in tray. Click the tray icon to reopen.")
        .show();
    let _ = window.hide();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(80));
        if WANT_VISIBLE.load(Ordering::SeqCst) {
            return;
        }
        let app2 = app.clone();
        let _ = app.run_on_main_thread(move || {
            if WANT_VISIBLE.load(Ordering::SeqCst) {
                return;
            }
            if let Some(w) = app2.get_webview_window("main") {
                DESTROYING.store(true, Ordering::SeqCst);
                let _ = w.destroy();
                DESTROYING.store(false, Ordering::SeqCst);
            }
        });
    });
}

/// Blocks on a named event: a second instance signals it instead of hunting for
/// a window title, which no longer works while the UI is destroyed.
#[cfg(windows)]
fn spawn_show_signal_listener(app: tauri::AppHandle) {
    use windows::Win32::Foundation::WAIT_OBJECT_0;
    use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject, INFINITE};
    use windows::core::PCWSTR;
    std::thread::spawn(move || {
        let name: Vec<u16> = "Local\\ClipThisShowEvent\0".encode_utf16().collect();
        let ev = match unsafe { CreateEventW(None, false, false, PCWSTR(name.as_ptr())) } {
            Ok(h) => h,
            Err(_) => return,
        };
        loop {
            let r = unsafe { WaitForSingleObject(ev, INFINITE) };
            if r == WAIT_OBJECT_0 {
                let a = app.clone();
                let _ = app.run_on_main_thread(move || open_window(&a));
            } else {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
    });
}

fn main() {
    // single instance
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::{CloseHandle, GetLastError};
        use windows::Win32::System::Threading::{
            CreateMutexW, OpenEventW, SetEvent, EVENT_MODIFY_STATE,
        };
        use windows::core::PCWSTR;
        unsafe {
            let name: Vec<u16> = "ClipThisSingleInstance\0".encode_utf16().collect();
            let _mutex = CreateMutexW(None, true, PCWSTR(name.as_ptr()));
            if GetLastError().0 == 183 {
                // Another instance owns the mutex. Ask it to show the UI through a
                // named event - the window may not exist right now, so there is no
                // HWND to find or restore.
                let ev_name: Vec<u16> = "Local\\ClipThisShowEvent\0".encode_utf16().collect();
                for _ in 0..40 {
                    if let Ok(ev) = OpenEventW(EVENT_MODIFY_STATE, false, PCWSTR(ev_name.as_ptr())) {
                        let _ = SetEvent(ev);
                        let _ = CloseHandle(ev);
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                return;
            }
        }
    }
    let settings = load_settings();
    let manager = CaptureManager::new(settings.clip_seconds);
    if settings.capture_mic {
        manager.start_mic(settings.clip_seconds);
    }
    let (clip_tx, clip_rx) = mpsc::channel::<()>();

    let shared_settings = Arc::new(Mutex::new(settings.clone()));

    let toast = toast::ToastService::start();

    // clip worker thread
    let shared_settings2 = shared_settings.clone();
    let manager3 = manager.clone();
    let toast2 = toast;
    std::thread::spawn(move || {
        while clip_rx.recv().is_ok() {
            let s = shared_settings2.lock().unwrap().clone();
            if let Ok(folder) = manager3.clip_all(&s) {
                if s.notify_clip {
                    toast2.show(&format!("Clip saved to {}", folder));
                }
            } else {
                let _ = std::fs::write("D:\\ClipThis\\clipnotify.log", "clip failed");
            }
        }
    });

    // hotkey thread with dynamic re-registration
    let (hotkey_tx, hotkey_rx) = mpsc::channel::<String>();
    let clip_tx2 = clip_tx.clone();
    let initial_hotkey = settings.hotkey.clone();
    std::thread::spawn(move || {
        use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
        let manager = match GlobalHotKeyManager::new() {
            Ok(m) => m,
            Err(e) => { eprintln!("hotkey manager: {}", e); return; }
        };
        let mut current: Option<global_hotkey::hotkey::HotKey> = None;
        // register initial
        if let Some(hk) = hotkey::parse(&initial_hotkey) {
            let res = manager.register(hk);
            if res.is_ok() {
                current = Some(hk);
            }
        }
        let event_rx = GlobalHotKeyEvent::receiver();
        loop {
            // handle hotkey changes
            if let Ok(new_str) = hotkey_rx.try_recv() {
                if let Some(hk) = current.take() {
                    let _ = manager.unregister(hk);
                }
                if let Some(hk) = hotkey::parse(&new_str) {
                    if manager.register(hk).is_ok() {
                        current = Some(hk);
                    }
                }
            }
            // pump Windows messages so WM_HOTKEY is dispatched
            unsafe {
                use windows::Win32::UI::WindowsAndMessaging::*;
                let mut msg = MSG::default();
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            // handle events
            if let Ok(event) = event_rx.try_recv() {
                if event.state == HotKeyState::Pressed {
                    let _ = clip_tx2.send(());
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    });

    let toast = toast.clone();
    let app_state = AppState {
        settings: Mutex::new(settings),
        shared_settings,
        manager,
        hotkey_tx,
        toast,
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .manage(app_state)
        .setup(|app| {
            *APP_HANDLE.lock().unwrap() = Some(app.handle().clone());
            #[cfg(windows)]
            {
                use windows::core::PCWSTR;
                use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
                let aumid: Vec<u16> = "com.clipthis.app\0".encode_utf16().collect();
                let _ = unsafe { SetCurrentProcessExplicitAppUserModelID(PCWSTR(aumid.as_ptr())) };
                register_aumid();
                ensure_start_menu_shortcut();
            }
            // tray icon::
            let show_item = MenuItem::with_id(app, "show", "Show ClipThis", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_item, &quit_item])?;
            let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/32x32.png")).expect("icon");
            let _tray = TrayIconBuilder::new()
                .icon(icon)
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| {
                    match event.id.as_ref() {
                        "show" => open_window(app),
                        "quit" => {
                            app.exit(0);
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                        open_window(tray.app_handle());
                    }
                })
                .build(app)?;
            spawn_show_signal_listener(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if DESTROYING.load(Ordering::SeqCst) {
                    return;
                }
                api.prevent_close();
                hide_and_release(window);
            }
        })
        .invoke_handler(tauri::generate_handler![get_settings, set_settings, trigger_clip, list_processes, pick_folder, hide_to_tray, get_version, check_update, apply_update])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, event| {
            // With a lazy window the app legitimately has zero windows most of
            // the time; never let that be treated as "quit".
            if let tauri::RunEvent::ExitRequested { code, api, .. } = event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
