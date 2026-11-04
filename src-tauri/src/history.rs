use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use crate::settings::Settings;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Item {
    pub id: u64,
    pub text: String,
    pub raw: String,
    pub at: u64,
    pub engine: String,
    pub app: String,
    pub kind: crate::style::AppKind,
    pub command: bool,
    pub status: String,
    pub error: String,
    pub has_audio: bool,
    #[serde(skip)]
    pub audio: Vec<f32>,
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

pub struct Store { pub items: Vec<Item>, dir: PathBuf }

impl Store {
    pub fn new(dir: PathBuf, s: &Settings) -> Self {
        let items = if s.history_retention != "session" {
            std::fs::read(dir.join("history.json")).ok().and_then(|v| serde_json::from_slice(&v).ok()).unwrap_or_default()
        } else { Vec::new() };
        let mut store = Self { items, dir };
        for item in &mut store.items {
            item.has_audio = s.save_audio && store.dir.join(format!("{}.wav", item.id)).exists();
            if item.status == "processing" { item.status = "interrupted".into(); }
        }
        let _ = store.persist(s);
        store
    }

    pub fn audio(&self, id: u64) -> anyhow::Result<Vec<f32>> {
        let item = self.items.iter().find(|i| i.id == id).ok_or_else(|| anyhow::anyhow!("Dictation no longer available"))?;
        if !item.audio.is_empty() { return Ok(item.audio.clone()); }
        anyhow::ensure!(item.has_audio, "No recording saved for this dictation");
        let mut reader = hound::WavReader::open(self.dir.join(format!("{id}.wav")))?;
        Ok(reader.samples::<i16>().collect::<Result<Vec<_>,_>>()?.iter().map(|s| *s as f32 / i16::MAX as f32).collect())
    }

    pub fn persist(&mut self, s: &Settings) -> anyhow::Result<()> {
        let age = match s.history_retention.as_str() { "day" => 86_400_000, "week" => 604_800_000, _ => u64::MAX };
        let cutoff = now().saturating_sub(age);
        self.items.retain(|i| i.at >= cutoff);
        self.items.truncate(20);
        std::fs::create_dir_all(&self.dir)?;
        let persistent = s.history_retention != "session";
        for item in &mut self.items {
            let wav = self.dir.join(format!("{}.wav", item.id));
            if persistent && s.save_audio && !item.audio.is_empty() && !wav.exists() {
                write_wav(&wav, &item.audio)?;
            }
            item.has_audio = !item.audio.is_empty() || (persistent && s.save_audio && wav.exists());
        }
        // Only files owned by this history store are considered for retention.
        for entry in std::fs::read_dir(&self.dir)? {
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == "wav") {
                let id = path.file_stem().and_then(|x| x.to_str()).and_then(|x| x.parse::<u64>().ok());
                if id.is_some() && (!persistent || !s.save_audio || !self.items.iter().any(|i| Some(i.id) == id)) {
                    std::fs::remove_file(path)?;
                }
            }
        }
        let target = self.dir.join("history.json");
        if persistent {
            let tmp = self.dir.join("history.tmp");
            std::fs::write(&tmp, serde_json::to_vec(&self.items)?)?;
            std::fs::rename(tmp, target)?;
        } else if target.exists() { std::fs::remove_file(target)?; }
        Ok(())
    }
}

fn write_wav(path: &Path, audio: &[f32]) -> anyhow::Result<()> {
    let tmp = path.with_extension("tmp");
    let spec = hound::WavSpec { channels: 1, sample_rate: 16000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut writer = hound::WavWriter::create(&tmp, spec)?;
    for sample in audio { writer.write_sample((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)?; }
    writer.finalize()?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn dir(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join(".scratch").join(format!("history-{name}-{}-{}", std::process::id(), now()))
    }
    fn item(id: u64) -> Item { Item { id, at: now(), text: "Thank you".into(), audio: vec![0.0, 0.5, -0.5], status: "processing".into(), ..Default::default() } }
    #[test]
    fn persistent_audio_and_interrupted_work_recover() {
        let s = Settings { history_retention: "day".into(), save_audio: true, ..Default::default() };
        let mut store = Store::new(dir("recover"), &s);
        store.items.push(item(1)); store.persist(&s).unwrap();
        let loaded = Store::new(store.dir.clone(), &s);
        assert_eq!(loaded.items[0].text, "Thank you");
        assert_eq!(loaded.items[0].status, "interrupted");
        assert_eq!(loaded.audio(1).unwrap().len(), 3);
    }
    #[test]
    fn disabling_storage_removes_disk_data_but_keeps_session_text() {
        let mut s = Settings { history_retention: "week".into(), save_audio: true, ..Default::default() };
        let mut store = Store::new(dir("privacy"), &s);
        store.items.push(item(2)); store.persist(&s).unwrap();
        s.history_retention = "session".into(); store.persist(&s).unwrap();
        assert!(!store.dir.join("history.json").exists());
        assert!(!store.dir.join("2.wav").exists());
        assert_eq!(store.audio(2).unwrap().len(), 3);
        assert!(Store::new(store.dir.clone(), &s).items.is_empty());
    }
    #[test]
    fn expired_history_and_audio_are_pruned() {
        let s = Settings { history_retention: "day".into(), save_audio: true, ..Default::default() };
        let mut store = Store::new(dir("expiry"), &s);
        store.items.push(item(3)); store.persist(&s).unwrap();
        store.items[0].at = now() - 86_400_001;
        store.persist(&s).unwrap();
        assert!(store.items.is_empty()); assert!(!store.dir.join("3.wav").exists());
        assert!(store.audio(3).is_err());
    }
}
