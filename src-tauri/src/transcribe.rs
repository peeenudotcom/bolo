use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::audio::TARGET_RATE;

pub type WhisperCache = Mutex<Option<(PathBuf, WhisperContext)>>;

fn transcription_prompt(terms: &[String], hinglish_roman: bool) -> Option<String> {
    let mut parts = Vec::new();
    if !terms.is_empty() {
        parts.push(format!("Vocabulary: {}.", terms.join(", ")));
    }
    // Whisper takes example transcript text, rather than instructions.
    if hinglish_roman {
        parts.push("Kal meri meeting hai, please send the report. Theek aa ji, kal milde haan.".into());
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// Whisper always encodes a 30 s window (1500 frames, 50 per second). For a
/// short dictation most of that is padding, so only encode what was spoken
/// plus a little headroom. Returns None when the full window is needed anyway.
fn audio_ctx_for(samples: usize) -> Option<i32> {
    let secs = samples as f32 / TARGET_RATE as f32;
    let frames = ((secs * 50.0).ceil() as i32 + 128 + 63) / 64 * 64;
    (frames < 1500).then_some(frames.max(256))
}

/// Runs whisper.cpp locally. Blocking; call from a blocking thread.
pub fn local(
    cache: &WhisperCache,
    model: &Path,
    audio: &[f32],
    language: &str,
    terms: &[String],
    hinglish_roman: bool,
) -> Result<String> {
    local_with(cache, model, audio, language, terms, hinglish_roman, true)
}

fn local_with(
    cache: &WhisperCache,
    model: &Path,
    audio: &[f32],
    language: &str,
    terms: &[String],
    hinglish_roman: bool,
    trim: bool,
) -> Result<String> {
    if audio.is_empty() { return Ok(String::new()); }
    anyhow::ensure!(!language.contains('\0') && terms.iter().all(|t| !t.contains('\0')),
        "Language or vocabulary contains an invalid null character. Remove it in Settings.");
    let mut guard = cache.lock().map_err(|_| anyhow!("Offline decoder failed. Restart Bolo and retry from History."))?;
    if guard.as_ref().map(|(p, _)| p.as_path()) != Some(model) {
        let mut cparams = WhisperContextParameters::default();
        cparams.flash_attn(true);
        let ctx = WhisperContext::new_with_params(model.to_str().context("bad model path")?, cparams)
        .map_err(|e| anyhow!("Could not load model: {e:?}"))?;
        *guard = Some((model.to_path_buf(), ctx));
    }
    let (_, ctx) = guard.as_ref().context("Offline model is unavailable")?;
    let mut state = ctx
        .create_state()
        .map_err(|e| anyhow!("whisper state: {e:?}"))?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(8) as i32;
    params.set_n_threads(threads);
    params.set_language(Some(language));
    params.set_translate(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_no_context(true);
    params.set_suppress_blank(true);
    if trim {
        if let Some(ctx) = audio_ctx_for(audio.len()) {
            params.set_audio_ctx(ctx);
        }
    }
    let prompt = transcription_prompt(terms, hinglish_roman);
    if let Some(p) = &prompt {
        params.set_initial_prompt(p);
    }

    // Whisper hallucinates on very short clips; pad to at least 1 second.
    let mut padded;
    let audio = if audio.len() < TARGET_RATE as usize {
        padded = audio.to_vec();
        padded.resize(TARGET_RATE as usize + 1600, 0.0);
        &padded[..]
    } else {
        audio
    };

    state
        .full(params, audio)
        .map_err(|e| anyhow!("transcription failed: {e:?}"))?;

    let mut text = String::new();
    for seg in state.as_iter() {
        text.push_str(&seg.to_string());
    }
    Ok(text.trim().to_string())
}

pub(crate) fn wav_bytes(audio: &[f32]) -> Result<Vec<u8>> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: TARGET_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cur = std::io::Cursor::new(Vec::new());
    {
        let mut w = hound::WavWriter::new(&mut cur, spec)?;
        for s in audio {
            w.write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)?;
        }
        w.finalize()?;
    }
    Ok(cur.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Set BOLO_TEST_MODEL (a ggml .bin) and BOLO_TEST_WAV (16 kHz mono) to run.
    #[test]
    fn transcribes_wav_locally() {
        let (Ok(model), Ok(wav)) = (std::env::var("BOLO_TEST_MODEL"), std::env::var("BOLO_TEST_WAV")) else {
            eprintln!("skipped: BOLO_TEST_MODEL / BOLO_TEST_WAV not set");
            return;
        };
        let mut reader = hound::WavReader::open(wav).unwrap();
        let audio: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / i16::MAX as f32)
            .collect();
        let cache = Mutex::new(None);
        let m = Path::new(&model);
        let _ = local(&cache, m, &audio, "en", &[], false).unwrap(); // load the model once
        let expect = std::env::var("BOLO_TEST_EXPECT").unwrap_or_else(|_| "meeting".into());
        for (lang, trim) in [("auto", false), ("auto", true), ("en", false), ("en", true)] {
            let start = std::time::Instant::now();
            let text = local_with(&cache, m, &audio, lang, &["Bolo".into()], false, trim).unwrap();
            eprintln!("lang={lang:<4} trimmed={trim:<5} {:>6.2}s  {text}", start.elapsed().as_secs_f32());
            assert!(text.to_lowercase().contains(&expect), "got: {text}");
        }
    }

    #[test]
    fn wav_roundtrip_has_header() {
        let bytes = wav_bytes(&[0.0, 0.5, -0.5]).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
    }

    #[test]
    fn empty_audio_does_not_load_a_model() {
        let cache = Mutex::new(None);
        assert_eq!(local(&cache, Path::new("missing-model"), &[], "auto", &[], false).unwrap(), "");
    }

    #[test]
    fn invalid_prompt_is_rejected_before_whisper_can_panic() {
        let cache = Mutex::new(None);
        for (language, terms) in [("en\0", vec![]), ("auto", vec!["bad\0word".into()])] {
            let error = local(&cache, Path::new("missing-model"), &[0.5], language, &terms, false).unwrap_err();
            assert!(error.to_string().contains("null character"));
        }
    }
}
