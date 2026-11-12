//! Online-meeting audio capture. Audio callbacks only enqueue bounded packets.
use anyhow::{Context, Result, bail, ensure};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::{path::PathBuf, sync::{Arc, atomic::{AtomicBool, Ordering}, mpsc}, time::{Duration, Instant}};
use tokio::sync::mpsc as async_channel;

pub struct Packet { pub start_ms: u64, pub samples: Vec<f32> }
struct Raw { at: u64, rate: u32, samples: Vec<f32> }

/// Things the recorder tells the meeting screen without stopping.
#[derive(Default)]
pub struct Notices { pub caption_gap: AtomicBool, pub device_switched: AtomicBool }

/// Live captions are best-effort: if they can't keep up (network stall) or
/// have stopped, drop the packet for captions but never stop recording.
fn offer(tx:&async_channel::Sender<Packet>,packet:Packet,notices:&Notices) {
    if let Err(async_channel::error::TrySendError::Full(_))=tx.try_send(packet) {
        notices.caption_gap.store(true,Ordering::Release);
    }
}

fn default_device_name(output:bool)->Option<String> {
    let host=cpal::default_host();
    let device=if output {host.default_output_device()} else {host.default_input_device()}?;
    device.description().ok().map(|d|d.name().to_owned())
}

pub fn outputs() -> Result<Vec<String>> {
    ensure!(cfg!(any(windows, target_os="macos")), "Online meeting capture requires Windows or macOS.");
    #[cfg(target_os="macos")]
    return Ok(vec![]);
    #[cfg(not(target_os="macos"))]
    Ok(cpal::default_host().output_devices()?.filter_map(|d| d.description().ok().map(|d| d.name().to_owned())).collect())
}

/// Owns the non-Send stream on this worker. The caller joins this worker at stop.
pub fn record(output: bool, name: String, path: PathBuf, epoch: Instant,
    stop: Arc<AtomicBool>, ready: mpsc::Sender<Result<()>>, tx: async_channel::Sender<Packet>, notices: Arc<Notices>) -> Result<()> {
    let (raw_tx, raw_rx) = mpsc::sync_channel::<Raw>(128);
    let failed = Arc::new(AtomicBool::new(false));
    // With no device picked, follow Windows' default device: plugging in
    // headphones mid-meeting used to record silence for the other side, and
    // unplugging a device ended the recording. (macOS system audio is
    // captured separately and doesn't depend on the output device.)
    let follow_default = name.is_empty() && !(cfg!(target_os="macos") && output);
    let mut current_default = if follow_default { default_device_name(output) } else { None };
    let mut checked = Instant::now();
    let reopen_tx = raw_tx.clone();
    let setup = (|| -> Result<_> {
        ensure!(cfg!(any(windows, target_os="macos")), "Online meeting capture requires Windows or macOS.");
        let capture = start_capture(output, name, epoch, raw_tx, failed.clone())?;
        let writer = hound::WavWriter::create(path, hound::WavSpec { channels:1, sample_rate:16000,
            bits_per_sample:16, sample_format:hound::SampleFormat::Int })?;
        Ok((capture,writer))
    })();
    let (mut stream, mut writer) = match setup { Ok(value) => { let _=ready.send(Ok(())); value }, Err(e) => {
        let _=ready.send(Err(anyhow::anyhow!(e.to_string()))); return Err(e);
    }};
    let mut written=0u64; let mut buffer=Vec::new(); let mut buffer_start=0;
    let mut flush=Instant::now();
    let result=(|| -> Result<()> {
        while !stop.load(Ordering::Acquire) {
            let device_changed = follow_default && checked.elapsed() >= Duration::from_secs(2) && {
                checked = Instant::now();
                let now = default_device_name(output);
                now.is_some() && now != current_default
            };
            if follow_default && (device_changed || failed.load(Ordering::Acquire)) {
                let _ = stream.stop();
                failed.store(false, Ordering::Release);
                stream = start_capture(output, String::new(), epoch, reopen_tx.clone(), failed.clone())
                    .context("The audio device disconnected and no other device was available. Recording stopped; saved audio remains available.")?;
                current_default = default_device_name(output);
                notices.device_switched.store(true, Ordering::Release);
            }
            ensure!(!failed.load(Ordering::Acquire), "Meeting audio disconnected or capture fell behind. Recording stopped; saved audio remains available.");
            match raw_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(raw) => {
                    let samples=crate::audio::resample(&raw.samples,raw.rate,16000);
                    ensure!(samples.iter().all(|v| v.is_finite()), "Invalid meeting audio received");
                    let desired=raw.at*16;
                    // Loopback produces no packets when the output device is silent.
                    // Preserve those gaps in the recording, rather than collapsing time.
                    if desired > written+1600 {
                        if !buffer.is_empty() {
                            offer(&tx, Packet {start_ms:buffer_start,samples:std::mem::take(&mut buffer)}, &notices);
                        }
                        for _ in written..desired { writer.write_sample(0i16)?; }
                        written=desired;
                    }
                    if buffer.is_empty() { buffer_start=written/16; }
                    for s in &samples { writer.write_sample((s.clamp(-1.0,1.0)*32767.0) as i16)?; }
                    written+=samples.len() as u64; buffer.extend(samples);
                    if buffer.len()>=3200 {
                        offer(&tx, Packet { start_ms:buffer_start,samples:std::mem::take(&mut buffer) }, &notices);
                    }
                },
                Err(mpsc::RecvTimeoutError::Timeout)=>{},
                Err(_)=>bail!("Meeting audio capture stopped unexpectedly"),
            }
            if flush.elapsed()>=Duration::from_secs(1) { writer.flush()?; flush=Instant::now(); }
        }
        Ok(())
    })();
    let stopped=stream.stop();
    drop(stream);
    // Finish queued callback packets, so Stop does not discard the final words.
    for raw in raw_rx.try_iter() {
        let samples=crate::audio::resample(&raw.samples,raw.rate,16000);
        if buffer.is_empty() { buffer_start=written/16; }
        for s in &samples { writer.write_sample((s.clamp(-1.0,1.0)*32767.0) as i16)?; }
        written+=samples.len() as u64; buffer.extend(samples);
    }
    if !buffer.is_empty() { offer(&tx, Packet {start_ms:buffer_start,samples:buffer}, &notices); }
    writer.finalize().context("Could not finish saving meeting audio")?;
    result?; stopped
}

