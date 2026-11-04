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
