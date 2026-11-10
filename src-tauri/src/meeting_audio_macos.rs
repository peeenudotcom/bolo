//! macOS system audio. ScreenCaptureKit supplies audio; no screen frames are retained.
use super::*;
use screencapturekit::{prelude::*, stream::delegate_trait::ErrorHandler};

pub(super) fn start(epoch:Instant, tx:mpsc::SyncSender<Raw>, failed:Arc<AtomicBool>)->Result<SCStream> {
    let content=SCShareableContent::get().context("Allow Bolo in System Settings > Privacy & Security > Screen & System Audio Recording, then restart Bolo and try again")?;
    let displays=content.displays();
    let display=displays.first().context("No display is available for meeting audio capture")?;
    let filter=SCContentFilter::create().with_display(display).with_excluding_windows(&[]).build()?;
    let config=SCStreamConfiguration::new().with_width(2).with_height(2)
        .with_captures_audio(true).with_sample_rate(16000).with_channel_count(1)
        .with_excludes_current_process_audio(true);
    let error=failed.clone();
    let delegate=ErrorHandler::new(move |_|{error.store(true,Ordering::Release);});
    let mut stream=SCStream::new_with_delegate(&filter,&config,delegate)?;
    stream.add_output_handler(move |sample:CMSampleBuffer, kind:SCStreamOutputType| {
        if kind!=SCStreamOutputType::Audio {return;}
        let Ok(buffers)=sample.audio_buffer_list() else {failed.store(true,Ordering::Release);return;};
        // The requested mono stream is native Float32 PCM. Reject unexpected
        // buffer shapes rather than uploading misinterpreted/corrupt samples.
        let mut iter=buffers.iter();
        let Some(buffer)=iter.next() else{return;};
        let data=buffer.data();
        if iter.next().is_some() || data.len()%4!=0 || data.len()/4!=sample.num_samples() as usize {
            failed.store(true,Ordering::Release);return;
        }
        let samples:Vec<f32>=data.chunks_exact(4).map(|b|f32::from_le_bytes([b[0],b[1],b[2],b[3]])).collect();
        if samples.is_empty(){return;}
        if samples.len()>16000 || samples.iter().any(|s|!s.is_finite()) {failed.store(true,Ordering::Release);return;}
        let at=(epoch.elapsed().as_millis() as u64).saturating_sub(samples.len() as u64*1000/16000);
        if tx.try_send(Raw {at,rate:16000,samples}).is_err(){failed.store(true,Ordering::Release);}
    },SCStreamOutputType::Audio)?;
    stream.start_capture().context("Could not start Mac system audio. Check Bolo's Screen & System Audio Recording permission")?;
    Ok(stream)
}
