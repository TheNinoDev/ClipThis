//! Clip output encoders.
//!
//! Every path writes a container that any player opens without ClipThis:
//!
//! | option | bytes/s (48k stereo) | kind |
//! |--------|----------------------|------|
//! | WAV float32 | 384 KB | lossless, full capture precision |
//! | WAV PCM16   | 192 KB | lossless for 16-bit sources, half the size |
//! | FLAC        | ~110-150 KB | lossless, compressed (flexaudio-encode) |
//! | ADPCM       | 96 KB | lossy 4:1, our own encoder, standard WAV |
//!
//! The ADPCM path is a hand-written IMA 4-bit encoder tuned for program audio:
//! it estimates the starting step size from the block's first sample instead of
//! always starting at index 0 (which wastes the first milliseconds of a loud
//! onset), and it blocks at 505 samples so every decoder in the wild agrees on
//! the layout. Output is a plain `WAVE_FORMAT_IMA_ADPCM` file.

use std::io::{BufWriter, Write};
use std::path::Path;

const IMA_INDEX_TABLE: [i32; 16] = [
    -1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8,
];

const IMA_STEP_TABLE: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66,
    73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449,
    494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272,
    2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493,
    10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];

/// IMA blocks are 505 samples: 1 in the header + 504 encoded, which is exactly
/// 126 four-byte groups (stereo) / 63 groups (mono). This is the value every
/// decoder expects, so do not change it.
const IMA_SAMPLES_PER_BLOCK: usize = 505;

/// Float32 WAV: the ring is 16-bit, so this is the same precision in a fatter
/// container. Kept for players/tools that want float.
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

/// PCM16 WAV: exactly what the ring holds, written straight through.
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

fn ima_encode(sample: i32, predictor: &mut i32, index: &mut i32) -> u8 {
    let step = IMA_STEP_TABLE[*index as usize];
    let mut diff = sample - *predictor;
    let mut nibble: u8 = 0;
    if diff < 0 {
        nibble = 8;
        diff = -diff;
    }
    let mut vpdiff = step >> 3;
    if diff >= step {
        nibble |= 4;
        diff -= step;
        vpdiff += step;
    }
    if diff >= step >> 1 {
        nibble |= 2;
        diff -= step >> 1;
        vpdiff += step >> 1;
    }
    if diff >= step >> 2 {
        nibble |= 1;
        vpdiff += step >> 2;
    }
    if nibble & 8 != 0 {
        *predictor -= vpdiff;
    } else {
        *predictor += vpdiff;
    }
    *predictor = (*predictor).clamp(-32768, 32767);
    *index = (*index + IMA_INDEX_TABLE[nibble as usize]).clamp(0, 88);
    nibble
}

#[cfg(test)]
fn ima_decode(nibble: u8, predictor: &mut i32, index: &mut i32) -> i32 {
    let step = IMA_STEP_TABLE[*index as usize];
    let mut vpdiff = step >> 3;
    if nibble & 4 != 0 {
        vpdiff += step;
    }
    if nibble & 2 != 0 {
        vpdiff += step >> 1;
    }
    if nibble & 1 != 0 {
        vpdiff += step >> 2;
    }
    if nibble & 8 != 0 {
        *predictor -= vpdiff;
    } else {
        *predictor += vpdiff;
    }
    *predictor = (*predictor).clamp(-32768, 32767);
    *index = (*index + IMA_INDEX_TABLE[(nibble & 0x0f) as usize]).clamp(0, 88);
    *predictor
}

/// Pick a starting step index from the block's average sample-to-sample delta.
/// Starting every block at index 0 costs the first few milliseconds of a loud
/// onset; estimating it costs nothing and is decoder-agnostic (the index is
/// carried in the block header).
fn ima_initial_index(block: &[i16], channels: usize, c: usize, frames: usize) -> i32 {
    let probe = frames.min(16);
    if probe < 2 {
        return 0;
    }
    let mut sum = 0i64;
    let mut prev = block[c] as i32;
    for f in 1..probe {
        let cur = block[f * channels + c] as i32;
        sum += (cur - prev).abs() as i64;
        prev = cur;
    }
    let target = ((sum / (probe as i64 - 1)) as i32).max(1);
    let mut i = 0usize;
    while i < 88 && IMA_STEP_TABLE[i] < target {
        i += 1;
    }
    i as i32
}

