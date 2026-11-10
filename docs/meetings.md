# Online meetings: implementation and validation

Windows preview. Meetings are independent of hold-to-talk dictation and never
paste into the focused application. Explicit Start captures the selected playback
endpoint through WASAPI loopback plus the dictation microphone. Headphones are
recommended: this implementation does not perform acoustic echo cancellation.
All audio on the selected output endpoint is captured, not just a meeting app.

## Decisions and primary sources (checked 10 November 2026)

- [Microsoft WASAPI loopback](https://learn.microsoft.com/en-us/windows/win32/coreaudio/loopback-recording): captures render-endpoint audio without a virtual cable.
- [OpenAI live transcription](https://developers.openai.com/api/docs/guides/realtime-transcription): streaming PCM, incremental/final events, client-side turn commits. Uses the existing key, with no new account/provider. The live model does not supply speaker IDs or word timestamps.
- [OpenAI model pricing](https://developers.openai.com/api/docs/models/gpt-live-transcribe): $0.017/minute per transmitted audio stream. Two continuous streams estimate $2.04/hour before any rounding; silence filtering reduces transmitted duration. Reservations precede every append and remain charged if a request fails.
- [sherpa-onnx diarization](https://k2-fsa.github.io/sherpa/onnx/speaker-diarization/index.html): local post-recording segmentation, embeddings and clustering. Windows uses CPU inference with two threads and separately packaged native DLLs. Models: pyannote segmentation 3.0 int8 (MIT) and 3D-Speaker ERes2Net (Apache-2.0), approximately 41 MB total, verified by SHA-256 before use. Notices are in licenses/.
- [Deepgram diarization](https://developers.deepgram.com/docs/diarization) and [AssemblyAI streaming diarization](https://www.assemblyai.com/docs/streaming/label-speakers-and-separate-channels) were considered. Both offer streaming speaker labels, but require another provider account and billing relationship. They are not integrated.

## Behaviour and limits

Live labels identify sources (Microphone / Computer audio). If local models have
been downloaded, computer audio is clustered after Stop. Numbered speakers are
estimates, not names or verified identities. A caption turn overlapping multiple
speaker segments is labelled Multiple speakers; text is not arbitrarily split or
assigned to one person. Turn timestamps are capture-time estimates, not word
alignment. Microphone audio is not assumed to be a single named person.

Captions preserve original spoken language/script. Dictation's rewriting,
Roman conversion, snippets and punctuation commands are not applied to meeting
speech. No extra cleanup requests are made. Recognition and speaker clustering
can still be wrong; mixed Hindi/Punjabi/English and overlapping speech require
real meeting evaluation.

Capture callbacks use bounded nonblocking queues. WAV files are written on worker
threads and flushed regularly; transcript finals are saved atomically. Persistent
JSON retains provisional/final flags. Interrupted sessions are marked on reload.
Device failure, queue overflow, API failure and budget exhaustion stop capture
and retain earlier text/audio. There is no automatic reupload or reconnect that
could duplicate captions or unexpectedly spend more. Maximum session: 60 minutes.

Meetings are stored separately from dictation History and persist until explicitly
deleted. Audio is local, not encrypted by Bolo. The UI discloses capture and cloud
upload before Start. The tray Quit action asks the user to stop/save an active
meeting; forced termination can still interrupt it. No meeting capture occurs
on application startup. macOS/Linux meeting capture is not implemented.

## Validation

- Unit tests: out-of-order completion, final replacement of partials, safe
  handling of failed events, source separation, cost calculation, ambiguous
  speaker assignment and mixed-script serialization.
- Local model smoke test using the repository's public speech fixture.
- Opt-in OpenAI live fixture test, using the real local budget ledger.
- Mocked frontend checks at 900x760 and 460x520, 125% scaling: transcript
  escaping, scrolling and no horizontal overflow.
- Actual meeting capture, two-person speaker accuracy and a sustained call must
  be tested in the user's Windows session before general office rollout.

Local installation verified on 10 November 2026. The public upstream two-speaker English fixture produced two clusters (2.23 s processing on this laptop). This validates that path, not Punjabi accuracy or real-call capture. Runtime ONNX 1.28.2 and speaker models are installed. Desktop call test is pending.