fn stream<T>(device:&cpal::Device, config:cpal::StreamConfig, channels:usize, rate:u32,
    epoch:Instant, tx:mpsc::SyncSender<Raw>, failed:Arc<AtomicBool>) -> Result<cpal::Stream>
where T:cpal::SizedSample, f32:cpal::FromSample<T> {
    let error=failed.clone();
    Ok(device.build_input_stream(config, move |data:&[T],_| {
        let samples:Vec<f32>=data.chunks_exact(channels).map(|frame| frame.iter().map(|s|s.to_sample::<f32>()).sum::<f32>()/channels as f32).collect();
        let duration=samples.len() as u64*1000/rate as u64;
        let at=(epoch.elapsed().as_millis() as u64).saturating_sub(duration);
        if tx.try_send(Raw {at,rate,samples}).is_err() { failed.store(true,Ordering::Release); }
    }, move |_| {error.store(true,Ordering::Release);},None)?)
}

#[cfg(target_os="macos")]
#[path="meeting_audio_macos.rs"]
mod macos;

enum Capture {
    Microphone(cpal::Stream),
    #[cfg(target_os="macos")]
    System(screencapturekit::stream::sc_stream::SCStream),
}
impl Capture {
    fn stop(&self)->Result<()> {
        match self {
            Self::Microphone(stream)=>{stream.pause()?;Ok(())},
            #[cfg(target_os="macos")]
            Self::System(stream)=>{stream.stop_capture()?;Ok(())},
        }
    }
}
fn start_capture(output:bool,name:String,epoch:Instant,raw_tx:mpsc::SyncSender<Raw>,failed:Arc<AtomicBool>)->Result<Capture> {
    #[cfg(target_os="macos")]
    if output {return macos::start(epoch,raw_tx,failed).map(Capture::System);}
    let host = cpal::default_host();
    let device = if name.is_empty() {
        if output { host.default_output_device() } else { host.default_input_device() }
    } else {
        let mut devices = if output { host.output_devices()? } else { host.input_devices()? };
        devices.find(|d| d.description().is_ok_and(|d| d.name() == name))
    }.context("Selected meeting audio device is unavailable. Refresh devices and try again.")?;
    let config = if output { device.default_output_config()? } else { device.default_input_config()? };
    let rate = config.sample_rate(); let channels = config.channels() as usize;
    ensure!(rate > 0 && channels > 0, "Invalid meeting audio format");
    let format = config.sample_format(); let config: cpal::StreamConfig = config.into();
    // On Windows cpal enables WASAPI loopback when an output device is opened as input.
    let stream = match format {
        cpal::SampleFormat::F32 => stream::<f32>(&device, config, channels, rate, epoch, raw_tx, failed.clone())?,
        cpal::SampleFormat::I16 => stream::<i16>(&device, config, channels, rate, epoch, raw_tx, failed.clone())?,
        cpal::SampleFormat::U16 => stream::<u16>(&device, config, channels, rate, epoch, raw_tx, failed.clone())?,
        cpal::SampleFormat::I32 => stream::<i32>(&device, config, channels, rate, epoch, raw_tx, failed.clone())?,
        _ => bail!("Unsupported meeting audio format"),
    };
    stream.play()?;
    Ok(Capture::Microphone(stream))
}
