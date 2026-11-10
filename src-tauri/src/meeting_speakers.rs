//! Local, post-meeting speaker clustering. Labels are estimates, never identities.
use anyhow::{Context,Result,ensure};
use std::{path::{Path,PathBuf},sync::atomic::{AtomicBool,Ordering}};
use tauri::{AppHandle,Manager};
use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;
use sha2::{Digest,Sha256};
static DOWNLOADING:AtomicBool=AtomicBool::new(false);
struct Guard;
impl Drop for Guard {fn drop(&mut self){DOWNLOADING.store(false,Ordering::Release);}}
const MODELS:[(&str,&str,u64,&str);2]=[
    ("segmentation.onnx","https://huggingface.co/csukuangfj/sherpa-onnx-pyannote-segmentation-3-0/resolve/9403a69/model.int8.onnx",1540506,"d582f4b4c6b48205de7e0643c57df0df5615a3c176189be3fc461e9d18827b5d"),
    ("embedding.onnx","https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx",39593761,"1a331345f04805badbb495c775a6ddffcdd1a732567d5ec8b3d5749e3c7a5e4b")
];
fn dir(app:&AppHandle)->Result<PathBuf>{Ok(app.path().app_data_dir()?.join("meeting-models"))}
fn hex(bytes:&[u8])->String {bytes.iter().map(|b|format!("{b:02x}")).collect()}
fn valid(path:&Path,size:u64,hash:&str)->bool {
    let Ok(meta)=std::fs::metadata(path) else{return false;};if meta.len()!=size{return false;}
    let Ok(bytes)=std::fs::read(path) else{return false;};hex(&Sha256::digest(bytes))==hash
}
#[tauri::command]
pub fn meeting_speaker_models(app:AppHandle)->bool {dir(&app).is_ok_and(|dir|MODELS.iter().all(|(name,_,size,hash)|valid(&dir.join(name),*size,hash)))}
#[tauri::command]
pub async fn meeting_download_speaker_models(app:AppHandle)->Result<(),String> {
    async {
        ensure!(cfg!(windows),"Meeting speaker labels currently require Windows");
        ensure!(!DOWNLOADING.swap(true,Ordering::AcqRel),"Speaker models are already downloading");let _guard=Guard;
        let dir=dir(&app)?;std::fs::create_dir_all(&dir)?;
        for (name,url,size,hash) in MODELS {
            let path=dir.join(name);if valid(&path,size,hash){continue;}
            let response=app.state::<crate::AppState>().http.get(url).timeout(std::time::Duration::from_secs(300)).send().await?.error_for_status()?;
            let tmp=path.with_extension("part");let mut file=tokio::fs::File::create(&tmp).await?;
            let mut stream=response.bytes_stream();let mut count=0;let mut digest=Sha256::new();
            while let Some(chunk)=stream.next().await {let chunk=chunk?;count+=chunk.len() as u64;ensure!(count<=size,"Speaker model download exceeded its expected size");digest.update(&chunk);file.write_all(&chunk).await?;}
            file.sync_all().await?;drop(file);
            ensure!(count==size && hex(&digest.finalize())==hash,"Speaker model verification failed. Please download again.");
            tokio::fs::rename(tmp,path).await?;
        }
        Ok::<_,anyhow::Error>(())
    }.await.map_err(|e|e.to_string())
}
#[derive(Clone,serde::Serialize,serde::Deserialize)]
pub struct Segment {pub start_ms:u64,pub end_ms:u64,pub speaker:String}
fn label(start:u64,end:u64,segments:&[Segment])->Option<String> {
    if end<=start{return None;}
    let mut labels=std::collections::BTreeSet::new();let mut duration=0;
    for s in segments {let overlap=end.min(s.end_ms).saturating_sub(start.max(s.start_ms));if overlap>=100 {labels.insert(s.speaker.clone());duration+=overlap;}}
    if duration<200 {None} else if labels.len()==1 {labels.into_iter().next()} else {Some("Multiple speakers".into())}
}
#[cfg(windows)]
pub fn analyze(models:&Path,audio:&Path)->Result<Vec<Segment>> {
    use sherpa_onnx::*;
    ensure!(MODELS.iter().all(|(name,_,size,hash)|valid(&models.join(name),*size,hash)),"Download the speaker-label models first (41 MB). Captions remain available.");
    let mut wav=hound::WavReader::open(audio)?;
    ensure!(wav.spec().sample_rate==16000 && wav.spec().channels==1 && wav.duration()<=16000*60*60+16000,"Unsupported or oversized meeting recording");
    let samples:Vec<f32>=wav.samples::<i16>().map(|s|s.map(|s|s as f32/32768.0)).collect::<std::result::Result<_,_>>()?;
    if samples.len()<1600 || !samples.iter().any(|v|v.abs()>0.001){return Ok(vec![]);}
    let config=OfflineSpeakerDiarizationConfig {
        segmentation:OfflineSpeakerSegmentationModelConfig {pyannote:OfflineSpeakerSegmentationPyannoteModelConfig {model:Some(models.join("segmentation.onnx").to_string_lossy().into_owned()),..Default::default()},num_threads:2,..Default::default()},
        embedding:SpeakerEmbeddingExtractorConfig {model:Some(models.join("embedding.onnx").to_string_lossy().into_owned()),num_threads:2,..Default::default()},
        ..Default::default()
    };
    let engine=OfflineSpeakerDiarization::create(&config).context("Could not load speaker-label models")?;
    let result=engine.process(&samples).context("Speaker labelling failed; captions are unchanged")?;
    let mut names=std::collections::HashMap::new();let mut segments=vec![];
    for s in result.sort_by_start_time() {
        if !s.start.is_finite()||!s.end.is_finite()||s.end<=s.start||s.start<0.0{continue;}
        let next=names.len()+1;let name=names.entry(s.speaker).or_insert_with(||format!("Speaker {next}"));
        segments.push(Segment {start_ms:(s.start*1000.0) as u64,end_ms:(s.end*1000.0) as u64,speaker:name.clone()});
    }
    Ok(segments)
}
#[cfg(not(windows))]
pub fn analyze(_models:&Path,_audio:&Path)->Result<Vec<Segment>> {anyhow::bail!("Speaker labelling currently requires Windows")}

