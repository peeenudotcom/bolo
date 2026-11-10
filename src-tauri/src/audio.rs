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
    failure: Arc<Mutex<Option<String>>>,
}

impl Recorder {
    pub fn start(app: AppHandle, microphone: String) -> Result<Self> {
        let samples = Arc::new(Mutex::new(Vec::with_capacity(48_000 * 30)));
        let rate = Arc::new(Mutex::new(TARGET_RATE));
        let failure = Arc::new(Mutex::new(None));
        let stream_failure = failure.clone();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<()>>();
        let (s2, r2) = (samples.clone(), rate.clone());

        let handle = thread::spawn(move || {
            let limit_app = app.clone();
            let stream = match open_stream(app, s2, r2, &microphone, stream_failure) {
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
            failure,
        })
    }

    /// Stops recording and returns 16 kHz mono samples.
    pub fn stop(mut self) -> Result<Vec<f32>> {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(());
        }
        if let Some(h) = self.handle.take() {
            h.join().map_err(|_| anyhow!("Microphone worker stopped unexpectedly. Please record again."))?;
        }
        let raw = std::mem::take(&mut *self.samples.lock().unwrap_or_else(|e| e.into_inner()));
        let rate = *self.rate.lock().unwrap_or_else(|e| e.into_inner());
        let failure = self.failure.lock().unwrap_or_else(|e| e.into_inner()).take();
        finish_recording(&raw, rate, failure.as_deref())
    }
}

fn finish_recording(raw: &[f32], rate: u32, failure: Option<&str>) -> Result<Vec<f32>> {
    if let Some(error) = failure { bail!("Microphone recording failed: {error}. Please record again; incomplete audio was not sent."); }
    anyhow::ensure!(rate > 0 && raw.iter().all(|v| v.is_finite()), "Microphone returned invalid audio. Please record again.");
    Ok(resample(raw, rate, TARGET_RATE))
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
    failure: Arc<Mutex<Option<String>>>,
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
        cpal::SampleFormat::F32 => build::<f32>(&device, &cfg, channels, sink, failure),
        cpal::SampleFormat::I16 => build::<i16>(&device, &cfg, channels, sink, failure),
        cpal::SampleFormat::U16 => build::<u16>(&device, &cfg, channels, sink, failure),
        cpal::SampleFormat::I32 => build::<i32>(&device, &cfg, channels, sink, failure),
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
    failure: Arc<Mutex<Option<String>>>,
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
        move |e| {
            *failure.lock().unwrap_or_else(|e| e.into_inner()) = Some(e.to_string());
            crate::status(&error_app, "error", Some("Microphone disconnected or failed. Cancel and record again.".into()));
        },
        None,
    )?;
    Ok(stream)
}

pub(crate) fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
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

/// Reject noise-only captures before either cloud or offline recognition.
/// Once speech is confirmed, preserve the entire capture: VAD onset estimates
/// can miss quiet opening words, even when later speech is detected correctly.
/// Runs after recording on a worker, never inside the keyboard/audio callback.
pub fn speech_audio(samples: Vec<f32>) -> Result<Vec<f32>> {
    anyhow::ensure!(samples.iter().all(|s| s.is_finite()), "Invalid microphone audio. Check the microphone and retry.");
    const FRAME: usize = 256; // Earshot requires 16 ms at 16 kHz.
    const WINDOW: usize = 12;
    let mut detector = earshot::Detector::default();
    let mut recent = [false; WINDOW];
    let mut confirmed = false;
    for (index, chunk) in samples.chunks(FRAME).enumerate() {
        let mut frame = [0.0; FRAME];
        for (out, input) in frame.iter_mut().zip(chunk) { *out = input.clamp(-1.0, 1.0); }
        let score = detector.predict_f32(&frame);
        // A conservative threshold avoids startup uncertainty on steady noise.
        let voiced = score >= 0.7;
        recent[index % WINDOW] = voiced;
        // Several speech frames close together: a single click is insufficient.
        confirmed |= recent.iter().filter(|&&v| v).count() >= 6;
    }
    anyhow::ensure!(confirmed, "No clear speech detected. Background noise was ignored; nothing was pasted. Try speaking closer to the microphone.");
    Ok(samples)
}

