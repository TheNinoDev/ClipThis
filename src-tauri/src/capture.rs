use flexaudio::{self, processes, SourceKind, StreamConfig};
use flexaudio_mic::CpalMicBackend;
use flexaudio_core::backend::CaptureBackend;
use flexaudio_core::{RawSink, raw_ring};
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use flexaudio_encode::FlacWriter;

#[derive(Clone, Serialize, serde::Deserialize)]
pub struct Settings {
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
    #[serde(default = "default_output_dir")]
    pub output_dir: String,
    #[serde(default = "default_format")]
    pub format: String,
    #[serde(default)]
    pub autostart: bool,
    #[serde(default = "default_clip_seconds")]
    pub clip_seconds: u32,
    #[serde(default = "default_notify_clip")]
    pub notify_clip: bool,
    #[serde(default)]
    pub capture_mic: bool,
}

fn default_hotkey() -> String { "Ctrl+Shift+F9".to_string() }
fn default_format() -> String { "flac".to_string() }

fn default_notify_clip() -> bool {
    true
}

fn default_clip_seconds() -> u32 {
    30
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: "Ctrl+Shift+F9".to_string(),
            output_dir: default_output_dir(),
            format: "wav".to_string(),
            autostart: false,
            clip_seconds: 30,
            notify_clip: true,
            capture_mic: false,
        }
    }
}

#[allow(dead_code)]
pub fn default_output_dir() -> String {
    if let Some(d) = dirs::audio_dir() {
        d.join("ClipThis").to_string_lossy().to_string()
    } else if let Some(d) = dirs::home_dir() {
        d.join("ClipThis").to_string_lossy().to_string()
    } else {
        "ClipThis".to_string()
    }
}

pub struct RingBuffer {
    /// VecDeque, not Vec: trimming the oldest audio must be O(chunk), not a
    /// memmove of the whole multi-megabyte buffer on every incoming packet.
    pub samples: VecDeque<i16>,
    pub sample_rate: u32,
    pub channels: u16,
    pub max_samples: usize,
}

pub struct CaptureEntry {
    pub pid: u32,
    pub name: String,
    pub buf: Arc<Mutex<RingBuffer>>,
    pub stop: Arc<AtomicBool>,
}

pub struct CaptureManager {
    pub buffers: Mutex<HashMap<u32, CaptureEntry>>,
    pub clip_seconds: Mutex<u32>,
    pub mic_buf: Mutex<Option<Arc<Mutex<RingBuffer>>>>,
    pub mic_stop: Mutex<Option<Arc<AtomicBool>>>,
}

impl CaptureManager {
    pub fn new(initial: u32) -> Arc<Self> {
        let m = Arc::new(Self {
            buffers: Mutex::new(HashMap::new()),
            clip_seconds: Mutex::new(initial),
            mic_buf: Mutex::new(None),
            mic_stop: Mutex::new(None),
        });
        let m2 = m.clone();
        thread::spawn(move || {
            loop {
                // enumerate processes
                let list = processes().unwrap_or_default();
                let current: std::collections::HashSet<u32> = list.iter().map(|p| p.pid).collect();
                {
                    let mut map = m2.buffers.lock().unwrap();
                    // mark removed
                    let to_remove: Vec<u32> = map.keys().filter(|pid| !current.contains(pid)).copied().collect();
                    for pid in to_remove {
                        if let Some(e) = map.remove(&pid) {
                            e.stop.store(true, Ordering::Relaxed);
                        }
                    }
                    // add new
                    for p in &list {
                        if !map.contains_key(&p.pid) {
                            let cs = *m2.clip_seconds.lock().unwrap();
                            let buf = Arc::new(Mutex::new(RingBuffer {
                                samples: VecDeque::new(),
                                sample_rate: 48000,
                                channels: 2,
                                max_samples: 48000 * 2 * cs as usize,
                            }));
                            let stop = Arc::new(AtomicBool::new(false));
                            let buf2 = buf.clone();
                            let stop2 = stop.clone();
                            let name = p.name.clone();
                            let pid = p.pid;
                            thread::spawn(move || {
                                run_capture(pid, name, buf2, stop2);
                            });
                            map.insert(p.pid, CaptureEntry {
                                pid: p.pid,
                                name: p.name.clone(),
                                buf,
                                stop,
                            });
                        }
                    }
                }
                thread::sleep(Duration::from_secs(2));
            }
        });
        m
    }