pub fn finish(app:&AppHandle,c:&crate::meetings::Control)->Result<()> {
    let models=dir(app)?;
    let id=c.document.lock().unwrap_or_else(|e|e.into_inner()).id;
    let audio=app.state::<crate::meetings::State>().root.join(id.to_string()).join("computer.wav");
    let segments=analyze(&models,&audio)?;
    c.update(app,|d| {
        for turn in &mut d.turns {if turn.source=="computer" {turn.speaker=label(turn.start_ms,turn.end_ms,&segments);}}
        d.speakers=segments;
    });
    crate::meetings::persist(app,c)
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn ambiguous_turns_are_not_assigned_to_a_single_person() {
        let segments=vec![Segment{start_ms:0,end_ms:1000,speaker:"Speaker 1".into()},Segment{start_ms:1000,end_ms:2000,speaker:"Speaker 2".into()}];
        assert_eq!(label(0,900,&segments).as_deref(),Some("Speaker 1"));
        assert_eq!(label(500,1500,&segments).as_deref(),Some("Multiple speakers"));
        assert_eq!(label(2100,2500,&segments),None);
    }
    #[test] #[ignore="Local speaker model smoke test; no audio is uploaded"]
    fn local_speaker_models_load_and_process_speech() {
        let root=PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let audio=std::env::var("BOLO_DIARIZATION_EVAL_WAV").map(PathBuf::from).unwrap_or_else(|_|root.join("tests/fixtures/speech.wav"));
        let segments=analyze(&root.join("../.scratch/meeting-models"),&audio).unwrap();
        if std::env::var_os("BOLO_DIARIZATION_EVAL_WAV").is_some() {let speakers:std::collections::BTreeSet<_>=segments.iter().map(|s|s.speaker.clone()).collect();assert_eq!(speakers.len(),2,"Public two-speaker reference should have two clusters");}
        assert!(!segments.is_empty());assert!(segments.iter().all(|s|s.end_ms>s.start_ms));
    }
}
