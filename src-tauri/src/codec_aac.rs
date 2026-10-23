//! AAC (M4A) encoder via Media Foundation.
//!
//! Produces a file the modern Windows Media Player can play, at a bitrate
//! comparable to or better than ADPCM, with much better quality (AAC).

use std::path::Path;
use windows::core::PCWSTR;
use windows::Win32::Media::MediaFoundation::{
    MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample, MFCreateSinkWriterFromURL,
    MFShutdown, MFStartup, IMFSample, IMFMediaBuffer, IMFSinkWriter, MF_MT_AAC_PAYLOAD_TYPE,
    MF_MT_AUDIO_AVG_BYTES_PER_SECOND, MF_MT_AUDIO_BITS_PER_SAMPLE, MF_MT_AUDIO_BLOCK_ALIGNMENT,
    MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE,
    MF_VERSION, MFSTARTUP_FULL, MFAudioFormat_AAC, MFAudioFormat_PCM, MFMediaType_Audio,
};

pub fn write_aac(path: &Path, samples: &[i16], sr: u32, ch: u16) -> Result<(), String> {
    if samples.is_empty() {
        // still create an empty-ish file? MF needs data; write nothing and return
        return Ok(());
    }
    unsafe {
        MFStartup(MF_VERSION, MFSTARTUP_FULL).map_err(|e| e.to_string())?;

        let result = encode(path, samples, sr, ch);

        let _ = MFShutdown();
        result
    }
}

unsafe fn encode(path: &Path, samples: &[i16], sr: u32, ch: u16) -> Result<(), String> {
    // wide path
    let wide: Vec<u16> = path.as_os_str().to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();

    let sink_writer: IMFSinkWriter = MFCreateSinkWriterFromURL(PCWSTR(wide.as_ptr()), None, None)
        .map_err(|e| e.to_string())?;

    // input media type: PCM
    let input_type = MFCreateMediaType().map_err(|e| e.to_string())?;
    input_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio).map_err(|e| e.to_string())?;
    input_type.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM).map_err(|e| e.to_string())?;
    input_type.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, ch as u32).map_err(|e| e.to_string())?;
    input_type.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, sr).map_err(|e| e.to_string())?;
    input_type.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16).map_err(|e| e.to_string())?;
    input_type.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, ch as u32 * 2).map_err(|e| e.to_string())?;
    input_type.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, sr * ch as u32 * 2).map_err(|e| e.to_string())?;

    // output media type: AAC
    let output_type = MFCreateMediaType().map_err(|e| e.to_string())?;
    output_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio).map_err(|e| e.to_string())?;
    output_type.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC).map_err(|e| e.to_string())?;
    output_type.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, ch as u32).map_err(|e| e.to_string())?;
    output_type.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, sr).map_err(|e| e.to_string())?;
    output_type.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16).map_err(|e| e.to_string())?;
    // raw AAC payload type 0
    let _ = output_type.SetUINT32(&MF_MT_AAC_PAYLOAD_TYPE, 0);

    // add stream
    let stream_index = sink_writer.AddStream(&output_type).map_err(|e| e.to_string())?;
    sink_writer.SetInputMediaType(stream_index, &input_type, None).map_err(|e| e.to_string())?;
    sink_writer.BeginWriting().map_err(|e| e.to_string())?;

    let channels = ch as usize;
    let frames_per_chunk = 1024usize;
    let block_align = channels * 2;

    let mut offset = 0usize; // frame index
    let total_frames = samples.len() / channels;

    while offset < total_frames {
        let n = frames_per_chunk.min(total_frames - offset);
        let byte_len = n * block_align;

        let sample: IMFSample = MFCreateSample().map_err(|e| e.to_string())?;
        let buffer: IMFMediaBuffer = MFCreateMemoryBuffer(byte_len as u32).map_err(|e| e.to_string())?;

        // lock buffer and copy PCM
        let mut ptr: *mut u8 = std::ptr::null_mut();
        let mut max_len: u32 = 0;
        let mut cur_len: u32 = 0;
        buffer.Lock(&mut ptr, Some(&mut max_len), Some(&mut cur_len)).map_err(|e| e.to_string())?;

        let src = &samples[offset * channels..(offset + n) * channels];
        let dst = std::slice::from_raw_parts_mut(ptr, byte_len);
        for (i, s) in src.iter().enumerate() {
            let bytes = s.to_le_bytes();
            dst[i * 2] = bytes[0];
            dst[i * 2 + 1] = bytes[1];
        }
        buffer.Unlock().map_err(|e| e.to_string())?;
        buffer.SetCurrentLength(byte_len as u32).map_err(|e| e.to_string())?;

        sample.AddBuffer(&buffer).map_err(|e| e.to_string())?;

        // timestamps in 100ns units
        let time = (offset as i64 * 10_000_000) / sr as i64;
        let duration = (n as i64 * 10_000_000) / sr as i64;
        sample.SetSampleTime(time).map_err(|e| e.to_string())?;
        sample.SetSampleDuration(duration).map_err(|e| e.to_string())?;

        sink_writer.WriteSample(stream_index, &sample).map_err(|e| e.to_string())?;

        offset += n;
    }

    sink_writer.Finalize().map_err(|e| e.to_string())?;
    Ok(())
}