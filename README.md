# Bolo

Free, open-source voice dictation for Windows, macOS and Linux. Hold a shortcut, speak, let go — clean text is typed into whatever app you're using.

- **Hold to talk** with just two fingers: Ctrl + Win (Fn on Mac)
- **Command mode**: select text, hold the shortcut plus Alt, say "make this more polite"
- **Matches the app**: casual in WhatsApp/Slack, full sentences in email, literal in code editors
- **Spoken formatting**: "new line", "new paragraph", "bullet point", "heart emoji"
- **Noise-only protection** checks for speech locally before transcription; rejected recordings show a message instead of pasting invented words. Once speech is detected, the full recording is preserved so quiet opening and ending words are not trimmed away
- **Works offline** with Whisper models that run on your own computer
- **One OpenAI API key** for cloud speech recognition and cleanup
- **AI cleanup** removes "um"s, fixes punctuation and applies your self-corrections (OpenAI)
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

On first launch, download an offline model (Base is a good start) or add an [OpenAI API key](https://platform.openai.com/api-keys).

**Mac:** the app isn't notarized yet, so right-click → Open the first time. Allow **Microphone** and **Accessibility** (System Settings → Privacy & Security) so Bolo can hear you and type for you.

**Linux:** typing into other apps works on X11. Wayland support depends on your desktop.

## English, Hindi, Hinglish and Punjabi

Bolo's current language scope is English, Hindi, Hinglish, Punjabi and mixed
speech. Leave **Language** on **Automatic / mixed speech** when switching between
them. Hinglish uses automatic Hindi/English recognition and Roman output; it is
not a separate speech-engine language code.

**Roman Hindi & Punjabi** is on by default: Hindi uses Roman letters
(“kal milte hain”), Punjabi uses Roman letters (“kal milde haan”), and English
stays English. Switch it off for Hindi in Devanagari and Punjabi in Gurmukhi.
Keep AI cleanup on with OpenAI for script conversion. Bolo checks for
Latin-script words before pasting when Roman output is enabled. If conversion
fails, the text stays in History. Automatic recognition is never forced into
Hindi based only on the transcript's script, since it may be Punjabi.

Script conversion must preserve the spoken language and meaning, without
translation. Language choices define the current product scope, not a guarantee
of recognition accuracy. Mixed speech and Punjabi still need evaluation on real
recordings; check important dictations before sending them.

## Formatting by voice

Enable **Spoken formatting** for explicit commands. English command phrases can
be used within English, Hindi, Hinglish or Punjabi dictation. Open **What can I
say for formatting?** in Dictation for examples.

| Say | Result |
|---|---|
| new line / next line / line break / skip a line | Line break |
| new paragraph / start a new paragraph | Blank line |
| comma / period / full stop / question mark / exclamation mark | Punctuation |
| colon / semicolon / ellipsis / em dash | `:` / `;` / `…` / `—` |
| bullet point milk, next item eggs, end list | Bullet list, then return to normal text |
| numbered list milk, next item eggs, end list | Numbered list |
| open quote hello close quote | `"hello"` |
| open parenthesis optional close parenthesis | `(optional)` |
| open bracket / close bracket / open brace / close brace | `[` / `]` / `{` / `}` |
| at sign / underscore / forward slash / hyphen / decimal point | `@` / `_` / `/` / `-` / `.` |
| percent sign / plus sign / equals sign / hash sign / ampersand | `%` / `+` / `=` / `#` / `&` |
| heart emoji / thumbs up emoji / folded hands emoji | ❤️ / 👍 / 🙏 |

AI cleanup additionally formats clear numbers, amounts, percentages, times,
email addresses, inferred lists and explicitly spoken email greetings/sign-offs.
It applies clear self-corrections such as “Tuesday, no wait, Wednesday”, while
preserving meaningful uses of words like “actually” and “like”. It must not
invent dates, currencies, names or missing words, or translate languages.
These AI-dependent cases can still be wrong; review important values.

Common references such as “remove the comma” remain words. Technical/literal
style preserves casing during spoken command conversion. Dictated paragraph
boundaries, explicit list numbering and requested final punctuation take
priority over automatic style. No formatting command sends a message or presses
Enter in the destination app; output is pasted as text.

## OpenAI recognition and cleanup

Save one OpenAI API key in Settings. The key is stored in the OS credential store,
never bundled with the app or stored in settings.json. OpenAI is the only cloud
provider. Speech uses `gpt-transcribe`; cleanup defaults to `gpt-6-sol` with
reasoning disabled for responsiveness. Automatic / mixed speech uses unrestricted detection with English, Hindi and
Punjabi context. The current API rejects Punjabi language codes, so Punjabi uses
context rather than a hard language hint. Punjabi recognition still needs real
voice testing; the app does not claim perfect mixed-language accuracy. Dictionary terms are vocabulary hints,
not instructions to insert words. Script conversion belongs to cleanup and must
never translate Punjabi to Hindi or either language to English.

Multi-line cleanup uses one structured request; Bolo restores explicit blank lines
and list markers locally and rejects malformed results.

OpenAI API usage is separate from ChatGPT/Codex weekly usage. Check the linked
API organization's included credits and expiry; keep auto-recharge off to avoid
additional purchases. Bolo cannot inspect billing with a project API key and does
not buy credits. Quota, permission and service errors stay visible in History;
there is no silent cloud-provider fallback or automatic retry of failed requests.

Offline recognition remains available. To keep all dictation processing local,
select Offline only and turn AI cleanup off; Roman conversion then requires the
offline recognizer itself to produce Roman text. With cleanup enabled, transcripts
still go to OpenAI. No cloud keys are distributed in installers. On another laptop,
save a key for that laptop in Settings.

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
- OpenAI keys are stored in the OS credential store (Windows Credential Manager,
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
- `transcribe.rs` runs whisper.cpp locally; `openai.rs` calls OpenAI speech recognition
- `cleanup.rs` polishes the transcript with an LLM and expands snippets
- `paste.rs` copies the text, presses Ctrl/Cmd+V, then restores your clipboard
- `lib.rs` wires up the shortcut, tray, settings and the floating pill

## License

MIT


## Word libraries and recording playback

In Words, export a JSON word library to share vocabulary, spelling corrections
and snippets with another laptop. Import previews additions and conflicts before
saving; existing replacements win. The file contains only these word entries,
never API keys, other settings, history or recordings. Treat shared entries as
office information and review imported text before adding it.

History offers Listen when the original recording is still available. Playback
is local and uses no API credits. Press play in the audio controls; playback stops
when leaving History or starting a new recording. Session recordings disappear
when Bolo quits unless saved audio and persistent history are enabled.

Snippet expansion uses the original dictated text once, with longer triggers
taking priority over shorter ones. Saved expansion text cannot trigger another
snippet, and empty expansions do not erase dictated words.


Recognition hints include short saved vocabulary entries, preferred correction
spellings and snippet triggers, with duplicates removed and a 100-term limit.
Snippet expansions are not included in these recognition hints. Hints improve
the information supplied to the recognizer; they do not guarantee accuracy.

When an entire dictation matches a saved snippet, Bolo inserts its literal
expansion without an AI cleanup request. Spoken empty list starters are handled
locally. Explicit repeated punctuation and full stops followed by paragraph
breaks survive cleanup/style processing. Cleanup that removes every word from
a nonempty spoken sentence is rejected and remains recoverable in History.

## Monthly budget

Settings → Monthly budget starts with a $20 USD limit; choose your own amount,
including $0 to block cloud requests. This is a local estimated cap for this
computer, starting with this feature, not an OpenAI balance or account-wide cap.
Calendar months reset at 00:00 UTC on the first. Changing a limit or clearing
History does not reset recorded usage. Past months are retained so rolling the
clock back does not create a fresh allowance.

Each speech, cleanup, edit and retry request reserves budget on disk before it
can be sent. Cleanup reserves its maximum output plus conservative input and
then reconciles returned token counts. Interrupted/failed requests retain their
reservation because charges may be unknown. Near the limit a request may be
blocked early. Unreadable budget files block cloud use rather than resetting
usage. Unknown cleanup models are blocked until their pricing is supported.

Estimates use gpt-transcribe at $0.0045/minute and gpt-6-sol/gpt-6.1-sol at
$2/$10 per million input/output tokens (checked 7 November 2026 at
https://developers.openai.com/api/docs/pricing and the gpt-6-sol model page).
Cache discounts are ignored. Provider price changes, taxes, other apps/devices
and usage before tracking began are not covered. Check the provider dashboard
for actual charges. Offline recognition with cleanup off needs no cloud budget.

## Online meetings (Windows preview)

Open **Meetings**, select the speakers/headphones used by your call, then click
**Start meeting**. Bolo captures computer audio and your selected microphone,
shows live OpenAI captions and saves both recordings locally. Use headphones to
avoid echoes. All sound on the selected output device is captured; let meeting
participants know before starting. Click **Stop meeting** to finish and save.

Download the optional 41 MB speaker models once to estimate numbered speakers
locally after stopping. Live source labels are not individual participant labels.
Overlapping/ambiguous speakers are not forcibly assigned to one person. Captions
preserve the spoken language/script; dictation formatting commands and AI cleanup
are not applied. No translation is requested.

Live transcription uses the existing OpenAI key and local monthly budget, at
$0.017/minute per transmitted stream (roughly $2.04/hour for two continuous
streams; silence filtering can reduce this). Local speaker labels cost no API
credits. Sessions stop after 60 minutes. Saved meetings persist until deleted,
independently of dictation History. Export text or open the saved WAV recordings
from Meetings. Closing the window keeps recording; use Stop meeting.

This is a Windows preview requiring a real meeting test before office rollout.
See [implementation, research and limitations](docs/meetings.md).
