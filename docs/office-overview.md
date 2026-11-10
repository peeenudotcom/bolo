# Presenting Bolo to your manager

## 30-second introduction

“Bolo is a free, open-source desktop tool we are developing to reduce office typing. Staff can dictate into existing applications, use spoken punctuation and formatting, and work with English, Hindi/Hinglish and Punjabi. It combines OpenAI services with local software; we build the desktop workflow, controls and safeguards. A new Windows and Mac meeting preview adds live captions, saved recordings and estimated speaker labels. I propose a small office pilot to measure accuracy, time saved and actual API spending before wider rollout.”

## What powers it

| Part | Technology / source | Where it runs |
|---|---|---|
| Desktop application | Tauri 2, Rust, TypeScript | Employee's computer |
| Cloud dictation recognition | OpenAI `gpt-transcribe` | OpenAI API |
| Dictation cleanup and Roman-script conversion | OpenAI `gpt-6-sol` by default | OpenAI API |
| Offline dictation | OpenAI's open-source Whisper models through whisper.cpp | Employee's computer |
| Live meeting captions | OpenAI `gpt-live-transcribe` | OpenAI API |
| Meeting computer-audio capture | Windows WASAPI / Mac ScreenCaptureKit | Employee's computer |
| Meeting speaker estimates after Stop | sherpa-onnx, pyannote segmentation, 3D-Speaker embeddings, ONNX Runtime | Employee's computer |
| Shortcuts, spoken formatting, clipboard, history, budget guard | Bolo application code | Employee's computer |

Bolo is an application built around these technologies, not a newly trained
foundation model. Its source code is MIT licensed; third-party notices are
included separately. No employee API key is included in the public installer.

## Cost and data handling

- The app has no Bolo subscription fee. Cloud processing consumes the configured
  OpenAI API balance; a ChatGPT subscription does not by itself establish an
  available API credit balance. Verify any account-specific credit grant separately.
- Bolo's monthly limit is an estimated guard for one computer, not an account-wide
  spending cap across all staff. Each laptop must be configured separately.
- Cloud dictation sends audio to OpenAI. Cleanup additionally sends transcript text
  and configured context. Offline recognition with cleanup off stays local.
- Live meetings send microphone and selected computer-output audio to OpenAI.
  Meeting recordings/transcripts are saved locally until deleted and are not
  encrypted by Bolo. Participants should know when recording is active.
- The meeting preview captures the selected output on Windows and system audio on Mac.
  Headphones avoid echoes; muting the meeting app does not mute Bolo's capture.
- Speaker labels are estimates, produced after Stop, not verified names. Meeting
  captions preserve the original language/script and do not apply dictation cleanup.

## A short demonstration

1. Dictate “Kal meri meeting hai, please send the report” into a draft document.
2. Say “Hello team, new paragraph, please send the report, question mark.”
3. Show the dictionary, recovery history and monthly budget controls.
4. On Windows or Mac, demonstrate a short consented two-person call: Start meeting,
   live captions, Stop, speaker estimates and transcript export.
5. Check names, numbers and Punjabi wording together; do not describe accuracy as perfect.

## Suggested pilot

Start with a few volunteers and non-sensitive work. Compare final text with what
was spoken, measure corrections and time saved, and record actual API costs.
Test long calls, background noise, reconnect behaviour and each office laptop's
microphone/headset before a broad rollout. There is currently no central office
login, shared administrator console or account-wide Bolo budget enforcement.

## Sources

- Bolo source and downloads: https://github.com/peeenudotcom/bolo
- Tauri: https://tauri.app/
- OpenAI transcription: https://developers.openai.com/api/docs/guides/transcription
- OpenAI live transcription: https://developers.openai.com/api/docs/guides/realtime-transcription
- OpenAI pricing: https://developers.openai.com/api/docs/pricing
- Whisper: https://github.com/openai/whisper
- whisper.cpp: https://github.com/ggml-org/whisper.cpp
- Windows audio capture: https://learn.microsoft.com/en-us/windows/win32/coreaudio/loopback-recording
- Local diarization: https://k2-fsa.github.io/sherpa/onnx/speaker-diarization/index.html
- ONNX Runtime: https://onnxruntime.ai/

Implementation and test limits: [Meetings technical notes](meetings.md).
