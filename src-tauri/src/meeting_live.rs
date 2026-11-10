//! Streaming captions over the documented OpenAI transcription WebSocket.
use anyhow::{Context,Result,ensure,bail};
use base64::Engine;
use futures_util::{SinkExt,StreamExt};
use serde_json::{Value,json};
use std::{collections::{HashMap,VecDeque},sync::Arc,time::Duration};
use tauri::AppHandle;
use tokio_tungstenite::{WebSocketStream,MaybeTlsStream,tungstenite::{Message,client::IntoClientRequest}};
use crate::{meeting_audio::Packet,meetings::{Control,Turn}};
type Socket=WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
const WAIT:Duration=Duration::from_secs(12);
pub async fn connect(key:&str,terms:&[String])->Result<Option<Socket>> {
    let mut request="wss://api.openai.com/v1/realtime?intent=transcription".into_client_request()?;
    request.headers_mut().insert("Authorization",format!("Bearer {}",key.trim()).parse().context("Invalid OpenAI key format")?);
    let (mut socket,_)=tokio::time::timeout(WAIT,tokio_tungstenite::connect_async(request)).await
        .context("OpenAI live connection timed out")?.map_err(|_|anyhow::anyhow!("Could not connect to OpenAI live transcription. Check your key, network and account access."))?;
    let keywords:Vec<_>=terms.iter().filter(|t|!t.contains(['<','>','\r','\n'])).take(100).collect();
    send(&mut socket,json!({"type":"session.update","session":{"type":"transcription","audio":{"input":{
        "format":{"type":"audio/pcm","rate":24000},"transcription":{"model":"gpt-live-transcribe","delay":"medium",
        "prompt":"An office meeting in English, Hindi and Punjabi, including mixed speech. Transcribe the words spoken, preserving the language. Do not translate or answer questions.","keywords":keywords},"turn_detection":null}}}})).await?;
    tokio::time::timeout(WAIT,async {
        while let Some(message)=socket.next().await {
            let message=message.map_err(|_|anyhow::anyhow!("OpenAI live connection failed"))?;
            if let Message::Text(text)=message {
                let event:Value=serde_json::from_str(&text)?;
                match event["type"].as_str() {Some("session.updated")=>return Ok(()),Some("error")=>bail!("OpenAI rejected the meeting transcription settings. Check model access and your API key."),_=>{}}
            }
        }
        bail!("OpenAI closed the live connection before recording started")
    }).await.context("OpenAI live setup timed out")??;
    Ok(Some(socket))
}
async fn send(socket:&mut Socket,event:Value)->Result<()> {
    tokio::time::timeout(WAIT,socket.send(Message::Text(event.to_string().into()))).await.context("Live upload timed out")?
        .map_err(|_|anyhow::anyhow!("Live connection interrupted. Saved meeting audio has been kept."))
}
fn cost(samples:usize)->u64 {((samples as u64)*17000).div_ceil(24000*60)}

