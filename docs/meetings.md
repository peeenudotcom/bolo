# Online meetings: implementation and validation

Windows and macOS preview. Meetings are independent of hold-to-talk dictation and never
paste into the focused application. Explicit Start captures the selected playback
endpoint through WASAPI loopback plus the dictation microphone. Headphones are
recommended: this implementation does not perform acoustic echo cancellation.
Windows captures all audio on the selected output endpoint. macOS 13+ uses ScreenCaptureKit for system audio and the selected microphone through CoreAudio; it requires Screen & System Audio Recording permission. Screen frames are not stored or uploaded. Neither platform limits capture to one meeting app.

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
on application startup. Linux meeting capture is not implemented. Mac native compilation, signature and runtime checks are required before release, followed by real-call testing on a user Mac.

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

## Mac parity validation

Mac system audio uses ScreenCaptureKit on macOS 13+, with a separate CoreAudio microphone. The same transcription, budget, save/export and local speaker pipeline is used on both platforms. Native CI runs on Intel and Apple Silicon, verifies the ad-hoc bundle signature and executes the packaged binary to catch loader failures. This does not replace permission and real-call testing on a user Mac. No Apple notarization is claimed.

## Reviewing and exporting

Search filters the displayed transcript locally, including speaker labels, without
changing saved content. Export always includes the full meeting. Text exports retain
unfinished drafts and recording errors. SRT subtitle exports contain final captions
only; timings are approximate capture times, and estimated speaker labels remain
explicitly marked. No cloud requests or extra API costs are involved in search/export.

Linux installers remain paused while Windows and Mac are the office targets.

## Notes and questions

After Stop, Generate meeting notes requests a recap, decisions and action items.
Ask answers one question using only that saved meeting. Both are explicit, budgeted
OpenAI requests using the configured cleanup model; there is no automatic analysis.
Only final captions are sent. Inputs too large for the bounded request are rejected
before sending, never silently shortened. An interrupted meeting may only have partial
coverage: notes describe the retained final captions, not missing speech.

Every returned item must cite an exact quote from an existing final caption. Invalid
references or altered quotes reject the response. This checks evidence references,
not whether the AI's interpretation is correct. Review notes and source captions.
Click quotes to jump to the transcript. Notes remain separate from captions, with the
latest 20 request results stored locally per meeting and a separate text export.
Deleting a meeting deletes its notes too; a pending response cannot recreate it.

## Long dictation, corrections and recovery

Microphone-only long recordings reuse the live capture/checkpoint path without opening
or uploading a system-audio stream. Both recording modes stop at 60 minutes. The UI
warns during the final five minutes; regular shortcut dictation stays at two minutes.
This mode is original-script live captioning, not the normal Roman-output cleanup path.

Speaker corrections are stored as separate user-assigned labels. A turn override wins
against a speaker/source name. Clearing an override restores the original estimate.
Captions themselves remain unchanged. Existing notes are not silently regenerated.

Audio recovery opens retained 16 kHz mono WAVs in bounded chunks (up to 60 seconds),
preferring quiet boundaries in the last ten seconds. It checks audio metadata against
its checkpoint and blocks changed files. Each request uses the normal budget reservation.
An explicit estimate precedes upload; silence detection may reduce actual usage.
Errors stop processing without automatic API retries. Resume skips completed chunks.
If a crash occurs after a response but before checkpoint persistence, that last chunk
may require another charged request. The original recording and transcript are retained.

Recovered captions are saved as a separate document, linked to the original recording.
They have coarse segment timestamps and source labels, not inferred individual speakers.
Deleting/correcting a source or output document is blocked during active recovery.
Checkpoints verify destination ownership and cannot overwrite unrelated recordings.
Native microphone permission, real long recordings and real interrupted-call recovery
still require validation in the user's Windows/Mac session before office rollout.
