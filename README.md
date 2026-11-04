# Bolo

Free, open-source voice dictation for Windows, macOS and Linux. Hold a shortcut, speak, let go — clean text is typed into whatever app you're using.

- **Hold to talk** with just two fingers: Ctrl + Win (Fn on Mac)
- **Command mode**: select text, hold the shortcut plus Alt, say "make this more polite"
- **Matches the app**: casual in WhatsApp/Slack, full sentences in email, literal in code editors
- **Spoken formatting**: "new line", "new paragraph", "bullet point", "heart emoji"
- **Works offline** with Whisper models that run on your own computer
- **Optional free Groq key** for near-instant, more accurate transcription
- **AI cleanup** removes "um"s, fixes punctuation and applies your self-corrections (Groq or a local Ollama model)
- **Hinglish**: speak Hindi and English mixed, get it written in Roman script
- **Dictionary** for names and jargon, **snippets** that expand ("my email" → your address)
- Floating pill shows when it's listening; lives in the system tray

| | Windows / Linux | Mac |
|---|---|---|
| Dictate | hold **Ctrl + Win** | hold **Fn** |
| Hands-free | **Ctrl + Win + Space**, then Ctrl + Win to stop | **Fn + Space**, then Fn |
| Command (edit selected text) | hold **Ctrl + Win + Alt** | hold **Fn + Option** |
| Cancel | **Esc** | **Esc** |

Right Ctrl, Right Alt or any custom combo can be picked in settings.

## Install

Download the installer for your computer from [Releases](../../releases):

| Computer | File |
|---|---|
| Windows | `Bolo_x.y.z_x64-setup.exe` |
| Mac (Apple Silicon) | `Bolo_x.y.z_aarch64.dmg` |
| Mac (Intel) | `Bolo_x.y.z_x64.dmg` |
| Linux | `.AppImage` or `.deb` |

On first launch, download an offline model (Base is a good start) or paste a free key from [console.groq.com/keys](https://console.groq.com/keys).

**Mac:** the app isn't notarized yet, so right-click → Open the first time. Allow **Microphone** and **Accessibility** (System Settings → Privacy & Security) so Bolo can hear you and type for you.

**Linux:** typing into other apps works on X11. Wayland support depends on your desktop.

## English, Hindi and Hinglish

Leave **Language** on **Automatic** when switching between English, Hindi and mixed speech.
**Write Hindi as Hinglish** is on by default: Hindi is written in Roman letters
(“kal milte hain”), while English stays English. Switch it off for Hindi in
Devanagari (“कल मिलते हैं”), with English words kept in English.
Keep AI cleanup on with Groq or Ollama for script conversion. Without cleanup,
Whisper's raw output can use a different script. This changes the writing script,
not the meaning or language; it does not translate Hindi into English.

Recognition can still miss words, especially with noise or mixed speech. Check
important dictations before sending them.

## Recovery, privacy and preferences

- Finished text is saved to History before Bolo attempts to paste. If you switch
  windows, click elsewhere or type while it is processing, Bolo keeps the text
  for you instead of pasting into the changed destination.
- **Alt + Shift + V** pastes the last finished dictation; **Alt + Shift + C** copies
  it. Retry in History re-transcribes saved audio without automatically pasting.
  Voice edits are not retried against a different selection.
- History defaults to the current session. Optional 24-hour or 7-day local
  retention keeps up to 20 entries. Saving audio across restarts is separately
  opt-in. Audio in session memory supports retry until you quit. These files are
  local, not encrypted by Bolo; clearing History removes its saved recordings.
- **Escape** cancels recording or processing. Requests time out after 60 seconds;
  recordings stop after 2 minutes. Offline decoding may finish in the background
  after cancellation, but its result is not pasted.
- Choose a microphone and test its level for 5 seconds without saving or uploading
  test audio. Choose formatting styles per app category. AI cleanup applies
  capitalization/style preferences; corrections and snippet expansion also work
  without AI.
- Add whole-word/phrase corrections in Words or teach them from History. Bolo
  never monitors all your edits or learns spelling without an explicit action.
- Optional Windows nearby-text support reads at most 200 characters before and
  100 after the caret in supported focused text fields. It sends that text to
  your selected cleanup provider for spelling. It does not capture screenshots.
  Standard password fields and address/search fields are excluded; custom fields
  may not identify themselves correctly. Keep it off for sensitive work.
- Groq keys are stored in the OS credential store (Windows Credential Manager,
  macOS Keychain or Linux Secret Service). Existing plaintext keys are migrated
  only after secure storage succeeds. The settings UI receives a placeholder,
  not the stored key. A locked/unavailable credential store may need unlocking.
- Windows clipboard preservation supports text, HTML, bitmap images and file
  lists, and respects anything you copy during the paste. Unsupported formats or
  clipboards over 64 MB are left untouched; recover your dictation from History.
  On other platforms preservation supports text/images; unsupported clipboard
  formats may require turning preservation off. Native context and clipboard
  improvements have been verified on Windows only.
- Update checks use published GitHub releases. Startup checks are opt-in; updates
  are never downloaded or installed automatically.

## Develop

Needs Node 20+, Rust, CMake and the [Tauri prerequisites](https://tauri.app/start/prerequisites/) (on Windows: Visual Studio C++ Build Tools).

```bash
npm install
npm run tauri dev
```

On Windows, also install LLVM and Ninja (`winget install LLVM.LLVM Ninja-build.Ninja`) and run `. .\scripts\win-env.ps1` first in the same PowerShell window. It builds whisper.cpp with clang-cl, which transcribes about 10x faster than an MSVC build.

Release: bump the version in `src-tauri/tauri.conf.json`, then push a tag like `v0.1.0`. GitHub Actions builds every installer and drafts a release.

## How it works

`src-tauri/src/`:

- `audio.rs` records the microphone (cpal) and resamples to 16 kHz
- `transcribe.rs` runs whisper.cpp locally or calls Groq's Whisper API
- `cleanup.rs` polishes the transcript with an LLM and expands snippets
- `paste.rs` copies the text, presses Ctrl/Cmd+V, then restores your clipboard
- `lib.rs` wires up the shortcut, tray, settings and the floating pill

## License

MIT
