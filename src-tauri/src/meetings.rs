//! Meeting sessions are independent of dictation and never paste into another app.
use anyhow::{Context, Result, ensure};
use serde::{Serialize, Deserialize};
use std::{path::{Path,PathBuf}, sync::{Arc,Mutex,atomic::{AtomicBool,Ordering}},time::{Duration,Instant}};
use tauri::{AppHandle,Emitter,Manager};

#[derive(Clone,Serialize,Deserialize)]
pub struct Turn { pub id:String, pub source:String, pub start_ms:u64, pub end_ms:u64, pub text:String, pub complete:bool, #[serde(default)] pub speaker:Option<String> }
#[derive(Clone,Serialize,Deserialize)]
pub struct Document { pub id:u64, pub title:String, pub status:String, pub error:String, pub elapsed_ms:u64, pub estimated_micros:u64, pub turns:Vec<Turn>, #[serde(default)] pub speakers:Vec<crate::meeting_speakers::Segment>, #[serde(default)] pub speaker_labels:std::collections::BTreeMap<String,String>, #[serde(default)] pub microphone_only:bool, #[serde(default)] pub recovered_from:Option<u64> }
pub struct Control { pub stop:Arc<AtomicBool>, pub document:Mutex<Document> }
pub struct State { pub active:Mutex<Option<Arc<Control>>>, pub root:PathBuf, pub(crate) io:Mutex<()> }
impl State {pub fn new(root:PathBuf)->Self {Self {active:Mutex::new(None),root,io:Mutex::new(())}}}
impl Control {
    fn emit(&self,app:&AppHandle) { let d=self.document.lock().unwrap_or_else(|e|e.into_inner()).clone(); let _=app.emit("bolo://meeting",d); }
    pub fn update(&self,app:&AppHandle,f:impl FnOnce(&mut Document)) { f(&mut self.document.lock().unwrap_or_else(|e|e.into_inner())); self.emit(app); }
}
pub(crate) fn save(root:&Path,d:&Document)->Result<()> {
    use std::io::Write;
    let dir=root.join(d.id.to_string()); std::fs::create_dir_all(&dir)?;
    let path=dir.join("meeting.json"); let tmp=dir.join("meeting.tmp");
    let mut file=std::fs::File::create(&tmp)?; file.write_all(&serde_json::to_vec(d)?)?; file.sync_all()?; drop(file);
    std::fs::rename(tmp,path).context("Could not save meeting transcript")
}
pub fn persist(app:&AppHandle,c:&Control)->Result<()> {
    let state=app.state::<State>(); let _io=state.io.lock().unwrap_or_else(|e|e.into_inner());
    let d=c.document.lock().unwrap_or_else(|e|e.into_inner()).clone(); save(&state.root,&d)
}
#[tauri::command]
pub fn meeting_devices()->Result<Vec<String>,String> {crate::meeting_audio::outputs().map_err(|e|e.to_string())}
#[tauri::command]
pub fn meeting_current(app:AppHandle)->Option<Document> {
    let active=app.state::<State>().active.lock().unwrap_or_else(|e|e.into_inner()).clone();
    active.map(|c|c.document.lock().unwrap_or_else(|e|e.into_inner()).clone())
}
#[tauri::command]
pub fn meeting_list(app:AppHandle)->Result<Vec<Document>,String> {
    let state=app.state::<State>(); let active=state.active.lock().unwrap_or_else(|e|e.into_inner()).clone();
    let active_id=active.map(|c|c.document.lock().unwrap_or_else(|e|e.into_inner()).id);
    if !state.root.exists() {return Ok(vec![]);}
    let mut docs=vec![];
    for entry in std::fs::read_dir(&state.root).map_err(|e|e.to_string())? {
        let entry=entry.map_err(|e|e.to_string())?;
        if entry.file_name().to_string_lossy().parse::<u64>().is_err() {continue;}
        let id=entry.file_name().to_string_lossy().parse::<u64>().map_err(|e|e.to_string())?;
        if Some(id)==active_id {continue;}
        let mut doc=meeting_get(app.clone(),id).unwrap_or_else(|error|Document {
            id,title:"Unreadable meeting".into(),status:"failed".into(),error,elapsed_ms:0,estimated_micros:0,turns:vec![],speakers:vec![],speaker_labels:Default::default(),microphone_only:false,recovered_from:None
        });
        // The sidebar only needs metadata. Load transcript content on selection.
        doc.turns.clear();doc.speakers.clear();
        docs.push(doc);
    }
    docs.sort_by_key(|d|std::cmp::Reverse(d.id)); Ok(docs)
}
#[tauri::command]
pub fn meeting_stop(app:AppHandle)->Result<(),String> {
    let active=app.state::<State>().active.lock().unwrap_or_else(|e|e.into_inner()).clone();
    if let Some(c)=active {c.stop.store(true,Ordering::Release);c.update(&app,|d|d.status="stopping".into());}
    Ok(())
}
#[tauri::command]
pub fn meeting_delete(app:AppHandle,id:u64)->Result<(),String> {
    let state=app.state::<State>(); let active=state.active.lock().unwrap_or_else(|e|e.into_inner());
    if active.as_ref().is_some_and(|c|c.document.lock().unwrap_or_else(|e|e.into_inner()).id==id) {return Err("Stop the meeting before deleting it".into());}
    let _io=state.io.lock().unwrap_or_else(|e|e.into_inner());
    if crate::meeting_recovery::busy_document(id){return Err("Cancel recovery and wait for it to finish before deleting this recording.".into());}
    let path=state.root.join(id.to_string());
    std::fs::remove_dir_all(path).map_err(|e|e.to_string())
}
#[tauri::command]
pub async fn meeting_start(app:AppHandle,title:String,output:String,microphone_only:Option<bool>)->Result<(),String> {
    let result=(||->Result<_> {
        ensure!(cfg!(any(windows,target_os="macos")),"Online meetings are supported on Windows and macOS.");
        let state=app.state::<crate::AppState>();
        let settings=state.settings.lock().unwrap_or_else(|e|e.into_inner()).clone();
        ensure!(!settings.openai_api_key.trim().is_empty(),"Add your OpenAI key in Settings first.");
        let budget=crate::budget::view()?; ensure!(budget.used_micros < budget.limit_cents.saturating_mul(10_000),"Monthly Bolo budget reached. Change the limit in Settings before starting live captions.");
        let phase=state.phase.lock().unwrap_or_else(|e|e.into_inner());
        ensure!(matches!(*phase,crate::Phase::Idle),"Finish dictation before starting a meeting.");
        ensure!(state.mic_test.lock().unwrap_or_else(|e|e.into_inner()).is_none(),"Stop the microphone test first.");
        let meetings=app.state::<State>(); let mut active=meetings.active.lock().unwrap_or_else(|e|e.into_inner());
        ensure!(active.is_none(),"A meeting is already active.");
        ensure!(!crate::meeting_recovery::any_running(),"Finish or cancel recovery before starting a recording.");
        let id=crate::history::now();
        let microphone_only=microphone_only.unwrap_or(false);
        let title=if microphone_only&&title.trim().is_empty(){"Long dictation".into()}else{title};
        let c=Arc::new(Control {stop:Arc::new(AtomicBool::new(false)), document:Mutex::new(Document {id,title:title.trim().chars().take(100).collect(),status:"connecting".into(),error:String::new(),elapsed_ms:0,estimated_micros:0,turns:vec![],speakers:vec![],speaker_labels:Default::default(),microphone_only,recovered_from:None})});
        save(&meetings.root,&c.document.lock().unwrap_or_else(|e|e.into_inner()))?;
        *active=Some(c.clone()); Ok((c,settings))
    })().map_err(|e|e.to_string())?;
    let (control,settings)=result;
    control.emit(&app);
    tauri::async_runtime::spawn(async move {
        let result=run(&app,control.clone(),settings,output).await;
        control.stop.store(true,Ordering::Release);
        control.update(&app,|d| {d.status=if result.is_ok(){"saved"}else{"failed"}.into();if let Err(e)=result {d.error=e.to_string();}});
        if let Err(e)=persist(&app,&control) {control.update(&app,|d| {d.status="failed".into();d.error=e.to_string();});}
        *app.state::<State>().active.lock().unwrap_or_else(|e|e.into_inner())=None;
        let _=app.emit("bolo://meetings-changed",());
    });
    Ok(())
}
async fn run(app:&AppHandle,c:Arc<Control>,settings:crate::settings::Settings,output:String)->Result<()> {
    // Connect both streams before starting capture. Failed authentication must not record indefinitely.
    let mut mic=crate::meeting_live::connect(&settings.openai_api_key,&settings.recognition_terms()).await?;
    let microphone_only=c.document.lock().unwrap_or_else(|e|e.into_inner()).microphone_only;
    let mut sources=vec![("microphone",settings.microphone.clone(),mic.take().context("Missing microphone session")?)];
    if !microphone_only {let mut remote=crate::meeting_live::connect(&settings.openai_api_key,&settings.recognition_terms()).await?;sources.push(("computer",output,remote.take().context("Missing meeting session")?));}
    if c.stop.load(Ordering::Acquire) {return Ok(());}
    let id=c.document.lock().unwrap_or_else(|e|e.into_inner()).id;
    let dir=app.state::<State>().root.join(id.to_string());
    let epoch=Instant::now(); let mut workers=vec![]; let mut tasks=vec![];
    for (source,name,socket) in sources {
        let (tx,rx)=tokio::sync::mpsc::channel(40);
        let (ready_tx,ready_rx)=std::sync::mpsc::channel();
        let stop=c.stop.clone(); let path=dir.join(format!("{source}.wav"));
        let worker=std::thread::spawn(move ||crate::meeting_audio::record(source=="computer",name,path,epoch,stop,ready_tx,tx));
        workers.push(worker);
        let ready=tauri::async_runtime::spawn_blocking(move||ready_rx.recv_timeout(Duration::from_secs(10))).await?;
        match ready {Ok(Ok(()))=>{}, other=>{
            c.stop.store(true,Ordering::Release);
            for worker in workers {let _=tauri::async_runtime::spawn_blocking(move||worker.join()).await;}
            for task in tasks {let _=task.await;}
            anyhow::bail!("Meeting audio could not start: {}",match other {Ok(Err(e))=>e.to_string(),_=>"Audio device timed out".into()});
        }}
        let app=app.clone();let control=c.clone();
        tasks.push(tauri::async_runtime::spawn(async move {let r=crate::meeting_live::transcribe(&app,control.clone(),source,socket,rx).await;if r.is_err(){control.stop.store(true,Ordering::Release);}r}));
    }
    c.update(app,|d|d.status="recording".into());
    while !c.stop.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let ms=epoch.elapsed().as_millis() as u64;
        c.update(app,|d|d.elapsed_ms=ms);
        if ms>=60*60*1000 {c.stop.store(true,Ordering::Release);}
        if workers.iter().any(|w|w.is_finished()) {c.stop.store(true,Ordering::Release);}
    }
    c.update(app,|d|d.status="stopping".into());
    let mut failure=None;
    for worker in workers {
        match tauri::async_runtime::spawn_blocking(move||worker.join()).await {
            Ok(Ok(Ok(())))=>{},Ok(Ok(Err(e)))=>failure=Some(e),_=>failure=Some(anyhow::anyhow!("Meeting capture worker stopped unexpectedly")),
        }
    }
    for task in tasks {match task.await {Ok(Ok(()))=>{},Ok(Err(e))=>failure=Some(e),Err(_)=>failure=Some(anyhow::anyhow!("Meeting transcription worker stopped unexpectedly"))}}
        if let Some(e)=failure {return Err(e);}
    if !microphone_only && crate::meeting_speakers::meeting_speaker_models(app.clone()) {
        c.update(app,|d|d.status="labelling".into());
        let app=app.clone();let control=c.clone();
        tauri::async_runtime::spawn_blocking(move||crate::meeting_speakers::finish(&app,&control)).await??;
    }
    Ok(())
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn documents_preserve_mixed_text_and_provisional_status() {
        let d=Document {id:123,title:"Meeting".into(),status:"recording".into(),error:String::new(),elapsed_ms:1234,estimated_micros:30,
            turns:vec![Turn {id:"a".into(),source:"computer".into(),start_ms:20,end_ms:100,text:"ਕੱਲ੍ਹ ਮਿਲਦੇ ਹਾਂ, report भेज देना".into(),complete:false,speaker:None}],speakers:vec![],speaker_labels:Default::default(),microphone_only:false,recovered_from:None};
        let parsed:Document=serde_json::from_slice(&serde_json::to_vec(&d).unwrap()).unwrap();
        let mut named=d.clone();named.speaker_labels.insert("source:computer".into(),"Priya".into());
        assert_eq!(label(&named,&named.turns[0]).map(String::as_str),Some("Priya"));
        named.speaker_labels.insert("turn:a".into(),"Alex".into());assert_eq!(label(&named,&named.turns[0]).map(String::as_str),Some("Alex"));
        assert!(named.turns[0].speaker.is_none());
        assert_eq!(parsed.turns[0].text,d.turns[0].text);assert!(!parsed.turns[0].complete);assert!(parsed.turns[0].speaker.is_none());
    }
}

#[tauri::command]
pub fn meeting_show_files(app:AppHandle,id:u64)->Result<(),String> {
    use tauri_plugin_opener::OpenerExt;
    let path=app.state::<State>().root.join(id.to_string()).join("meeting.json");
    if !path.is_file(){return Err("Meeting files are unavailable".into());}
    app.opener().reveal_item_in_dir(path).map_err(|e|e.to_string())
}

#[tauri::command]
pub fn meeting_get(app:AppHandle,id:u64)->Result<Document,String> {
    let path=app.state::<State>().root.join(id.to_string()).join("meeting.json");
    let meta=std::fs::metadata(&path).map_err(|_|"This meeting could not be opened. Its files have been kept.".to_string())?;
    if meta.len()>16*1024*1024 {return Err("This meeting transcript is too large to open safely.".into());}
    let bytes=std::fs::read(path).map_err(|e|e.to_string())?;
    let mut d:Document=serde_json::from_slice(&bytes).map_err(|_|"This meeting transcript is damaged. Open its saved recordings to recover the audio.".to_string())?;
    if d.id!=id{return Err("Meeting record identifier does not match its folder".into());}
    if d.status=="recovering"&&!crate::meeting_recovery::busy_document(id){d.status="interrupted".into();d.error.push_str(" Recovery is incomplete; resume it from the original recording.");}
    if matches!(d.status.as_str(),"connecting"|"recording"|"stopping"|"labelling") {
        d.status="interrupted".into();d.error="Bolo closed before this meeting finished. Saved audio and captions are retained.".into();
    }
    Ok(d)
}


pub fn label<'a>(d:&'a Document,t:&Turn)->Option<&'a String> {
    d.speaker_labels.get(&format!("turn:{}",t.id)).or_else(||d.speaker_labels.get(&t.speaker.as_ref().map(|s|format!("speaker:{s}")).unwrap_or_else(||format!("source:{}",t.source))))
}
#[tauri::command]
pub fn meeting_label_speaker(app:AppHandle,id:u64,target:String,name:String)->Result<Document,String> {
    let _request=crate::meeting_notes::RequestGuard::acquire(id).map_err(|e|e.to_string())?;
    let state=app.state::<State>();
    let active=state.active.lock().unwrap_or_else(|e|e.into_inner());
    if active.as_ref().is_some_and(|c|c.document.lock().unwrap_or_else(|e|e.into_inner()).id==id){return Err("Stop and save before correcting speakers.".into());}
    let _io=state.io.lock().unwrap_or_else(|e|e.into_inner());
    if crate::meeting_recovery::busy_document(id){return Err("Finish recovery before changing speaker labels.".into());}
    let mut doc=meeting_get(app.clone(),id)?;
    let name=name.trim();
    if name.chars().count()>80||name.chars().any(char::is_control){return Err("Use a name of up to 80 characters, without line breaks.".into());}
    if !doc.turns.iter().any(|t|target==format!("turn:{}",t.id)||target==format!("source:{}",t.source)||t.speaker.as_ref().is_some_and(|s|target==format!("speaker:{s}"))){return Err("That speaker or turn is unavailable.".into());}
    if name.is_empty(){doc.speaker_labels.remove(&target);}else{doc.speaker_labels.insert(target,name.into());}
    save(&state.root,&doc).map_err(|e|e.to_string())?;Ok(doc)
}
