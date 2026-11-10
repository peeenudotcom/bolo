//! Online-meeting audio capture. Audio callbacks only enqueue bounded packets.
use anyhow::{Context, Result, bail, ensure};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::{path::PathBuf, sync::{Arc, atomic::{AtomicBool, Ordering}, mpsc}, time::{Duration, Instant}};
use tokio::sync::mpsc as async_channel;

pub struct Packet { pub start_ms: u64, pub samples: Vec<f32> }
struct Raw { at: u64, rate: u32, samples: Vec<f32> }

pub fn outputs() -> Result<Vec<String>> {
    ensure!(cfg!(windows), "Online meeting capture is currently available on Windows.");
    Ok(cpal::default_host().output_devices()?.filter_map(|d| d.description().ok().map(|d| d.name().to_owned())).collect())
}

/// Owns the non-Send stream on this worker. The caller joins this worker at stop.
pub fn record(output: bool, name: String, path: PathBuf, epoch: Instant,
    stop: Arc<AtomicBool>, ready: mpsc::Sender<Result<()>>, tx: async_channel::Sender<Packet>) -> Result<()> {
    let (raw_tx, raw_rx) = mpsc::sync_channel::<Raw>(128);
    let failed = Arc::new(AtomicBool::new(false));
    let setup = (|| -> Result<_> {
        ensure!(cfg!(windows), "Online meeting capture is currently available on Windows.");
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
        let writer = hound::WavWriter::create(path, hound::WavSpec { channels:1, sample_rate:16000,
            bits_per_sample:16, sample_format:hound::SampleFormat::Int })?;
        stream.play()?;
        Ok((stream,writer))
    })();
    let (stream, mut writer) = match setup { Ok(value) => { let _=ready.send(Ok(())); value }, Err(e) => {
        let _=ready.send(Err(anyhow::anyhow!(e.to_string()))); return Err(e);
    }};
    let mut written=0u64; let mut buffer=Vec::new(); let mut buffer_start=0;
    let mut flush=Instant::now();
    let result=(|| -> Result<()> {
        while !stop.load(Ordering::Acquire) {
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
                            tx.try_send(Packet {start_ms:buffer_start,samples:std::mem::take(&mut buffer)})
                                .map_err(|_| anyhow::anyhow!("Live transcription fell behind. Meeting stopped to avoid losing audio."))?;
                        }
                        for _ in written..desired { writer.write_sample(0i16)?; }
                        written=desired;
                    }
                    if buffer.is_empty() { buffer_start=written/16; }
                    for s in &samples { writer.write_sample((s.clamp(-1.0,1.0)*32767.0) as i16)?; }
                    written+=samples.len() as u64; buffer.extend(samples);
                    if buffer.len()>=3200 {
                        tx.try_send(Packet { start_ms:buffer_start,samples:std::mem::take(&mut buffer) })
                            .map_err(|_| anyhow::anyhow!("Live transcription fell behind. Meeting stopped to avoid losing audio."))?;
                    }
                },
                Err(mpsc::RecvTimeoutError::Timeout)=>{},
                Err(_)=>bail!("Meeting audio capture stopped unexpectedly"),
            }
            if flush.elapsed()>=Duration::from_secs(1) { writer.flush()?; flush=Instant::now(); }
        }
        Ok(())
    })();
    drop(stream);
    // Finish queued callback packets, so Stop does not discard the final words.
    for raw in raw_rx.try_iter() {
        let samples=crate::audio::resample(&raw.samples,raw.rate,16000);
        if buffer.is_empty() { buffer_start=written/16; }
        for s in &samples { writer.write_sample((s.clamp(-1.0,1.0)*32767.0) as i16)?; }
        written+=samples.len() as u64; buffer.extend(samples);
    }
    let tail = if !buffer.is_empty() { tx.try_send(Packet {start_ms:buffer_start,samples:buffer}).map_err(|_|anyhow::anyhow!("Final audio could not reach live transcription. The recording has been saved.")) } else {Ok(())};
    writer.finalize().context("Could not finish saving meeting audio")?;
    result?; tail
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
