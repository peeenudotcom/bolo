use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::audio::TARGET_RATE;

pub type WhisperCache = Mutex<Option<(PathBuf, WhisperContext)>>;

fn vocab_prompt(terms: &[String]) -> Option<String> {
    if terms.is_empty() {
        None
    } else {
        Some(format!("Vocabulary: {}.", terms.join(", ")))
    }
}

/// Runs whisper.cpp locally. Blocking; call from a blocking thread.
pub fn local(
    cache: &WhisperCache,
    model: &Path,
    audio: &[f32],
    language: &str,
    terms: &[String],
) -> Result<String> {
    let mut guard = cache.lock().unwrap();
    if guard.as_ref().map(|(p, _)| p.as_path()) != Some(model) {
        let mut cparams = WhisperContextParameters::default();
        cparams.flash_attn(true);
        let ctx = WhisperContext::new_with_params(model.to_str().context("bad model path")?, cparams)
        .map_err(|e| anyhow!("Could not load model: {e:?}"))?;
        *guard = Some((model.to_path_buf(), ctx));
    }
    let (_, ctx) = guard.as_ref().unwrap();
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
    let prompt = vocab_prompt(terms);
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

fn wav_bytes(audio: &[f32]) -> Result<Vec<u8>> {
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
        let start = std::time::Instant::now();
        let text = local(&cache, Path::new(&model), &audio, "auto", &["Bolo".into()]).unwrap();
        eprintln!("transcript ({:?}): {text}", start.elapsed());
        let start = std::time::Instant::now();
        let _ = local(&cache, Path::new(&model), &audio, "en", &[]).unwrap();
        eprintln!("second run, model cached, language fixed: {:?}", start.elapsed());
        assert!(text.to_lowercase().contains("meeting"), "got: {text}");
    }

    #[test]
    fn wav_roundtrip_has_header() {
        let bytes = wav_bytes(&[0.0, 0.5, -0.5]).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
    }
}

/// Groq's hosted Whisper (OpenAI-compatible endpoint).
pub async fn groq(
    http: &reqwest::Client,
    key: &str,
    model: &str,
    audio: &[f32],
    language: &str,
    terms: &[String],
) -> Result<String> {
    let part = reqwest::multipart::Part::bytes(wav_bytes(audio)?)
        .file_name("audio.wav")
        .mime_str("audio/wav")?;
    let mut form = reqwest::multipart::Form::new()
        .part("file", part)
        .text("model", model.to_string())
        .text("response_format", "json")
        .text("temperature", "0");
    if language != "auto" {
        form = form.text("language", language.to_string());
    }
    if let Some(p) = vocab_prompt(terms) {
        form = form.text("prompt", p);
    }
    let res = http
        .post("https://api.groq.com/openai/v1/audio/transcriptions")
        .bearer_auth(key)
        .multipart(form)
        .send()
        .await?;
    let status = res.status();
    let body: serde_json::Value = res.json().await.unwrap_or_default();
    if !status.is_success() {
        let msg = body["error"]["message"].as_str().unwrap_or("request failed");
        return Err(anyhow!("Groq {status}: {msg}"));
    }
    Ok(body["text"].as_str().unwrap_or_default().trim().to_string())
}
