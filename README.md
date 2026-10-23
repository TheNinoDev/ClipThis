# ClipThis

ClipThis is a portable Windows app that continuously records the audio of every running application. Press a global hotkey and the last few seconds of each app's own audio are saved into a timestamped folder — one file per app.

## Features
- Per-app audio capture — each program gets its own file
- Global hotkey, fully customizable (press the keys, don't type them)
- Configurable clip length (seconds)
- Multiple compression options: WAV 32-bit, WAV 16-bit, FLAC, and a custom 4-bit ADPCM
- Microphone track (optional, saved as its own file)
- System tray — closes to tray, auto-hides after 1 minute of inactivity
- Single instance, frameless window, draggable, portable single .exe
- Auto-update from GitHub releases

## Usage
1. Run `ClipThis.exe` — it starts in the tray.
2. Open the window from the tray icon to change settings.
3. Press your hotkey (default Alt+F) to save a clip.

Clips are saved to `Music\ClipThis\<timestamp>\` with one file per application.

## Compression options
- **WAV 32** — lossless, full precision
- **WAV 16** — lossless for 16-bit sources, half the size
- **FLAC** — lossless, ~3x smaller
- **ADPCM** — custom 4-bit lossy, ~8x smaller, plays in any standard player

The ADPCM codec is standard IMA ADPCM (WAVE_FORMAT_IMA_ADPCM) and needs no installation — Windows Media Player, VLC and Audacity play it natively.

## License
MIT