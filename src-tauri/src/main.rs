#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod capture;
mod codec;
mod codec_aac;
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
// use tauri_plugin_notification::NotificationExt;

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

#[derive(serde::Serialize, Clone)]
struct ClipFileDto {
    name: String,
    path: String,
    size: u64,
    duration_ms: u64,
    waveform: Vec<u8>,
}

#[derive(serde::Serialize, Clone)]
struct PlaybackStateDto {
    path: Option<String>,
    position_ms: u64,
    duration_ms: u64,
    playing: bool,
}

struct ActivePlayer {
    path: String,
    duration_ms: u64,
    player: rodio::Player,
}

#[derive(serde::Serialize, Clone)]
struct ClipFolderDto {
    name: String,
    path: String,
    files: Vec<ClipFileDto>,
}

static APP_HANDLE: Mutex<Option<tauri::AppHandle>> = Mutex::new(None);
static TRANSCRIBE_STOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static PLAYER_STREAM: std::sync::OnceLock<std::sync::Mutex<Option<rodio::MixerDeviceSink>>> = std::sync::OnceLock::new();
static CURRENT_PLAYER: std::sync::OnceLock<std::sync::Mutex<Option<ActivePlayer>>> = std::sync::OnceLock::new();
static CLIP_CACHE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, (u64, Vec<u8>)>>> = std::sync::OnceLock::new();

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
fn reset_settings(state: State<AppState>) -> Result<Settings, String> {
    let defaults = Settings::default();
    capture::set_autostart(defaults.autostart)?;
    state.manager.set_clip_seconds(defaults.clip_seconds);
    state.manager.stop_mic();
    let _ = state.hotkey_tx.send(defaults.hotkey.clone());
    *state.settings.lock().unwrap() = defaults.clone();
    *state.shared_settings.lock().unwrap() = defaults.clone();
    save_settings(&defaults);
    Ok(defaults)
}

#[tauri::command]
async fn trigger_clip(state: State<'_, AppState>) -> Result<String, String> {
    let settings = state.settings.lock().unwrap().clone();
    let manager = state.manager.clone();
    let toast = state.toast.clone();
    let folder = tauri::async_runtime::spawn_blocking(move || {
        manager.clip_all(&settings)
    }).await.map_err(|e| e.to_string())??;
    if state.settings.lock().unwrap().notify_clip {
        toast.show(&format!("Clip saved to {}", folder));
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
    // stop any running transcription
    TRANSCRIBE_STOP.store(true, std::sync::atomic::Ordering::Relaxed);
    hide_and_release(&window);
    state.toast.show("ClipThis hidden to tray");
}

#[tauri::command]
fn list_clips(state: State<AppState>) -> Result<Vec<ClipFolderDto>, String> {
    let settings = state.settings.lock().unwrap().clone();
    let base = std::path::PathBuf::from(&settings.output_dir);
    if !base.exists() {
        return Ok(vec![]);
    }
    let mut folders = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&base) {
        for e in entries.flatten() {
            if !e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let folder_path = e.path();
            let folder_name = e.file_name().to_string_lossy().to_string();
            let mut files = Vec::new();
            if let Ok(sub) = std::fs::read_dir(&folder_path) {
                for f in sub.flatten() {
                    let path = f.path();
                    if let Some(ext) = path.extension() {
                        let ext = ext.to_string_lossy().to_lowercase();
                        if ext == "wav" || ext == "flac" || ext == "m4a" {
                            if let Ok(meta) = f.metadata() {
                                files.push(ClipFileDto {
                                    name: f.file_name().to_string_lossy().to_string(),
                                    path: path.to_string_lossy().to_string(),
                                    size: meta.len(),
                                    duration_ms: 0,
                                    waveform: Vec::new(),
                                });
                            }
                        }
                    }
                }
            }
            files.sort_by(|a, b| a.name.cmp(&b.name));
            folders.push(ClipFolderDto {
                name: folder_name,
                path: folder_path.to_string_lossy().to_string(),
                files,
            });
        }
    }
    folders.sort_by(|a, b| b.name.cmp(&a.name)); // newest first
    Ok(folders)
}