fn encode_block(samples: &[i16], start: usize, frames: usize, channels: usize, out: &mut Vec<u8>) {
    let mut block = vec![0i16; IMA_SAMPLES_PER_BLOCK * channels];
    let last = start + frames - 1;
    for f in 0..IMA_SAMPLES_PER_BLOCK {
        let src = if f < frames { start + f } else { last };
        for c in 0..channels {
            block[f * channels + c] = samples[src * channels + c];
        }
    }

    let mut predictor = [0i32; 2];
    let mut index = [0i32; 2];
    for c in 0..channels {
        let s = block[c] as i32;
        predictor[c] = s;
        index[c] = ima_initial_index(&block, channels, c, IMA_SAMPLES_PER_BLOCK);
        out.extend_from_slice(&(s as i16).to_le_bytes());
        out.push(index[c] as u8);
        out.push(0);
    }

    // IMA in WAV: 8 samples per channel per group, 4 bytes per channel, and the
    // first sample of each byte lives in the LOW nibble (see ffmpeg
    // adpcm_ima_wav_decode_frame). Channels are grouped, not interleaved.
    let mut f = 1usize;
    while f < IMA_SAMPLES_PER_BLOCK {
        for c in 0..channels {
            for k in 0..4 {
                let a = block[(f + k * 2) * channels + c] as i32;
                let b = block[(f + k * 2 + 1) * channels + c] as i32;
                let na = ima_encode(a, &mut predictor[c], &mut index[c]);
                let nb = ima_encode(b, &mut predictor[c], &mut index[c]);
                out.push((nb << 4) | na);
            }
        }
        f += 8;
    }
}

fn encode_ima_data(samples: &[i16], channels: usize) -> Vec<u8> {
    let frames = samples.len() / channels;
    let block_align = if channels == 2 { 512 } else { 256 };
    let mut out = Vec::with_capacity((frames / IMA_SAMPLES_PER_BLOCK + 1) * block_align);
    let mut pos = 0usize;
    while pos < frames {
        let n = (frames - pos).min(IMA_SAMPLES_PER_BLOCK);
        encode_block(samples, pos, n, channels, &mut out);
        pos += n;
    }
    while out.len() % block_align != 0 {
        out.push(0);
    }
    out
}

