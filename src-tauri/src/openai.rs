//! OpenAI speech recognition. A single key also powers text cleanup.
use anyhow::{anyhow, Context, Result};

pub fn check_status(status: u16) -> Result<()> {
    match status {
        200..=299 => Ok(()),
        401 | 403 => Err(anyhow!("OpenAI rejected the key or project permissions. Check your API key in Settings.")),
        402 | 429 => Err(anyhow!("OpenAI API credit or rate limit reached. Nothing was retried. Check your API balance; your recording is safe in History.")),
        400 | 404 => Err(anyhow!("OpenAI could not accept this model or request. Your recording is safe in History.")),
        _ => Err(anyhow!("OpenAI request failed (HTTP {status}). Retry from History when the service is available.")),
    }
}

fn languages(language: &str) -> Result<Vec<&str>> {
    match language {
        // The live API rejects pa/pan. Do not restrict mixed speech to en/hi:
        // that would exclude the very Punjabi speech we need to preserve.
        "auto" | "pa" => Ok(vec![]),
        "en" | "hi" => Ok(vec![language]),
        _ => Err(anyhow!("Choose English, Hindi, Punjabi or Automatic / mixed speech")),
    }
}

fn response_text(body: &serde_json::Value) -> Result<String> {
    let text = body["text"].as_str().context("OpenAI returned no transcript")?.trim();
    anyhow::ensure!(!text.is_empty(), "No speech recognized. Nothing was pasted.");
    Ok(text.to_owned())
}

pub async fn transcribe(http: &reqwest::Client, key: &str, audio: &[f32], language: &str, terms: &[String]) -> Result<String> {
    anyhow::ensure!(!key.trim().is_empty(), "Add your OpenAI API key in Settings first");
    anyhow::ensure!(!audio.is_empty() && audio.iter().all(|v| v.is_finite()), "No valid audio to transcribe");
    let context = if language == "pa" {
        "Punjabi office dictation, sometimes containing English words. Punjabi words and grammar are preserved rather than normalized into Hindi."
    } else {
        "Office dictation switching between English, Hindi and Punjabi. Punjabi words and grammar are preserved rather than normalized into Hindi."
    };
    let file = reqwest::multipart::Part::bytes(crate::transcribe::wav_bytes(audio)?)
        .file_name("audio.wav").mime_str("audio/wav")?;
    let mut form = reqwest::multipart::Form::new().part("file", file)
        .text("model", "gpt-transcribe")
        .text("prompt", context);
    for language in languages(language)? { form = form.text("languages[]", language.to_owned()); }
    // Unsupported keyword characters reject the entire request; keep only valid hints.
    for term in terms.iter().map(|t| t.trim()).filter(|t| !t.is_empty() && !t.contains(['<', '>', '\r', '\n'])).take(100) {
        form = form.text("keywords[]", term.to_owned());
    }
    let _reservation = crate::budget::reserve(crate::budget::speech_cost(audio.len()))?;
    let res = http.post("https://api.openai.com/v1/audio/transcriptions")
        .bearer_auth(key).multipart(form).send().await
        .map_err(|_| anyhow!("Could not reach OpenAI. Check your connection and retry from History."))?;
    check_status(res.status().as_u16())?;
    let body = res.json().await.context("OpenAI returned an invalid transcript response")?;
    response_text(&body)
}


#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mixed_and_punjabi_modes_do_not_restrict_detection_to_hindi() {
        assert!(languages("auto").unwrap().is_empty());
        assert!(languages("pa").unwrap().is_empty());
        assert_eq!(languages("hi").unwrap(), vec!["hi"]);
        assert_eq!(languages("en").unwrap(), vec!["en"]);
        assert!(languages("invalid").is_err());
    }
    #[test]
    fn malformed_responses_are_rejected_and_punjabi_is_preserved() {
        for value in [serde_json::json!({}), serde_json::json!({"text": " "}), serde_json::json!({"text": 7})] {
            assert!(response_text(&value).is_err());
        }
        let raw = "ਕੱਲ੍ਹ ਮਿਲਦੇ ਹਾਂ, please send the report.";
        assert_eq!(response_text(&serde_json::json!({"text": raw})).unwrap(), raw);
        for code in [400,401,403,404,429,500] { assert!(check_status(code).is_err()); }
    }
    #[test]
    fn live_openai_speech_and_cleanup() {
        let Ok(key) = crate::settings::live_test_key() else { return; };
        let mut wav = hound::WavReader::new(std::io::Cursor::new(include_bytes!("../tests/fixtures/speech.wav"))).unwrap();
        let audio = wav.samples::<i16>().map(|v| v.unwrap() as f32 / 32768.0).collect();
        let audio = crate::audio::speech_audio(audio).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(60)).build().unwrap();
            let raw = transcribe(&http, &key, &audio, "auto", &[]).await.unwrap();
            assert!(raw.to_lowercase().contains("please send the report"), "Unexpected transcript: {raw}");
            let target = crate::cleanup::Target { base_url: "https://api.openai.com/v1", key: Some(&key), model: crate::settings::DEFAULT_OPENAI_MODEL };
            let out = crate::cleanup::run(&http, target, &raw, &[], true, None).await.unwrap();
            assert!(out.to_lowercase().contains("please send the report"), "Unexpected cleanup: {out}");
            eprintln!("Live OpenAI pipeline passed: {out}");
        });
    }
}
