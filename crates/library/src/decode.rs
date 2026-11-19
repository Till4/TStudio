use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use tstudio_dsp::SampleData;

/// Duration in seconds from container metadata, without decoding (fast).
pub fn probe_duration(path: &Path) -> Option<f64> {
    let file = std::fs::File::open(path).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe().format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default()).ok()?;
    let track = probed.format.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL)?;
    let frames = track.codec_params.n_frames?;
    let sr = track.codec_params.sample_rate?;
    Some(frames as f64 / sr as f64)
}

/// Decodes wav/mp3/flac/ogg/aiff/m4a into de-interleaved stereo f32.
pub fn decode_file(path: &Path) -> Result<SampleData, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("Format nicht erkannt: {e}"))?;
    let mut format = probed.format;
    let track = format.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL).ok_or("keine Audiospur gefunden")?;
    let track_id = track.id;
    let mut sr = track.codec_params.sample_rate.unwrap_or(44100);
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("Codec nicht unterstützt: {e}"))?;

    let mut left = Vec::new();
    let mut right = Vec::new();
    let mut buf: Option<SampleBuffer<f32>> = None;
    let mut buf_frames = 0usize;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(Error::IoError(_)) | Err(Error::ResetRequired) => break,
            Err(e) => {
                if left.is_empty() {
                    return Err(e.to_string());
                }
                break;
            }
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                let spec = *decoded.spec();
                sr = spec.rate;
                let ch = spec.channels.count().max(1);
                let frames = decoded.capacity();
                if buf.is_none() || buf_frames < frames {
                    buf = Some(SampleBuffer::<f32>::new(frames as u64, spec));
                    buf_frames = frames;
                }
                let b = buf.as_mut().expect("buffer");
                b.copy_interleaved_ref(decoded);
                for frame in b.samples().chunks(ch) {
                    left.push(frame[0]);
                    right.push(if ch > 1 { frame[1] } else { frame[0] });
                }
            }
            Err(Error::DecodeError(_)) => continue,
            Err(Error::IoError(_)) => break,
            Err(e) => return Err(e.to_string()),
        }
    }
    if left.is_empty() {
        return Err("Datei enthält keine Audiodaten".into());
    }
    Ok(SampleData::new(left, right, sr))
}
