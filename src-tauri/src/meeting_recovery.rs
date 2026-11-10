//! Resumable, explicit re-transcription of retained microphone/system WAVs.
use anyhow::{ensure,Context,Result};
use serde::{Serialize,Deserialize};
use std::{collections::HashMap,path::{Path,PathBuf},sync::{Arc,Mutex,LazyLock,atomic::{AtomicBool,Ordering}}};
use tauri::{AppHandle,Manager,Emitter};
use crate::meetings::{Document,Turn};
static JOBS:LazyLock<Mutex<HashMap<u64,(Arc<AtomicBool>,Option<u64>)>>>=LazyLock::new(||Mutex::new(HashMap::new()));
pub fn busy_document(id:u64)->bool {JOBS.lock().unwrap_or_else(|e|e.into_inner()).iter().any(|(source,(_,output))|*source==id||*output==Some(id))}
pub fn any_running()->bool {!JOBS.lock().unwrap_or_else(|e|e.into_inner()).is_empty()}
struct Job(u64);
impl Drop for Job {fn drop(&mut self){JOBS.lock().unwrap_or_else(|e|e.into_inner()).remove(&self.0);}}
#[derive(Clone,Serialize,Deserialize,PartialEq)]
struct Track {source:String,samples:u32,stamp:u128}
#[derive(Serialize,Deserialize)]
struct Progress {tracks:Vec<Track>,positions:Vec<u32>,document:Document}
#[derive(Serialize)]
pub struct Quote {pub estimated_micros:u64,pub remaining_seconds:u64,pub recovered_id:Option<u64>}
fn tracks(dir:&Path)->Result<Vec<Track>> {
 let mut tracks=vec![];
 for source in ["microphone","computer"] {
  let path=dir.join(format!("{source}.wav"));if !path.exists(){continue;}
  let reader=hound::WavReader::open(&path).context("Saved audio could not be opened")?;
  let spec=reader.spec();ensure!(spec.channels==1&&spec.sample_rate==16000&&spec.bits_per_sample==16&&spec.sample_format==hound::SampleFormat::Int,"Saved audio format is unsupported");
  ensure!(reader.duration()<=16000*61*60,"Saved audio exceeds the recovery length limit");
  let stamp=std::fs::metadata(&path)?.modified()?.duration_since(std::time::UNIX_EPOCH)?.as_nanos();
  tracks.push(Track {source:source.into(),samples:reader.duration(),stamp});
 }
 ensure!(!tracks.is_empty(),"No saved audio here. For a recovered transcript, open the original meeting to access its recordings.");Ok(tracks)
}
fn read_progress(path:&Path,current:&[Track])->Result<Option<Progress>> {
 if !path.exists(){return Ok(None);}
 ensure!(std::fs::metadata(path)?.len()<=16*1024*1024,"Recovery checkpoint is too large");
 let p:Progress=serde_json::from_slice(&std::fs::read(path)?).context("Recovery checkpoint could not be read. It has been kept.")?;
 ensure!(p.tracks==current&&p.positions.len()==current.len()&&p.positions.iter().zip(current).all(|(at,t)|*at<=t.samples),"Saved audio changed since recovery began; the checkpoint has been kept. No request was sent.");Ok(Some(p))
}
fn estimate(tracks:&[Track],positions:&[u32])->Quote {
 let samples:u64=tracks.iter().enumerate().map(|(i,t)|t.samples.saturating_sub(positions.get(i).copied().unwrap_or(0)) as u64).sum();
 // Bound rounding for chunks of at least 50 seconds, plus a final short chunk per track.
 let rounding=if samples==0{0}else{samples/(16000*50)+tracks.len() as u64};
 Quote {estimated_micros:if samples==0{0}else{crate::budget::speech_cost(samples as usize)+rounding},remaining_seconds:samples.div_ceil(16000),recovered_id:None}
}
fn checkpoint(root:&Path,original:u64,p:&Progress)->Result<()> {
 use std::io::Write;
 let dir=root.join(original.to_string());ensure!(dir.join("meeting.json").is_file(),"Original meeting was deleted; recovery stopped.");
 ensure!(p.document.id!=original&&p.document.recovered_from==Some(original),"Recovery checkpoint has an invalid destination");
 let destination=root.join(p.document.id.to_string()).join("meeting.json");
 if destination.exists(){
  let existing:Document=serde_json::from_slice(&std::fs::read(&destination)?)?;
  ensure!(existing.id==p.document.id&&existing.recovered_from==Some(original),"Recovery would overwrite an unrelated recording; nothing was replaced");
 } else {ensure!(!dir.join("recovery.json").exists(),"The recovered copy was deleted. Its checkpoint has been retained; it was not recreated");}
 let temp=dir.join("recovery.tmp");let mut f=std::fs::File::create(&temp)?;f.write_all(&serde_json::to_vec(p)?)?;f.sync_all()?;drop(f);
 // Save the result first; if checkpoint writing fails a retry may re-request only
 // the last chunk, but cannot lose the already saved transcript.
 crate::meetings::save(root,&p.document)?;
 std::fs::rename(temp,dir.join("recovery.json")).context("Could not save recovery progress")
}
fn chunk(path:PathBuf,position:u32)->Result<Vec<f32>> {
 let mut r=hound::WavReader::open(path)?;r.seek(position)?;
 let mut samples:Vec<f32>=r.samples::<i16>().take(16000*60).map(|v|v.map(|v|v as f32/32768.0)).collect::<std::result::Result<_,_>>()?;
 // Prefer a quiet boundary near the end instead of splitting a spoken word.
 if samples.len()==16000*60 {
  if let Some(at)=(16000*50..samples.len()-1600).step_by(1600).rev().find(|at|samples[*at..*at+1600].iter().map(|x|x*x).sum::<f32>()/1600.0<0.000004){samples.truncate(at+800);}
 }
 Ok(samples)
}
#[tauri::command]
pub fn meeting_recovery_quote(app:AppHandle,id:u64)->Result<Quote,String> {
 (||->Result<_>{
  let state=app.state::<crate::meetings::State>();let active=state.active.lock().unwrap_or_else(|e|e.into_inner());ensure!(active.is_none(),"Stop recording before recovering audio.");
  let _io=state.io.lock().unwrap_or_else(|e|e.into_inner());let dir=state.root.join(id.to_string());let tracks=tracks(&dir)?;
  let progress=read_progress(&dir.join("recovery.json"),&tracks)?;
  let mut quote=estimate(&tracks,progress.as_ref().map(|p|p.positions.as_slice()).unwrap_or(&[]));quote.recovered_id=progress.map(|p|p.document.id);Ok(quote)
 })().map_err(|e|e.to_string())
}
#[tauri::command]
pub fn meeting_recovery_cancel(id:u64){if let Some((stop,_))=JOBS.lock().unwrap_or_else(|e|e.into_inner()).get(&id){stop.store(true,Ordering::Release);}}
#[tauri::command]
pub async fn meeting_recover(app:AppHandle,id:u64,max_estimated_micros:u64)->Result<u64,String> {
 recover(app,id,max_estimated_micros).await.map_err(|e|format!("Recovery stopped: {e}. Original recordings and completed checkpoints are retained."))
}
async fn recover(app:AppHandle,id:u64,maximum:u64)->Result<u64> {
 let _request=crate::meeting_notes::RequestGuard::acquire(id)?;
 let state=app.state::<crate::meetings::State>();
 let stop=Arc::new(AtomicBool::new(false));JOBS.lock().unwrap_or_else(|e|e.into_inner()).insert(id,(stop.clone(),None));let _job=Job(id);
 let (mut progress,dir)={
  let active=state.active.lock().unwrap_or_else(|e|e.into_inner());ensure!(active.is_none(),"Stop recording first");
  let _io=state.io.lock().unwrap_or_else(|e|e.into_inner());
  let dir=state.root.join(id.to_string());let tracks=tracks(&dir)?;
  let existing=read_progress(&dir.join("recovery.json"),&tracks)?;
  let quote=estimate(&tracks,existing.as_ref().map(|p|p.positions.as_slice()).unwrap_or(&[]));ensure!(quote.estimated_micros<=maximum,"The estimate changed. Review the new estimate first");
  let budget=crate::budget::view()?;ensure!(quote.estimated_micros<=budget.limit_cents.saturating_mul(10000).saturating_sub(budget.used_micros),"Not enough monthly budget remains for the estimate");
  let p=match existing {Some(p)=>p,None=>{
   let original=crate::meetings::meeting_get(app.clone(),id).map_err(anyhow::Error::msg)?;
   let mut new_id=crate::history::now().max(id+1);while state.root.join(new_id.to_string()).exists(){new_id+=1;}
   let document=Document{id:new_id,title:format!("{} · recovered",original.title),status:"recovering".into(),error:format!("Original audio is in meeting {id}. Recovery timestamps are approximate; individual speakers are not inferred."),elapsed_ms:original.elapsed_ms,estimated_micros:0,turns:vec![],speakers:vec![],speaker_labels:original.speaker_labels.into_iter().filter(|(k,_)|k.starts_with("source:")).collect(),microphone_only:original.microphone_only,recovered_from:Some(id)};
   Progress{positions:vec![0;tracks.len()],tracks,document}
  }};
  JOBS.lock().unwrap_or_else(|e|e.into_inner()).get_mut(&id).context("Recovery state unavailable")?.1=Some(p.document.id);
  checkpoint(&state.root,id,&p)?;(p,dir)
 };
 let app_state=app.state::<crate::AppState>();let settings=app_state.settings.lock().unwrap_or_else(|e|e.into_inner()).clone();
 ensure!(!settings.openai_api_key.trim().is_empty(),"Add your OpenAI key first");
 let mut spent=0u64;
 for i in 0..progress.tracks.len(){
  while progress.positions[i]<progress.tracks[i].samples {
   ensure!(!stop.load(Ordering::Acquire),"Cancelled after the current request");
   ensure!(dir.join("meeting.json").is_file(),"Original meeting was deleted");
   let position=progress.positions[i];let path=dir.join(format!("{}.wav",progress.tracks[i].source));
   let audio=tauri::async_runtime::spawn_blocking(move||chunk(path,position)).await??;ensure!(!audio.is_empty(),"Unexpected end of saved audio");
   let count=audio.len();let voice=tauri::async_runtime::spawn_blocking(move||crate::audio::speech_audio(audio)).await?;
   if let Ok(audio)=voice {
    let cost=crate::budget::speech_cost(audio.len());ensure!(spent.saturating_add(cost)<=maximum,"Approved recovery estimate reached");spent+=cost;
    let text=crate::openai::transcribe(&app_state.http,&settings.openai_api_key,&audio,&settings.language,&settings.recognition_terms()).await?;
    progress.document.estimated_micros+=cost;
    progress.document.turns.push(Turn{id:format!("recovered:{}:{position}",progress.tracks[i].source),source:progress.tracks[i].source.clone(),start_ms:position as u64/16,end_ms:(position as u64+count as u64)/16,text,complete:true,speaker:None});
   }
   progress.positions[i]+=count as u32;
   {let _io=state.io.lock().unwrap_or_else(|e|e.into_inner());checkpoint(&state.root,id,&progress)?;}
   let remaining=estimate(&progress.tracks,&progress.positions).remaining_seconds;
   let _=app.emit("bolo://recovery",serde_json::json!({"id":id,"remaining_seconds":remaining}));
  }
 }
 progress.document.status="recovered".into();
 {let _io=state.io.lock().unwrap_or_else(|e|e.into_inner());checkpoint(&state.root,id,&progress)?;}
 Ok(progress.document.id)
}
#[cfg(test)] mod tests {
 use super::*;
 #[test] fn estimate_counts_only_remaining_audio_and_bounds_rounding(){
  let tracks=vec![Track{source:"microphone".into(),samples:16000*120,stamp:1},Track{source:"computer".into(),samples:16000*60,stamp:1}];
  assert_eq!(estimate(&tracks,&[16000*60,0]).remaining_seconds,120);
  assert!(estimate(&tracks,&[16000*60,0]).estimated_micros>=9000);
  assert_eq!(estimate(&tracks,&[16000*120,16000*60]).estimated_micros,0);
 }
 #[test] fn recovery_checkpoints_resume_and_never_overwrite_originals(){
  let root=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.scratch/recovery-verification");
  std::fs::create_dir_all(&root).unwrap();
  let original=Document{id:1,title:"Original".into(),status:"saved".into(),error:String::new(),elapsed_ms:1000,estimated_micros:0,turns:vec![],speakers:vec![],speaker_labels:Default::default(),microphone_only:true,recovered_from:None};
  crate::meetings::save(&root,&original).unwrap();
  let mut copy=original.clone();copy.id=2;copy.recovered_from=Some(1);copy.status="recovering".into();
  let p=Progress{tracks:vec![Track{source:"microphone".into(),samples:16000,stamp:1}],positions:vec![8000],document:copy};
  checkpoint(&root,1,&p).unwrap();
  let loaded=read_progress(&root.join("1/recovery.json"),&p.tracks).unwrap().unwrap();
  assert_eq!(loaded.positions,vec![8000]);
  let original_bytes=std::fs::read(root.join("1/meeting.json")).unwrap();
  let mut invalid=p;invalid.document.id=1;
  assert!(checkpoint(&root,1,&invalid).is_err());
  assert_eq!(std::fs::read(root.join("1/meeting.json")).unwrap(),original_bytes);
  invalid.document.id=2;
  let changed=vec![Track{source:"microphone".into(),samples:16000,stamp:2}];
  assert!(read_progress(&root.join("1/recovery.json"),&changed).is_err());
 }

}
