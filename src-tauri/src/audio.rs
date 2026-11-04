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
    pub fn start(app: AppHandle, microphone: String) -> Result<Self> {
        let samples = Arc::new(Mutex::new(Vec::with_capacity(48_000 * 30)));
        let rate = Arc::new(Mutex::new(TARGET_RATE));
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<()>>();
        let (s2, r2) = (samples.clone(), rate.clone());

        let handle = thread::spawn(move || {
            let limit_app = app.clone();
            let stream = match open_stream(app, s2, r2, &microphone) {
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
            if ready_tx.send(Ok(())).is_err() { return; }
            if matches!(stop_rx.recv_timeout(Duration::from_secs(120)), Err(mpsc::RecvTimeoutError::Timeout)) {
                crate::act(&limit_app, crate::hotkey::Action::Finish { command: false });
            }
            drop(stream);
        });

        ready_rx
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| anyhow!("Microphone did not start. Check the device and retry."))??;
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
        let raw = std::mem::take(&mut *self.samples.lock().unwrap_or_else(|e| e.into_inner()));
        let rate = *self.rate.lock().unwrap_or_else(|e| e.into_inner());
        resample(&raw, rate, TARGET_RATE)
    }
}

impl Drop for Recorder {
    fn drop(&mut self) { if let Some(tx) = self.stop_tx.take() { let _ = tx.send(()); } }
}

pub fn microphones() -> Result<Vec<String>> {
    Ok(cpal::default_host().input_devices()?.filter_map(|d| d.description().ok().map(|d| d.name().to_string())).collect())
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
            let mut buf = self.samples.lock().unwrap_or_else(|e| e.into_inner());
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
    microphone: &str,
) -> Result<cpal::Stream> {
    let host = cpal::default_host();
    let device = if microphone.is_empty() { host.default_input_device() }
        else { host.input_devices()?.find(|d| d.description().is_ok_and(|d| d.name() == microphone)) }
        .ok_or_else(|| anyhow!("Microphone unavailable. Choose another microphone in Settings."))?;
    let config = device.default_input_config()?;
    let channels = config.channels() as usize;
    anyhow::ensure!(channels > 0 && sample_rate_of(&config) > 0, "Microphone returned an invalid audio format");
    *rate.lock().unwrap_or_else(|e| e.into_inner()) = sample_rate_of(&config);
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
    let error_app = sink.app.clone();
    let stream = device.build_input_stream(
        cfg.clone(),
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            sink.push(data.chunks(channels).map(|frame| {
                frame.iter().map(|s| s.to_sample::<f32>()).sum::<f32>() / channels as f32
            }));
        },
        move |e| crate::status(&error_app, "error", Some(format!("Microphone disconnected or failed: {e}. Cancel and retry."))),
        None,
    )?;
    Ok(stream)
}

fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == 0 || to == 0 { return Vec::new(); }
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let out_len = (input.len() as f64 / ratio) as usize;
    if from > to {
        // Band-limit before decimation. Linear interpolation alone aliases
        // frequencies above the new Nyquist limit into the speech band.
        // Precompute fractional phases so recording completion stays fast.
        const PHASES: usize = 256;
        const RADIUS: isize = 32;
        let cutoff = 0.94 / ratio;
        let kernels: Vec<Vec<f64>> = (0..PHASES).map(|phase| {
            let frac = phase as f64 / PHASES as f64;
            let mut weights: Vec<f64> = (-RADIUS..=RADIUS).map(|tap| {
                let x = tap as f64 - frac;
                let sinc = if x.abs() < 1e-10 { cutoff }
                    else { (std::f64::consts::PI * cutoff * x).sin() / (std::f64::consts::PI * x) };
                let window = if x.abs() > RADIUS as f64 { 0.0 }
                    else { 0.42 + 0.5 * (std::f64::consts::PI * x / RADIUS as f64).cos()
                        + 0.08 * (2.0 * std::f64::consts::PI * x / RADIUS as f64).cos() };
                sinc * window
            }).collect();
            let sum: f64 = weights.iter().sum();
            for w in &mut weights { *w /= sum; }
            weights
        }).collect();
        return (0..out_len).map(|i| {
            let pos = i as f64 * ratio;
            let center = pos.floor() as isize;
            let phase = ((pos.fract() * PHASES as f64) as usize).min(PHASES - 1);
            kernels[phase].iter().enumerate().map(|(tap, weight)| {
                let index = (center + tap as isize - RADIUS).clamp(0, input.len() as isize - 1) as usize;
                input[index] as f64 * weight
            }).sum::<f64>() as f32
        }).collect();
    }
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

#[cfg(test)]
mod tests {
    #[test]
    fn downsampling_preserves_speech_and_rejects_aliasing() {
        for rate in [44_100, 48_000] {
            let tone = |hz: f64| (0..rate).map(|i|
                (2.0 * std::f64::consts::PI * hz * i as f64 / rate as f64).sin() as f32
            ).collect::<Vec<_>>();
            let rms = |data: &[f32]| (data[100..data.len()-100].iter().map(|x| x*x).sum::<f32>()
                / (data.len()-200) as f32).sqrt();
            let speech = super::resample(&tone(1000.0), rate, 16000);
            let noise = super::resample(&tone(12000.0), rate, 16000);
            assert_eq!(speech.len(), 16000);
            assert!((rms(&speech) - 0.7071).abs() < 0.01);
            assert!(rms(&noise) < 0.01, "aliased noise at {rate} Hz: {}", rms(&noise));
            let dc = super::resample(&vec![0.25; rate as usize], rate, 16000);
            assert!(dc.iter().all(|x| (*x - 0.25).abs() < 0.0001));
        }
    }

    #[test]
    fn resampling_handles_empty_invalid_and_short_audio() {
        assert!(super::resample(&[], 48000, 16000).is_empty());
        assert!(super::resample(&[1.0], 0, 16000).is_empty());
        assert!(super::resample(&[1.0], 16000, 0).is_empty());
        assert_eq!(super::resample(&[0.5], 16000, 48000), vec![0.5; 3]);
    }
}