#[derive(Default)]
struct Events { pending:VecDeque<(u64,u64)>, ranges:HashMap<String,(u64,u64)>, completed:std::collections::HashSet<String> }
impl Events {
    fn apply(&mut self,event:&Value,current:(u64,u64),source:&str,turns:&mut Vec<Turn>)->Result<bool> {
        let kind=event["type"].as_str().unwrap_or("");
        if matches!(kind,"error"|"conversation.item.input_audio_transcription.failed") {bail!("OpenAI could not transcribe part of this meeting. Saved audio and earlier text have been kept.");}
        let id=event["item_id"].as_str().unwrap_or("");
        if id.is_empty(){return Ok(false);}
        if kind=="input_audio_buffer.committed" {
            let range=self.pending.pop_front().context("Unexpected meeting turn acknowledgement")?;
            self.ranges.insert(id.into(),range);
            if let Some(turn)=turns.iter_mut().find(|t|t.id==format!("{source}:{id}")) {turn.start_ms=range.0;turn.end_ms=range.1;}
            return Ok(false);
        }
        if !matches!(kind,"conversation.item.input_audio_transcription.delta"|"conversation.item.input_audio_transcription.completed") {return Ok(false);}
        let complete=kind.ends_with(".completed");
        let text=event[if complete{"transcript"}else{"delta"}].as_str().unwrap_or("");
        ensure!(text.len()<=128_000,"Meeting transcript event was too large");
        let range=*self.ranges.entry(id.into()).or_insert_with(||self.pending.front().copied().unwrap_or(current));
        let key=format!("{source}:{id}");
        let index=if let Some(index)=turns.iter().position(|t|t.id==key){index}else{
            ensure!(turns.len()<10_000,"Meeting transcript reached its size limit");
            turns.push(Turn {id:key,source:source.into(),start_ms:range.0,end_ms:range.1,text:String::new(),complete:false,speaker:None});turns.len()-1
        };
        let turn=&mut turns[index];
        if complete {turn.text=text.into();turn.complete=true;self.completed.insert(id.into());}
        else if !turn.complete {ensure!(turn.text.len()+text.len()<=128_000,"Meeting turn was too large");turn.text.push_str(text);}
        Ok(complete)
    }
}
pub async fn transcribe(app:&AppHandle,c:Arc<Control>,source:&str,mut socket:Socket,mut rx:tokio::sync::mpsc::Receiver<Packet>)->Result<()> {
    let mut events=Events::default(); let mut detector=earshot::Detector::default();
    let mut preroll=VecDeque::<Packet>::new();let mut active=false;let mut start=0;let mut end=0;let mut last_voice=0;
    let mut closing=None;let mut expected=0usize;
    let mut tick=tokio::time::interval(Duration::from_millis(200));
    loop {
        tokio::select! {
            packet=rx.recv(),if closing.is_none()=>{
                match packet {
                    Some(packet)=>{
                        let voiced=packet.samples.chunks(256).map(|chunk| {
                            let mut frame=[0.0;256]; for (o,i) in frame.iter_mut().zip(chunk){*o=i.clamp(-1.0,1.0);}
                            detector.predict_f32(&frame)>=0.7
                        }).filter(|voiced|*voiced).count()>=3;
                        end=packet.start_ms+packet.samples.len() as u64/16;
                        if voiced {last_voice=end;}
                        preroll.push_back(packet);
                        if !active && !voiced {
                            while preroll.len()>3 {preroll.pop_front();}
                            continue;
                        }
                        if !active {start=preroll.front().map(|p|p.start_ms).unwrap_or(end);active=true;}
                        while let Some(packet)=preroll.pop_front() {
                            let pcm=crate::audio::resample(&packet.samples,16000,24000);
                            let amount=cost(pcm.len());
                            // Reserve durably before sending any audio. Failed sends keep their charge.
                            crate::budget::reserve(amount)?;
                            c.update(app,|d|d.estimated_micros=d.estimated_micros.saturating_add(amount));
                            let bytes:Vec<u8>=pcm.iter().flat_map(|s|((s.clamp(-1.0,1.0)*32767.0) as i16).to_le_bytes()).collect();
                            send(&mut socket,json!({"type":"input_audio_buffer.append","audio":base64::engine::general_purpose::STANDARD.encode(bytes)})).await?;
                        }
                        if end.saturating_sub(last_voice)>=800 || end.saturating_sub(start)>=15000 {
                            events.pending.push_back((start,end));expected+=1;
                            send(&mut socket,json!({"type":"input_audio_buffer.commit"})).await?;active=false;
                        }
                    },
                    None=>{
                        if active {events.pending.push_back((start,end));expected+=1;send(&mut socket,json!({"type":"input_audio_buffer.commit"})).await?;}
                        closing=Some(tokio::time::Instant::now());
                    }
                }
            },
            message=socket.next()=>{
                match message {
                    Some(Ok(Message::Text(text)))=>{
                        let event:Value=serde_json::from_str(&text).context("Invalid live transcription response")?;
                        let final_event={let mut d=c.document.lock().unwrap_or_else(|e|e.into_inner());events.apply(&event,(start,end),source,&mut d.turns)?};
                        c.update(app, |_|{});
                        if final_event {crate::meetings::persist(app,&c)?;}
                    },
                    Some(Ok(Message::Ping(data)))=>{socket.send(Message::Pong(data)).await.map_err(|_|anyhow::anyhow!("Meeting connection closed"))?;},
                    Some(Ok(Message::Close(_)))|None|Some(Err(_))=>bail!("OpenAI live connection ended. The transcript may be incomplete; saved audio has been kept."),
                    _=>{}
                }
            },
            _=tick.tick()=>{
                if active && end.saturating_sub(start)>=100 && last_voice+1000 < c.document.lock().unwrap_or_else(|e|e.into_inner()).elapsed_ms {
                    // Output loopback can stop producing packets during silence.
                    events.pending.push_back((start,end));expected+=1;
                    send(&mut socket,json!({"type":"input_audio_buffer.commit"})).await?;active=false;
                }
                if let Some(at)=closing {
                    if events.completed.len()>=expected {break;}
                    ensure!(at.elapsed()<Duration::from_secs(15),"Final meeting captions timed out. Earlier captions and audio are saved.");
                }
            }
        }
    }
    let _=tokio::time::timeout(Duration::from_secs(2),socket.close(None)).await;
    Ok(())
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn out_of_order_completions_keep_their_timestamps_and_sources() {
        let mut e=Events::default();let mut turns=vec![];
        e.pending.extend([(100,900),(1000,1900)]);
        for id in ["a","b"] {e.apply(&json!({"type":"input_audio_buffer.committed","item_id":id}),(0,0),"computer",&mut turns).unwrap();}
        for (id,text) in [("b","second"),("a","first")] {e.apply(&json!({"type":"conversation.item.input_audio_transcription.completed","item_id":id,"transcript":text}),(0,0),"computer",&mut turns).unwrap();}
        assert_eq!(turns[0].start_ms,1000);assert_eq!(turns[1].start_ms,100);
        e.apply(&json!({"type":"conversation.item.input_audio_transcription.delta","item_id":"a","delta":"duplicate"}),(0,0),"computer",&mut turns).unwrap();
        assert_eq!(turns[1].text,"first");assert!(turns.iter().all(|t|t.speaker.is_none()));
    }
    #[test] fn final_replaces_partial_and_error_is_visible() {
        let mut e=Events::default();let mut turns=vec![];
        e.apply(&json!({"type":"conversation.item.input_audio_transcription.delta","item_id":"a","delta":"Kal"}),(20,200),"microphone",&mut turns).unwrap();
        e.apply(&json!({"type":"conversation.item.input_audio_transcription.completed","item_id":"a","transcript":"Kal milde haan."}),(20,300),"microphone",&mut turns).unwrap();
        assert_eq!(turns.len(),1);assert_eq!(turns[0].text,"Kal milde haan.");
        assert!(e.apply(&json!({"type":"error"}),(0,0),"microphone",&mut turns).is_err());
        assert_eq!(cost(24000*60),17000);assert_eq!(cost(24000*60)*2,34000);
    }
}

