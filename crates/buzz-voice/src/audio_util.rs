//! Small audio helpers that avoid linking prebuilt C++ (sherpa-onnx).
//!
//! Windows MSVC toolsets older than the sherpa release used to fail LNK2001 on
//! `__std_find_end_*` when linking those prebuilts. Pocket TTS only needs a
//! mono WAV reader and a resampler — pure Rust is enough.

use std::fs::File;
use std::path::Path;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Load a mono (or first-channel) WAV/PCM file as f32 samples + sample rate.
pub fn read_wav_mono_f32(path: &Path) -> Result<(Vec<f32>, i32), String> {
    let file = File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("probe {}: {e}", path.display()))?;

    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| format!("no audio track in {}", path.display()))?
        .clone();
    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| format!("missing sample rate in {}", path.display()))? as i32;
    let channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(1);

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("decoder {}: {e}", path.display()))?;

    let mut samples = Vec::new();
    let track_id = track.id;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::ResetRequired) => continue,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => {
                // Symphonia ends many files with this once exhausted.
                if matches!(e, SymError::IoError(_)) {
                    break;
                }
                return Err(format!("read {}: {e}", path.display()));
            }
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = decoder
            .decode(&packet)
            .map_err(|e| format!("decode {}: {e}", path.display()))?;
        let spec = *decoded.spec();
        let mut buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buf.copy_interleaved_ref(decoded);
        let plane = buf.samples();
        if channels <= 1 {
            samples.extend_from_slice(plane);
        } else {
            // Keep left/first channel only.
            for frame in plane.chunks_exact(channels) {
                samples.push(frame[0]);
            }
        }
    }

    if samples.is_empty() {
        return Err(format!("voice WAV is empty: {}", path.display()));
    }
    Ok((samples, sample_rate))
}

/// Linear resampler (good enough for reference-voice conditioning).
pub fn resample_linear(samples: &[f32], from_hz: i32, to_hz: i32) -> Vec<f32> {
    if from_hz <= 0 || to_hz <= 0 || samples.is_empty() {
        return samples.to_vec();
    }
    if from_hz == to_hz {
        return samples.to_vec();
    }
    let ratio = to_hz as f64 / from_hz as f64;
    let out_len = ((samples.len() as f64) * ratio).round().max(1.0) as usize;
    let mut out = Vec::with_capacity(out_len);
    let last = samples.len() - 1;
    for i in 0..out_len {
        let src = i as f64 / ratio;
        let i0 = src.floor() as usize;
        let i1 = (i0 + 1).min(last);
        let t = (src - i0 as f64) as f32;
        let a = samples[i0.min(last)];
        let b = samples[i1];
        out.push(a + (b - a) * t);
    }
    out
}