    pub fn set_clip_seconds(&self, secs: u32) {
        *self.clip_seconds.lock().unwrap() = secs;
        let max = 48000 * 2 * secs as usize;
        let map = self.buffers.lock().unwrap();
        for e in map.values() {
            let mut rb = e.buf.lock().unwrap();
            rb.max_samples = max;
            if rb.samples.len() > max {
                let excess = rb.samples.len() - max;
                rb.samples.drain(0..excess);
            }        }
    }

    pub fn start_mic(&self, cs: u32) {
        let mut stop_guard = self.mic_stop.lock().unwrap();
        if stop_guard.is_some() {
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let buf = Arc::new(Mutex::new(RingBuffer {
            samples: VecDeque::new(),
            sample_rate: 48000,
            channels: 1,
            max_samples: 48000 * 1 * cs as usize,
        }));
        let buf2 = buf.clone();
        let handle = thread::spawn(move || {
            let mut backend = CpalMicBackend::new(None);
            let native = backend.native_format();
            let sr = native.0;
            let ch = native.1;
            let (prod, mut cons) = raw_ring(sr as usize * ch as usize * 2);
            let sink = RawSink::new(prod, sr, ch);
            if backend.start(sink).is_err() {
                return;
            }
            // fix the ring's declared rate/channels
            {
                let mut rb = buf2.lock().unwrap();
                rb.sample_rate = sr;
                rb.channels = ch;
                rb.max_samples = sr as usize * ch as usize * cs as usize;
            }
            let mut tmp = vec![0f32; 1024];
            while !stop2.load(Ordering::Relaxed) {
                let n = cons.pop_slice(&mut tmp);
                if n == 0 {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }
                let mut rb = buf2.lock().unwrap();
                rb.samples.extend(tmp[..n].iter().map(|s| {
                    let v = if s.is_finite() { s.clamp(-1.0, 1.0) } else { 0.0 };
                    (v * 32767.0).round() as i16
                }));
                if rb.samples.len() > rb.max_samples {
                    let excess = rb.samples.len() - rb.max_samples;
                    rb.samples.drain(0..excess);
                }
            }
        });
        *stop_guard = Some(stop);
        *self.mic_buf.lock().unwrap() = Some(buf);
        // keep handle alive by leaking into a global? store it
        let _ = handle;
    }

    pub fn stop_mic(&self) {
        if let Some(stop) = self.mic_stop.lock().unwrap().take() {
            stop.store(true, Ordering::Relaxed);
        }
        *self.mic_buf.lock().unwrap() = None;
    }

    pub fn clip_all(&self, settings: &Settings) -> Result<String, String> {
        let timestamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
        let base = PathBuf::from(&settings.output_dir);
        let folder = base.join(&timestamp);
        fs::create_dir_all(&folder).map_err(|e| e.to_string())?;

        let entries: Vec<(u32, String, Arc<Mutex<RingBuffer>>)> = {
            let map = self.buffers.lock().unwrap();
            map.values().map(|e| (e.pid, e.name.clone(), e.buf.clone())).collect()
        };

        for (pid, name, buf) in entries {
            let (samples, sr, ch) = {
                let mut rb = buf.lock().unwrap();
                (rb.samples.make_contiguous().to_vec(), rb.sample_rate, rb.channels)
            };
            if samples.is_empty() {
                continue;
            }
            let max_samples = sr as usize * ch as usize * settings.clip_seconds as usize;
            let samples = if samples.len() > max_samples {
                samples[samples.len() - max_samples..].to_vec()
            } else {
                samples
            };
            if samples.is_empty() {
                continue;
            }
            let safe_name: String = name.chars().map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
            let ext = if settings.format == "flac" { "flac" } else if settings.format == "aac" { "m4a" } else { "wav" };
            let path = folder.join(format!("{}_{}_{}s.{}", safe_name, pid, settings.clip_seconds, ext));
            match settings.format.as_str() {
                "flac" => {
                    let f: Vec<f32> = samples.iter().map(|s| *s as f32 / 32768.0).collect();
                    let mut writer = FlacWriter::create(&path, sr, ch).map_err(|e| e.to_string())?;
                    writer.write_chunk(&f).map_err(|e| e.to_string())?;
                    writer.finalize().map_err(|e| e.to_string())?;
                }
                "wav16" => crate::codec::write_wav_i16(&path, &samples, sr, ch)?,
                "adpcm" => crate::codec::write_ima_adpcm(&path, &samples, sr, ch)?,
                "aac" => crate::codec_aac::write_aac(&path, &samples, sr, ch)?,
                _ => crate::codec::write_wav_f32(&path, &samples, sr, ch)?,
            }
        }
        // Microphone track, if enabled and captured.
        if settings.capture_mic {
            if let Some(buf) = self.mic_buf.lock().unwrap().clone() {
                let (samples, sr, ch) = {
                    let mut rb = buf.lock().unwrap();
                    (rb.samples.make_contiguous().to_vec(), rb.sample_rate, rb.channels)
                };
                if !samples.is_empty() {
                    let max_samples = sr as usize * ch as usize * settings.clip_seconds as usize;
                    let samples = if samples.len() > max_samples {
                        samples[samples.len() - max_samples..].to_vec()
                    } else {
                        samples
                    };
                    if !samples.is_empty() {
                        let safe = "Microphone";
                        let ext = if settings.format == "flac" { "flac" } else if settings.format == "aac" { "m4a" } else { "wav" };
                        let path = folder.join(format!("{}_0_{}s.{}", safe, settings.clip_seconds, ext));
                        match settings.format.as_str() {
                            "flac" => {
                                let f: Vec<f32> = samples.iter().map(|s| *s as f32 / 32768.0).collect();
                                let mut writer = FlacWriter::create(&path, sr, ch).map_err(|e| e.to_string())?;
                                writer.write_chunk(&f).map_err(|e| e.to_string())?;
                                writer.finalize().map_err(|e| e.to_string())?;
                            }
                            "wav16" => crate::codec::write_wav_i16(&path, &samples, sr, ch)?,
                            "adpcm" => crate::codec::write_ima_adpcm(&path, &samples, sr, ch)?,
                            "aac" => crate::codec_aac::write_aac(&path, &samples, sr, ch)?,
                            _ => crate::codec::write_wav_f32(&path, &samples, sr, ch)?,
                        }
                    }
                }
            }
        }
        Ok(folder.to_string_lossy().to_string())
    }
}

fn run_capture(pid: u32, name: String, buf: Arc<Mutex<RingBuffer>>, stop: Arc<AtomicBool>) {
    let config = StreamConfig {
        kind: SourceKind::ProcessLoopback,
        target_pid: Some(pid),
        ..Default::default()
    };
    let mut stream = match flexaudio::open(config) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("clipthis: open pid {} ({}): {}", pid, name, e);
            return;
        }
    };
    if let Err(e) = stream.start() {
        eprintln!("clipthis: start pid {} ({}): {}", pid, name, e);
        return;
    }
    while !stop.load(Ordering::Relaxed) {
        // Drain in bounded batches, then yield. Polling without a floor turns
        // the thread into a spin loop and burns a whole core per stream.
        let mut batch = 0;
        while batch < 4 {
            match stream.poll_chunk() {
                Some(chunk) => {
                    batch += 1;
                    if chunk.data.is_empty() {
                        continue;
                    }
                    let mut rb = buf.lock().unwrap();
                    rb.samples.extend(chunk.data.iter().map(|s| {
                        let v = if s.is_finite() { s.clamp(-1.0, 1.0) } else { 0.0 };
                        (v * 32767.0).round() as i16
                    }));
                    if rb.samples.len() > rb.max_samples {
                        let excess = rb.samples.len() - rb.max_samples;
                        rb.samples.drain(0..excess);
                    }
                }
                None => break,
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
    stream.stop();
}

pub fn set_autostart(enabled: bool) -> Result<(), String> {
    use windows::Win32::System::Registry::*;
    use windows::core::PCWSTR;
    let subkey: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Run\0".encode_utf16().collect();
    let mut hkey = HKEY::default();
    unsafe {
        let res = RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(subkey.as_ptr()), 0, KEY_SET_VALUE, &mut hkey);
        if res.is_err() {
            return Err(format!("{:?}", res));
        }
        if enabled {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let valstr = format!("\"{}\" --autostart\0", exe.display());
            let val: Vec<u16> = valstr.encode_utf16().collect();
            let name: Vec<u16> = "ClipThis\0".encode_utf16().collect();
            let bytes: Vec<u8> = val.iter().flat_map(|c| c.to_le_bytes()).collect();
            let r = RegSetValueExW(hkey, PCWSTR(name.as_ptr()), 0, REG_SZ, Some(&bytes));
            let _ = RegCloseKey(hkey);
            if r.is_err() {
                return Err(format!("{:?}", r));
            }
        } else {
            let name: Vec<u16> = "ClipThis\0".encode_utf16().collect();
            let _ = RegDeleteValueW(hkey, PCWSTR(name.as_ptr()));
            let _ = RegCloseKey(hkey);
        }
    }
    Ok(())
}