#[cfg(test)] mod live_tests {
    use super::*;
    #[tokio::test]
    #[ignore="Opt-in OpenAI fixture test; reserves real Bolo budget before uploading"]
    async fn meeting_live_fixture() {
        let key=crate::settings::live_test_key().expect("Live test key must be available");
        let ledger=std::env::var("BOLO_EVAL_BUDGET_FILE").expect("Provide the actual Bolo budget file");
        crate::budget::init(std::path::PathBuf::from(ledger));
        let mut wav=hound::WavReader::new(std::io::Cursor::new(include_bytes!("../tests/fixtures/speech.wav"))).unwrap();
        let samples:Vec<f32>=wav.samples::<i16>().map(|s|s.unwrap() as f32/32768.0).collect();
        let pcm=crate::audio::resample(&samples,16000,24000);
        crate::budget::reserve(cost(pcm.len())).unwrap();
        let mut socket=connect(&key,&[]).await.unwrap().unwrap();
        for packet in pcm.chunks(4800) {
            let bytes:Vec<u8>=packet.iter().flat_map(|s|((s.clamp(-1.0,1.0)*32767.0) as i16).to_le_bytes()).collect();
            send(&mut socket,json!({"type":"input_audio_buffer.append","audio":base64::engine::general_purpose::STANDARD.encode(bytes)})).await.unwrap();
        }
        send(&mut socket,json!({"type":"input_audio_buffer.commit"})).await.unwrap();
        let text=tokio::time::timeout(Duration::from_secs(20),async {
            while let Some(message)=socket.next().await {
                if let Message::Text(text)=message.unwrap() {
                    let e:Value=serde_json::from_str(&text).unwrap();
                    if e["type"]=="error" {panic!("Live API rejected the fixture request");}
                    if e["type"]=="conversation.item.input_audio_transcription.completed" {return e["transcript"].as_str().unwrap_or("").to_string();}
                }
            }
            String::new()
        }).await.unwrap();
        assert!(text.split_whitespace().count()>=3,"Fixture transcription was empty or incomplete");
        socket.close(None).await.unwrap();
    }
}