#[cfg(test)]
mod tests {
    fn noise(len: usize) -> Vec<f32> {
        let mut seed = 42u32;
        (0..len).map(|_| {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            ((seed >> 8) as f32 / 16777216.0 - 0.5) * 0.12
        }).collect()
    }

    #[test]
    fn failed_recordings_cannot_return_partial_audio() {
        assert!(super::finish_recording(&[0.1,0.2], 16000, Some("device disconnected")).is_err());
        assert!(super::finish_recording(&[f32::NAN], 16000, None).is_err());
        assert_eq!(super::finish_recording(&[0.1,0.2], 16000, None).unwrap(), vec![0.1,0.2]);
    }

    #[test]
    fn rejects_silence_fan_like_noise_and_clicks() {
        let len = super::TARGET_RATE as usize * 3;
        assert!(super::speech_audio(vec![]).is_err());
        assert!(super::speech_audio(vec![0.0; len]).is_err());
        assert!(super::speech_audio(vec![0.1; len]).is_err());
        assert!(super::speech_audio(vec![f32::NAN; 256]).is_err());
        let white = noise(len);
        for gain in [0.01, 0.1, 1.0, 3.0] {
            assert!(super::speech_audio(white.iter().map(|x| x * gain).collect()).is_err(), "noise accepted at gain {gain}");
        }
        let mut low = 0.0;
        let fan: Vec<f32> = white.iter().enumerate().map(|(i, &x)| {
            low = low * 0.95 + x * 0.05;
            low + 0.03 * (std::f32::consts::TAU * 100.0 * i as f32 / 16000.0).sin()
        }).collect();
        assert!(super::speech_audio(fan).is_err());
        let mut clicks = vec![0.0; len];
        for i in (0..len).step_by(8000) { clicks[i] = 0.8; }
        assert!(super::speech_audio(clicks).is_err());
    }

    #[test]
    fn speech_fixture_survives_quiet_levels_and_background_noise() {
        let bytes = include_bytes!("../tests/fixtures/speech.wav");
        let mut wav = hound::WavReader::new(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(wav.spec().sample_rate, super::TARGET_RATE);
        let speech: Vec<f32> = wav.samples::<i16>().map(|v| v.unwrap() as f32 / 32768.0).collect();
        for gain in [1.0, 0.1, 0.03] {
            let quiet: Vec<f32> = speech.iter().map(|s| s * gain).collect();
            assert!(super::speech_audio(quiet).is_ok(), "quiet speech rejected at gain {gain}");
        }
        let background = noise(speech.len());
        let mixed: Vec<f32> = speech.iter().zip(background).map(|(s, n)| s + n * 0.3).collect();
        assert!(super::speech_audio(mixed).is_ok());
        let mut padded = vec![0.0; 16000];
        padded.extend_from_slice(&speech);
        padded.extend(vec![0.0; 16000]);
        assert_eq!(super::speech_audio(padded.clone()).unwrap(), padded);
    }

    #[test]
    fn quiet_opening_and_ending_are_not_cut_from_confirmed_speech() {
        let mut wav = hound::WavReader::new(std::io::Cursor::new(include_bytes!("../tests/fixtures/speech.wav"))).unwrap();
        let speech: Vec<f32> = wav.samples::<i16>().map(|v| v.unwrap() as f32 / 32768.0).collect();
        // A quiet phrase precedes and follows clearly detected speech. The gate
        // must not use the louder middle as a boundary for deleting words.
        let quiet: Vec<f32> = speech.iter().map(|s| s * 0.001).collect();
        let mut capture = quiet.clone();
        capture.extend(vec![0.0; 16000]);
        capture.extend_from_slice(&speech);
        capture.extend(vec![0.0; 16000]);
        capture.extend_from_slice(&quiet);
        let prepared = super::speech_audio(capture.clone()).unwrap();
        assert_eq!(prepared, capture, "Speech detection must not discard uncertain words at either edge");
    }

    #[test]
    fn short_word_is_not_mistaken_for_a_click() {
        let mut wav = hound::WavReader::new(std::io::Cursor::new(include_bytes!("../tests/fixtures/yes.wav"))).unwrap();
        let speech: Vec<f32> = wav.samples::<i16>().map(|v| v.unwrap() as f32 / 32768.0).collect();
        for gain in [1.0, 0.1] {
            assert!(super::speech_audio(speech.iter().map(|x| x * gain).collect()).is_ok());
        }
    }

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
