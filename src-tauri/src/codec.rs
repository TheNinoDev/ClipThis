//! Audio format writers for WAV formats.

use std::path::Path;

/// Float32 WAV: 32-bit floating-point WAV container.
pub fn write_wav_f32(path: &Path, samples: &[i16], sr: u32, ch: u16) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels: ch,
        sample_rate: sr,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(path, spec).map_err(|e| e.to_string())?;
    for s in samples {
        writer
            .write_sample(*s as f32 / 32768.0)
            .map_err(|e| e.to_string())?;
    }
    writer.finalize().map_err(|e| e.to_string())
}

/// PCM16 WAV: 16-bit integer PCM audio.
pub fn write_wav_i16(path: &Path, samples: &[i16], sr: u32, ch: u16) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels: ch,
        sample_rate: sr,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).map_err(|e| e.to_string())?;
    for s in samples {
        writer.write_sample(*s).map_err(|e| e.to_string())?;
    }
    writer.finalize().map_err(|e| e.to_string())
}