fn clip_info(path: &std::path::Path) -> (u64, Vec<u8>) {
    let Ok(file) = std::fs::File::open(path) else { return (0, vec![14; 60]) };
    let Ok(source) = rodio::Decoder::try_from(std::io::BufReader::new(file)) else { return (0, vec![14; 60]) };
    use rodio::Source;
    let duration_ms = source.total_duration().map(|d| d.as_millis() as u64).unwrap_or(0);
    const BARS: usize = 60;
    let samples: Vec<f32> = source.collect();
    if samples.is_empty() {
        return (duration_ms, vec![14; BARS]);
    }
    let mut peaks = vec![0.0f32; BARS];
    for (i, &sample) in samples.iter().enumerate() {
        let bucket = (i * BARS / samples.len()).min(BARS - 1);
        peaks[bucket] = peaks[bucket].max(sample.abs());
    }
    let max = peaks.iter().copied().fold(0.001f32, f32::max);
    let wave: Vec<u8> = peaks.into_iter().map(|p| {
        let norm = (p / max).sqrt();
        ((norm * 86.0) + 14.0).clamp(14.0, 100.0) as u8
    }).collect();
    (duration_ms, wave)
}

#[tauri::command]
async fn enrich_clip(path: String) -> Result<(u64, Vec<u8>), String> {
    let cache = CLIP_CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    {
        let guard = cache.lock().unwrap();
        if let Some(val) = guard.get(&path) {
            return Ok(val.clone());
        }
    }
    let p_clone = path.clone();
    let res = tauri::async_runtime::spawn_blocking(move || {
        let p = std::path::PathBuf::from(&p_clone);
        if !p.exists() { return Err("clip file not found".to_string()); }
        Ok(clip_info(&p))
    }).await.map_err(|e| e.to_string())??;
    {
        let mut guard = cache.lock().unwrap();
        guard.insert(path, res.clone());
    }
    Ok(res)
}

#[tauri::command]
fn play_clip(path: String) -> Result<(), String> {
    let holder = PLAYER_STREAM.get_or_init(|| std::sync::Mutex::new(None));
    let mut guard = holder.lock().map_err(|_| "audio player lock failed".to_string())?;
    if guard.is_none() {
        *guard = Some(rodio::DeviceSinkBuilder::open_default_sink().map_err(|e| e.to_string())?);
    }
    let players = CURRENT_PLAYER.get_or_init(|| std::sync::Mutex::new(None));
    let mut current = players.lock().map_err(|_| "audio player lock failed".to_string())?;
    if let Some(active) = current.as_ref() {
        if active.path == path {
            if active.player.is_paused() { active.player.play(); } else { active.player.pause(); }
            return Ok(());
        }
    }
    if let Some(active) = current.take() { active.player.stop(); }

    let file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
    let source = rodio::Decoder::try_from(std::io::BufReader::new(file)).map_err(|e| e.to_string())?;
    use rodio::Source;
    let duration_ms = source.total_duration().map(|d| d.as_millis() as u64).unwrap_or_else(|| {
        let cache = CLIP_CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
        cache.lock().unwrap().get(&path).map(|(d, _)| *d).unwrap_or(0)
    });
    let player = rodio::Player::connect_new(guard.as_ref().unwrap().mixer());
    player.append(source);
    *current = Some(ActivePlayer { path: path.clone(), duration_ms, player });
    Ok(())
}

#[tauri::command]
fn playback_state() -> Result<PlaybackStateDto, String> {
    let Some(holder) = CURRENT_PLAYER.get() else {
        return Ok(PlaybackStateDto { path: None, position_ms: 0, duration_ms: 0, playing: false });
    };
    let guard = holder.lock().map_err(|_| "audio player lock failed".to_string())?;
    let Some(active) = guard.as_ref() else {
        return Ok(PlaybackStateDto { path: None, position_ms: 0, duration_ms: 0, playing: false });
    };
    Ok(PlaybackStateDto {
        path: Some(active.path.clone()),
        position_ms: active.player.get_pos().as_millis() as u64,
        duration_ms: active.duration_ms,
        playing: !active.player.is_paused() && !active.player.empty(),
    })
}

