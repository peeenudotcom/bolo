//! Microphone capture. cpal streams are not `Send`, so each recording lives on
//! its own thread and is stopped through a channel.

use anyhow::{anyhow, bail, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

pub const TARGET_RATE: u32 = 16_000;

pub struct Recorder {
    stop_tx: Option<mpsc::Sender<()>>,
    handle: Option<thread::JoinHandle<()>>,
    samples: Arc<Mutex<Vec<f32>>>,
    rate: Arc<Mutex<u32>>,
}

impl Recorder {
    pub fn start(app: AppHandle) -> Result<Self> {
        let samples = Arc::new(Mutex::new(Vec::with_capacity(48_000 * 30)));
        let rate = Arc::new(Mutex::new(TARGET_RATE));
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<()>>();
        let (s2, r2) = (samples.clone(), rate.clone());

        let handle = thread::spawn(move || {
            let stream = match open_stream(app, s2, r2) {
                Ok(s) => s,
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            if let Err(e) = stream.play() {
                let _ = ready_tx.send(Err(e.into()));
                return;
            }
            let _ = ready_tx.send(Ok(()));
            let _ = stop_rx.recv();
            drop(stream);
        });

        ready_rx
            .recv()
            .map_err(|_| anyhow!("audio thread exited"))??;
        Ok(Self {
            stop_tx: Some(stop_tx),
            handle: Some(handle),
            samples,
            rate,
        })
    }

    /// Stops recording and returns 16 kHz mono samples.
    pub fn stop(mut self) -> Vec<f32> {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(());
        }
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        let raw = std::mem::take(&mut *self.samples.lock().unwrap());
        let rate = *self.rate.lock().unwrap();
        resample(&raw, rate, TARGET_RATE)
    }
}

struct Sink {
    samples: Arc<Mutex<Vec<f32>>>,
    app: AppHandle,
    last_emit: Instant,
    acc_sq: f32,
    acc_n: u32,
}

impl Sink {
    fn push(&mut self, mono: impl Iterator<Item = f32>) {
        {
            let mut buf = self.samples.lock().unwrap();
            for s in mono {
                buf.push(s);
                self.acc_sq += s * s;
                self.acc_n += 1;
            }
        }
        if self.last_emit.elapsed() >= Duration::from_millis(50) && self.acc_n > 0 {
            let rms = (self.acc_sq / self.acc_n as f32).sqrt();
            let _ = self.app.emit("bolo://level", rms);
            self.acc_sq = 0.0;
            self.acc_n = 0;
            self.last_emit = Instant::now();
        }
    }
}

fn open_stream(
    app: AppHandle,
    samples: Arc<Mutex<Vec<f32>>>,
    rate: Arc<Mutex<u32>>,
) -> Result<cpal::Stream> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow!("No microphone found"))?;
    let config = device.default_input_config()?;
    let channels = config.channels() as usize;
    *rate.lock().unwrap() = sample_rate_of(&config);
    let format = config.sample_format();
    let cfg: cpal::StreamConfig = config.into();
    let sink = Sink {
        samples,
        app,
        last_emit: Instant::now(),
        acc_sq: 0.0,
        acc_n: 0,
    };
    match format {
        cpal::SampleFormat::F32 => build::<f32>(&device, &cfg, channels, sink),
        cpal::SampleFormat::I16 => build::<i16>(&device, &cfg, channels, sink),
        cpal::SampleFormat::U16 => build::<u16>(&device, &cfg, channels, sink),
        cpal::SampleFormat::I32 => build::<i32>(&device, &cfg, channels, sink),
        other => bail!("Unsupported microphone sample format: {other:?}"),
    }
}

fn sample_rate_of(config: &cpal::SupportedStreamConfig) -> u32 {
    config.sample_rate()
}

fn build<T>(
    device: &cpal::Device,
    cfg: &cpal::StreamConfig,
    channels: usize,
    mut sink: Sink,
) -> Result<cpal::Stream>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    let stream = device.build_input_stream(
        cfg.clone(),
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            sink.push(data.chunks(channels).map(|frame| {
                frame.iter().map(|s| s.to_sample::<f32>()).sum::<f32>() / channels as f32
            }));
        },
        |e| eprintln!("audio stream error: {e}"),
        None,
    )?;
    Ok(stream)
}

fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let out_len = (input.len() as f64 / ratio) as usize;
    (0..out_len)
        .map(|i| {
            let pos = i as f64 * ratio;
            let idx = pos as usize;
            let frac = (pos - idx as f64) as f32;
            let a = input[idx];
            let b = *input.get(idx + 1).unwrap_or(&a);
            a + (b - a) * frac
        })
        .collect()
}
