//! Local Whisper models (ggml format from the whisper.cpp project).

use anyhow::{anyhow, Result};
use futures_util::StreamExt;
use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::AsyncWriteExt;

pub struct ModelInfo {
    pub name: &'static str,
    pub label: &'static str,
    pub size_mb: u32,
    pub note: &'static str,
}

pub const MODELS: &[ModelInfo] = &[
    ModelInfo { name: "tiny", label: "Tiny", size_mb: 75, note: "Fastest, rough accuracy" },
    ModelInfo { name: "base", label: "Base", size_mb: 142, note: "Fast, good for clear English" },
    ModelInfo { name: "small", label: "Small", size_mb: 466, note: "Better accuracy and Hindi" },
    ModelInfo { name: "large-v3-turbo-q5_0", label: "Large v3 Turbo", size_mb: 547, note: "Best accuracy, slower on CPU" },
];

#[derive(Serialize)]
pub struct ModelStatus {
    pub name: String,
    pub label: String,
    pub size_mb: u32,
    pub note: String,
    pub downloaded: bool,
}

pub fn dir(app: &AppHandle) -> PathBuf {
    app.path().app_data_dir().expect("no data dir").join("models")
}

pub fn path(app: &AppHandle, name: &str) -> PathBuf {
    dir(app).join(format!("ggml-{name}.bin"))
}

pub fn list(app: &AppHandle) -> Vec<ModelStatus> {
    MODELS
        .iter()
        .map(|m| ModelStatus {
            name: m.name.into(),
            label: m.label.into(),
            size_mb: m.size_mb,
            note: m.note.into(),
            downloaded: path(app, m.name).exists(),
        })
        .collect()
}

#[derive(Clone, Serialize)]
struct Progress<'a> {
    name: &'a str,
    downloaded: u64,
    total: u64,
}

pub async fn download(app: &AppHandle, http: &reqwest::Client, name: &str) -> Result<()> {
    if !MODELS.iter().any(|m| m.name == name) {
        return Err(anyhow!("unknown model {name}"));
    }
    let url = format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{name}.bin");
    let res = http.get(url).send().await?.error_for_status()?;
    let total = res.content_length().unwrap_or(0);
    std::fs::create_dir_all(dir(app))?;
    let final_path = path(app, name);
    let tmp = final_path.with_extension("part");
    let mut file = tokio::fs::File::create(&tmp).await?;
    let mut stream = res.bytes_stream();
    let mut downloaded = 0u64;
    let mut last_pct = u64::MAX;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;
        let pct = if total > 0 { downloaded * 100 / total } else { 0 };
        if pct != last_pct {
            last_pct = pct;
            let _ = app.emit("bolo://download", Progress { name, downloaded, total });
        }
    }
    file.flush().await?;
    drop(file);
    tokio::fs::rename(&tmp, &final_path).await?;
    Ok(())
}
