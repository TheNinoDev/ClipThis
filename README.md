<p align="center">
  <img src="https://github.com/TheNinoDev/ClipThis/blob/main/banner.png?raw=true" alt="ClipThis Banner" width="100%">
</p>

<h1 align="center">ClipThis</h1>

<p align="center">
  <img src="https://img.shields.io/github/stars/TheNinoDev/ClipThis?style=for-the-badge" alt="GitHub Stars">
  <img src="https://img.shields.io/github/downloads/TheNinoDev/ClipThis/total?style=for-the-badge" alt="Total Downloads">
  <img src="https://img.shields.io/github/v/release/TheNinoDev/ClipThis?style=for-the-badge" alt="Latest Release">
  <img src="https://img.shields.io/github/license/TheNinoDev/ClipThis?style=for-the-badge" alt="License">
</p>

<p align="center">
  <b>Continuously buffers and clips per-app audio on Windows into separate, isolated tracks.</b>
</p>

---

## 🚀 Features

- **Per-App Audio Isolation**: Capture clean audio from your games, voice calls (Discord, TeamSpeak), music, and browser streams individually into their own separate files.
- **Built-In Audio Player & Waveform Visualizer**:
  - Browse your saved clips in an organized, modern dark UI grouped by session.
  - Interactive audio waveforms showing the exact sound peaks for every track.
  - Click anywhere on the waveform to seek forward or backward instantly.
  - Dedicated play/pause controls, duration counters, and volume sliders.
- **Smart Silence Filtering**: Automatically detects silent tracks and skips empty recordings — keeping your folders clean and saving disk space.
- **Microphone Recording**: Option to record your microphone simultaneously as its own separate track.
- **Multiple Audio Formats**:
  - **AAC (.m4a)**: Modern, highly-compressed audio with crystal clear quality (~10x smaller, plays everywhere).
  - **FLAC (.flac)**: Studio-grade lossless compression (~3x smaller).
  - **WAV 32-bit (.wav)**: Full floating-point capture quality.
  - **WAV 16-bit (.wav)**: Standard uncompressed PCM audio.
- **Desktop Toast Notifications**: Clean, non-intrusive dark notifications when a clip is saved, when running in the background, or when updates are ready.
- **One-Click Auto-Updates**: Checks for updates from GitHub and automatically downloads and restarts into the newest version.
- **System Tray & Autostart**:
  - Runs quietly in the Windows system tray with low memory usage.
  - Optional autostart with Windows so you never miss a moment.
- **Customizable Global Hotkey**: Set any shortcut directly from your keyboard (e.g. `Alt+F`, `Ctrl+Shift+F9`).

---

## 📥 Installation & Running

1. Download `clipthis.exe` from the [Latest Release](https://github.com/TheNinoDev/ClipThis/releases/latest).
2. Run `clipthis.exe`.
3. Open the window from the system tray icon to configure your hotkey, format, clip duration, and autostart preferences.
4. Press your hotkey anytime while gaming or listening to clip your audio!

Clips are saved to your `Music\ClipThis\<timestamp>\` directory with each program named and timestamped clearly.

---

## ⚙️ Keyboard Controls & Settings

| Setting | Options / Default | Description |
|---|---|---|
| **Hotkey** | `Alt+F` (configurable) | Global shortcut to capture buffered audio |
| **Clip Length** | 10s, 15s, 30s, 60s, 120s | Length of past audio kept in buffer |
| **Format** | `AAC`, `FLAC`, `WAV 32`, `WAV 16` | Encoding format for saved clips |
| **Mic** | On / Off | Records microphone into a separate audio track |
| **Autostart** | On / Off | Launches automatically in the tray when Windows boots |
| **Notify on Clip** | On / Off | Shows desktop notification when a clip is saved |

---

## ⭐ Star History

<p align="center">
  <a href="https://www.star-history.com/#TheNinoDev/ClipThis&type=date&legend=top-left">
    <img src="https://api.star-history.com/svg?repos=TheNinoDev/ClipThis&type=Date" alt="Star History Chart">
  </a>
</p>

---

## 📄 License

MIT License.
