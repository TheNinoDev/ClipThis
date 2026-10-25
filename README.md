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
  <b>A high-performance Windows application to continuously buffer and clip per-app audio into isolated tracks.</b>
</p>

---

## 🚀 Features

- **Per-App Audio Loopback**: Each running game, voice client, browser, or media player gets its own isolated, high-quality audio track using WASAPI process loopback capture.
- **Smart Silence & Empty Track Suppression**: Automatically analyzes audio amplitudes before saving — never pollutes your hard drive with silent, empty recordings or dead folders.
- **In-App Clips Player & Visualizer**:
  - Figma-inspired dark UI with organized date and time cards (`DD Mon YYYY   HH:MM`).
  - Real, interactive audio waveform visualization extracted directly from the recorded audio.
  - Interactive click-to-seek timeline with instant backward and forward random access.
  - Per-track playback controls, elapsed/total duration indicators, and volume sliders.
- **Microphone Capture**: Optional dedicated mic recording saved alongside your application clips.
- **Multiple Audio Formats**:
  - **AAC (.m4a)**: Modern, efficient lossy compression (~10x smaller), crystal clear sound quality, plays natively across all Windows media players, phones, and web browsers.
  - **FLAC (.flac)**: Lossless compression (~3x smaller) without losing a single bit of audio fidelity.
  - **WAV 32-bit (.wav)**: Full 32-bit floating-point audio precision straight from the capture engine.
  - **WAV 16-bit (.wav)**: Classic uncompressed 16-bit PCM.
- **Native Windows Toast Notifications**: Sleek, custom dark Win32 toast notifications with smooth cubic ease-out animations indicating when clips are saved, when minimized, when autostarting, or when updates are ready.
- **Seamless Auto-Updates**: One-click check and update directly from GitHub Releases with a styled background updater card and automatic relaunch.
- **System Tray & Autostart**:
  - Starts automatically with Windows into the system tray when enabled.
  - Auto-hides to tray after 60 seconds of inactivity to save memory.
  - Double-click or single-click the tray icon to restore the UI anytime.
- **Customizable Global Hotkey**: Click to bind any key combination directly from your keyboard (e.g. `Alt+F`, `Ctrl+Shift+F9`).

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
| **Clip Length** | 10s, 15s, 30s, 60s, 120s | Time window of audio retained in memory |
| **Format** | `AAC`, `FLAC`, `WAV 32`, `WAV 16` | Encoding format for saved clips |
| **Mic** | On / Off | Also records your microphone to a separate audio file |
| **Autostart** | On / Off | Launches automatically in the tray when Windows boots |
| **Notify on Clip** | On / Off | Shows native toast when a clip is saved |

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
