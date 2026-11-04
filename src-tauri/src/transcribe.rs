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
        parts.push("Haan, main Hindi aur English mein baat karta hoon. Kal meeting ke baad report bhej dena.".into());
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

    /// Optional full Groq speech + cleanup check. Supply 16 kHz mono i16 WAVs
    /// named hindi.wav, mixed.wav and english.wav in BOLO_TEST_SPEECH_DIR.
    #[test]
    fn groq_speech_script_modes() {
        let (Ok(key), Ok(dir)) = (std::env::var("BOLO_TEST_GROQ_KEY"), std::env::var("BOLO_TEST_SPEECH_DIR")) else {
            eprintln!("skipped: BOLO_TEST_GROQ_KEY / BOLO_TEST_SPEECH_DIR not set");
            return;
        };
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let http = reqwest::Client::new();
            let target = crate::cleanup::Target {
                base_url: "https://api.groq.com/openai/v1", key: Some(&key),
                model: crate::settings::DEFAULT_GROQ_MODEL,
            };
            for name in ["hindi", "mixed", "english"] {
                let mut reader = hound::WavReader::open(Path::new(&dir).join(format!("{name}.wav"))).unwrap();
                assert_eq!(reader.spec().sample_rate, TARGET_RATE);
                assert_eq!(reader.spec().channels, 1);
                let audio: Vec<f32> = reader.samples::<i16>().map(|s| s.unwrap() as f32 / i16::MAX as f32).collect();
                for roman in [true, false] {
                    // The app leaves script conversion to cleanup when available.
                    let raw = groq(&http, &key, "whisper-large-v3-turbo", &audio, "auto", &[], false).await.unwrap();
                    let out = crate::cleanup::run(&http, target, &raw, &[], roman, None).await.unwrap();
                    eprintln!("{name} roman={roman}: raw={raw} -> {out}");
                    assert!(!out.is_empty());
                    let devanagari = out.chars().any(|c| ('\u{0900}'..='\u{097F}').contains(&c));
                    assert_eq!(devanagari, !roman && name != "english", "{out}");
                    if name == "mixed" {
                        assert!(out.to_lowercase().contains("report"), "{out}");
                    } else if name == "english" {
                        assert!(out.to_lowercase().contains("document") && out.to_lowercase().contains("friday"), "{out}");
                    }
                }
            }
        });
    }

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
}

/// Groq's hosted Whisper (OpenAI-compatible endpoint).
pub async fn groq(
    http: &reqwest::Client,
    key: &str,
    model: &str,
    audio: &[f32],
    language: &str,
    terms: &[String],
    hinglish_roman: bool,
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
    if let Some(p) = transcription_prompt(terms, hinglish_roman) {
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