/// IMA ADPCM WAV: our own 4-bit lossy encoder, 4:1 against PCM16.
pub fn write_ima_adpcm(path: &Path, samples: &[i16], sr: u32, ch: u16) -> Result<(), String> {
    let channels = ch as usize;
    // IMA in WAV is defined for 1 and 2 channels; anything else falls back.
    if channels != 1 && channels != 2 {
        return write_wav_i16(path, samples, sr, ch);
    }
    if samples.is_empty() {
        return write_wav_i16(path, samples, sr, ch);
    }
    let data = encode_ima_data(samples, channels);
    let block_align: u16 = if channels == 2 { 512 } else { 256 };
    let avg_bytes = (sr as u64 * block_align as u64 / IMA_SAMPLES_PER_BLOCK as u64) as u32;
    let cb_size: u16 = 2;
    let fmt_size: u32 = 18 + cb_size as u32; // 20, matches the IMA spec

    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut w = BufWriter::new(file);
    let pad = (data.len() % 2) as u32;
    let riff = 4 + 8 + fmt_size + 8 + data.len() as u32 + pad;

    let io = |r: std::io::Result<()>| r.map_err(|e| e.to_string());

    io(w.write_all(b"RIFF"))?;
    io(w.write_all(&riff.to_le_bytes()))?;
    io(w.write_all(b"WAVE"))?;
    io(w.write_all(b"fmt "))?;
    io(w.write_all(&fmt_size.to_le_bytes()))?;
    io(w.write_all(&0x0011u16.to_le_bytes()))?; // WAVE_FORMAT_IMA_ADPCM
    io(w.write_all(&(channels as u16).to_le_bytes()))?;
    io(w.write_all(&sr.to_le_bytes()))?;
    io(w.write_all(&avg_bytes.to_le_bytes()))?;
    io(w.write_all(&block_align.to_le_bytes()))?;
    io(w.write_all(&4u16.to_le_bytes()))?; // bits per sample
    io(w.write_all(&cb_size.to_le_bytes()))?;
    io(w.write_all(&(IMA_SAMPLES_PER_BLOCK as u16).to_le_bytes()))?;
    io(w.write_all(b"data"))?;
    io(w.write_all(&(data.len() as u32).to_le_bytes()))?;
    io(w.write_all(&data))?;
    if pad == 1 {
        io(w.write_all(&[0u8]))?;
    }
    io(w.flush())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference decode of the block layout the writer emits. Used to prove the
    /// encoder/container agree; a spec mismatch would show up as garbage here.
    fn decode(data: &[u8], channels: usize) -> Vec<i16> {
        let block_align = if channels == 2 { 512 } else { 256 };
        let mut out = Vec::new();
        for block in data.chunks(block_align) {
            let mut predictor = [0i32; 2];
            let mut index = [0i32; 2];
            for c in 0..channels {
                let base = c * 4;
                predictor[c] = i16::from_le_bytes([block[base], block[base + 1]]) as i32;
                index[c] = block[base + 2] as i32;
                out.push(predictor[c] as i16);
            }
            let bytes_per_channel = 4;
            let frames_per_group = 8;
            let mut p = 4 * channels;
            let mut f = 1usize;
            while f < IMA_SAMPLES_PER_BLOCK {
                for c in 0..channels {
                    for _ in 0..bytes_per_channel {
                        let byte = block[p];
                        p += 1;
                        let lo = ima_decode(byte & 0x0f, &mut predictor[c], &mut index[c]);
                        let hi = ima_decode(byte >> 4, &mut predictor[c], &mut index[c]);
                        out.push(lo as i16);
                        out.push(hi as i16);
                    }
                }
                f += frames_per_group;
            }
        }
        out
    }

    fn snr_db(orig: &[i16], dec: &[i16]) -> f64 {
        let n = orig.len().min(dec.len());
        let mut sig = 0f64;
        let mut err = 0f64;
        for i in 0..n {
            let a = orig[i] as f64;
            let b = dec[i] as f64;
            sig += a * a;
            err += (a - b) * (a - b);
        }
        if err == 0.0 {
            return 999.0;
        }
        10.0 * (sig / err).log10()
    }

    #[test]
    fn adpcm_block_layout_is_exact() {
        for ch in [1usize, 2] {
            let frames = IMA_SAMPLES_PER_BLOCK * 2 + 137;
            let mut samples = vec![0i16; frames * ch];
            for i in 0..samples.len() {
                samples[i] = (((i as f32) * 0.01).sin() * 0.5 * 32767.0).round() as i16;
            }
            let data = encode_ima_data(&samples, ch);
            let block_align = if ch == 2 { 512 } else { 256 };
            assert_eq!(data.len() % block_align, 0, "data must be block aligned");
            assert_eq!(data.len(), (frames / IMA_SAMPLES_PER_BLOCK + 1) * block_align);
        }
    }

    #[test]
    fn adpcm_roundtrip_quality() {
        let sr = 48_000usize;
        let secs = 2usize;
        let mut samples = vec![0i16; sr * secs * 2];
        for i in 0..sr * secs {
            let t = i as f32 / sr as f32;
            // musical tone + a transient + quiet noise floor
            let music = (2.0 * std::f32::consts::PI * 220.0 * t).sin() * 0.45
                + (2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.20;
            let burst = if (i % 24_000) < 240 {
                ((i as f32) * 0.3).sin() * 0.6
            } else {
                0.0
            };
            let v = ((music + burst).clamp(-1.0, 1.0) * 32767.0).round() as i16;
            samples[i * 2] = v;
            samples[i * 2 + 1] = (v as f32 * 0.9).round() as i16;
        }
        let orig: Vec<i16> = samples.clone();
        let encoded = encode_ima_data(&samples, 2);
        let decoded = decode(&encoded, 2);
        // 4:1 compression vs PCM16
        let pcm16 = samples.len() * 2;
        assert!(encoded.len() < pcm16 / 3, "expected ~4:1, got {}", pcm16 / encoded.len());
        let snr = snr_db(&orig, &decoded);
        println!("adpcm: {} bytes vs {} pcm16 ({:.2}:1), snr {:.1} dB",
            encoded.len(), pcm16, pcm16 as f64 / encoded.len() as f64, snr);
        assert!(snr > 15.0, "adpcm snr too low: {snr:.1} dB");
    }

    #[test]
    #[ignore]
    fn dump_for_external_decoder() {
        let sr = 48_000u32;
        let n = sr as usize * 3;
        let mut samples = vec![0i16; n * 2];
        for i in 0..n {
            let t = i as f32 / sr as f32;
            let mut v = (2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.5;
            v += (2.0 * std::f32::consts::PI * 220.0 * t).sin() * 0.25;
            if (i / 12_000) % 2 == 0 { v *= 0.1; }
            let s = (v.clamp(-1.0, 1.0) * 32767.0).round() as i16;
            samples[i * 2] = s;
            samples[i * 2 + 1] = (s as f32 * 0.8).round() as i16;
        }
        let p = std::path::Path::new("D:\\ClipThis\\_codec_check.wav");
        write_ima_adpcm(p, &samples, sr, 2).unwrap();
        let pcm = std::path::Path::new("D:\\ClipThis\\_codec_check_pcm16.wav");
        write_wav_i16(pcm, &samples, sr, 2).unwrap();
        write_wav_f32(std::path::Path::new("D:\\ClipThis\\_codec_check_pcm32.wav"), &samples, sr, 2).unwrap();
    }

    #[test]
    fn adpcm_header_shape() {
        let dir = std::env::temp_dir();
        let p = dir.join("clipthis_adpcm_test.wav");
        let samples: Vec<i16> = (0..48_000 * 2).map(|i| (((i as f32) * 0.05).sin() * 0.4 * 32767.0).round() as i16).collect();
        write_ima_adpcm(&p, &samples, 48_000, 2).unwrap();
        let bytes = std::fs::read(&p).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(u16::from_le_bytes([bytes[20], bytes[21]]), 0x0011);
        assert_eq!(u16::from_le_bytes([bytes[32], bytes[33]]), 512); // block align
        assert_eq!(u16::from_le_bytes([bytes[34], bytes[35]]), 4); // bits
        let _ = std::fs::remove_file(&p);
    }
}