#[tauri::command]
fn pause_clip() -> Result<(), String> {
    if let Some(holder) = CURRENT_PLAYER.get() {
        if let Some(active) = holder.lock().map_err(|_| "audio player lock failed".to_string())?.as_ref() {
            active.player.pause();
        }
    }
    Ok(())
}

#[tauri::command]
fn resume_clip() -> Result<(), String> {
    if let Some(holder) = CURRENT_PLAYER.get() {
        if let Some(active) = holder.lock().map_err(|_| "audio player lock failed".to_string())?.as_ref() {
            active.player.play();
        }
    }
    Ok(())
}

#[tauri::command]
fn set_playback_volume(volume: f32) -> Result<(), String> {
    if let Some(holder) = CURRENT_PLAYER.get() {
        if let Some(active) = holder.lock().map_err(|_| "audio player lock failed".to_string())?.as_ref() {
            active.player.set_volume(volume.clamp(0.0, 1.0));
        }
    }
    Ok(())
}

#[tauri::command]
fn seek_clip(path: String, position_ms: u64) -> Result<(), String> {
    let Some(holder) = CURRENT_PLAYER.get() else { return Ok(()) };
    let guard = holder.lock().map_err(|_| "audio player lock failed".to_string())?;
    if let Some(active) = guard.as_ref() {
        if active.path == path {
            active.player.try_seek(std::time::Duration::from_millis(position_ms))
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[tauri::command]
fn stop_clip() -> Result<(), String> {
    if let Some(holder) = CURRENT_PLAYER.get() {
        if let Some(active) = holder.lock().map_err(|_| "audio player lock failed".to_string())?.take() {
            active.player.stop();
        }
    }
    Ok(())
}

#[tauri::command]
fn transcribe_clip(path: String) -> Result<String, String> {
    use std::process::Command;
    TRANSCRIBE_STOP.store(false, std::sync::atomic::Ordering::Relaxed);
    let source = std::path::PathBuf::from(&path);
    if !source.exists() {
        return Err("clip file not found".to_string());
    }
    // Windows SpeechRecognitionEngine only accepts PCM WAV reliably. Convert
    // every ClipThis format through ffmpeg first, then delete the temporary WAV.
    // ffmpeg is also used by VLC/most codec tools and is checked at runtime.
    let temp = std::env::temp_dir().join(format!(
        "clipthis_transcribe_{}_{}.wav",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let ffmpeg = [
        std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("ffmpeg.exe"))),
        Some(std::path::PathBuf::from("ffmpeg.exe")),
    ]
    .into_iter()
    .flatten()
    .find(|p| p.exists() || p.to_string_lossy() == "ffmpeg.exe")
    .ok_or_else(|| "FFmpeg is required for transcription. Put ffmpeg.exe next to ClipThis.exe or add FFmpeg to PATH.".to_string())?;
    let converted = Command::new(ffmpeg)
        .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(&source)
        .args(["-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"])
        .arg(&temp)
        .status()
        .map_err(|_| "ffmpeg is required for transcription; install ffmpeg or add it to PATH".to_string())?;
    if !converted.success() {
        let _ = std::fs::remove_file(&temp);
        return Err("could not convert this audio file to PCM for speech recognition".to_string());
    }
    let script = format!(
        "Add-Type -AssemblyName System.Speech; \
        $r = New-Object System.Speech.Recognition.SpeechRecognitionEngine; \
        $r.SetInputToWaveFile('{}'); \
        $g = New-Object System.Speech.Recognition.DictationGrammar; \
        $r.LoadGrammar($g); \
        $res = $r.Recognize(); \
        if ($res) {{ $res.Text }} else {{ '' }}; \
        $r.Dispose()",
        temp.to_string_lossy().replace('\'', "''")
    );
    let output = Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .output()
        .map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&temp);
    if TRANSCRIBE_STOP.load(std::sync::atomic::Ordering::Relaxed) {
        return Err("cancelled".to_string());
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        Ok("(no speech detected)".to_string())
    } else {
        Ok(text)
    }
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
    let current_exe = std::env::current_exe().map_err(|e| e.to_string())?;

    // PowerShell script: shows a small dark borderless "Updating…" window
    // with rounded corners, runs the swap, then closes and starts the new exe.
    let ps1 = std::env::temp_dir().join("clipthis_updater.ps1");
    let ps1_content = format!(
        "Add-Type -AssemblyName System.Windows.Forms\r\n\
        Add-Type -AssemblyName System.Drawing\r\n\
        $w = 280; $h = 80; $r = 10\r\n\
        $gp = New-Object Drawing.Drawing2D.GraphicsPath\r\n\
        $gp.AddArc(0, 0, $r*2, $r*2, 180, 90)\r\n\
        $gp.AddArc($w - $r*2, 0, $r*2, $r*2, 270, 90)\r\n\
        $gp.AddArc($w - $r*2, $h - $r*2, $r*2, $r*2, 0, 90)\r\n\
        $gp.AddArc(0, $h - $r*2, $r*2, $r*2, 90, 90)\r\n\
        $gp.CloseFigure()\r\n\
        $f = New-Object Windows.Forms.Form\r\n\
        $f.Size = New-Object Drawing.Size($w, $h)\r\n\
        $f.StartPosition = 'CenterScreen'\r\n\
        $f.FormBorderStyle = 'None'\r\n\
        $f.BackColor = [Drawing.ColorTranslator]::FromHtml('#1b1d26')\r\n\
        $f.ShowInTaskbar = $false\r\n\
        $f.TopMost = $true\r\n\
        $f.Region = New-Object Drawing.Region($gp)\r\n\
        $l = New-Object Windows.Forms.Label\r\n\
        $l.Text = 'Updating ClipThis...'\r\n\
        $l.ForeColor = [Drawing.ColorTranslator]::FromHtml('#e8e9ee')\r\n\
        $l.Font = New-Object Drawing.Font('Segoe UI',12)\r\n\
        $l.AutoSize = $false\r\n\
        $l.Size = New-Object Drawing.Size($w, $h)\r\n\
        $l.TextAlign = [Drawing.ContentAlignment]::MiddleCenter\r\n\
        $f.Controls.Add($l)\r\n\
        $f.Show()\r\n\
        [Windows.Forms.Application]::DoEvents()\r\n\
        Start-Sleep -Seconds 3\r\n\
        Stop-Process -Name ClipThis -Force -ErrorAction SilentlyContinue 2>$null\r\n\
        Start-Sleep -Seconds 2\r\n\
        Copy-Item '{}' '{}' -Force\r\n\
        Remove-Item '{}' -Force -ErrorAction SilentlyContinue\r\n\
        $f.Close()\r\n\
        Start-Process -FilePath '{}'\r\n\
        Remove-Item $MyInvocation.MyCommand.Source -Force -ErrorAction SilentlyContinue\r\n",
        temp.display().to_string().replace('\'', "''"),
        current_exe.display().to_string().replace('\'', "''"),
        temp.display().to_string().replace('\'', "''"),
        current_exe.display().to_string().replace('\'', "''"),
    );
    std::fs::write(&ps1, ps1_content).map_err(|e| e.to_string())?;

    std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-ExecutionPolicy", "Bypass",
            "-WindowStyle", "Hidden",
            "-File", ps1.to_str().unwrap(),
        ])
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
    let args: Vec<String> = std::env::args().collect();
    let is_autostart = args.iter().any(|a| a == "--autostart");

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
        // .plugin(tauri_plugin_notification::init())
        .manage(app_state)
        .setup(move |app| {
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
            // User start → show window. Autostart → stay in tray.
            if !is_autostart {
                open_window(app.handle());
            }
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
        .invoke_handler(tauri::generate_handler![get_settings, set_settings, reset_settings, trigger_clip, list_processes, pick_folder, hide_to_tray, get_version, check_update, apply_update, list_clips, enrich_clip, play_clip, stop_clip, playback_state, pause_clip, resume_clip, set_playback_volume, seek_clip, transcribe_clip])
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
