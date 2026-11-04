mod audio;
mod cleanup;
mod hotkey;
mod models;
mod paste;
mod settings;
mod style;
mod transcribe;

use hotkey::Action;
use serde::Serialize;
use settings::Settings;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt as _};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

/// With a custom key combo, a press shorter than this switches to hands-free.
const TAP_THRESHOLD: Duration = Duration::from_millis(350);
const MIN_SAMPLES: usize = audio::TARGET_RATE as usize / 4;

enum Phase {
    Idle,
    Recording {
        rec: audio::Recorder,
        started: Instant,
        hands_free: bool,
        command: bool,
        target: style::Target,
    },
    Processing,
}

#[derive(Clone, Serialize)]
struct HistoryItem {
    text: String,
    raw: String,
    at: u64,
    engine: String,
    app: String,
    command: bool,
}

struct AppState {
    settings: Mutex<Settings>,
    phase: Mutex<Phase>,
    whisper: transcribe::WhisperCache,
    history: Mutex<Vec<HistoryItem>>,
    http: reqwest::Client,
}

#[derive(Clone, Serialize)]
struct StatusEvent {
    state: &'static str,
    message: Option<String>,
}

fn status(app: &AppHandle, state: &'static str, message: Option<String>) {
    let _ = app.emit("bolo://state", StatusEvent { state, message });
}

// ---------- dictation actions ----------

fn act(app: &AppHandle, action: Action) {
    let state = app.state::<AppState>();
    let mut phase = state.phase.lock().unwrap();
    match (action, &mut *phase) {
        (Action::Start, Phase::Idle) => match audio::Recorder::start(app.clone()) {
            Ok(rec) => {
                let target = if state.settings.lock().unwrap().app_style {
                    style::detect()
                } else {
                    style::Target { app: String::new(), kind: style::AppKind::Other }
                };
                *phase = Phase::Recording { rec, started: Instant::now(), hands_free: false, command: false, target };
                hotkey::set_recording(true);
                status(app, "recording", None);
            }
            Err(e) => status(app, "error", Some(e.to_string())),
        },
        (Action::CommandMode, Phase::Recording { command, hands_free, .. }) => {
            *command = true;
            let msg = if *hands_free { "command hands-free" } else { "command" };
            status(app, "recording", Some(msg.into()));
        }
        (Action::HandsFree, Phase::Recording { hands_free, command, .. }) => {
            *hands_free = true;
            let msg = if *command { "command hands-free" } else { "hands-free" };
            status(app, "recording", Some(msg.into()));
        }
        (Action::Finish { command: from_keys }, Phase::Recording { .. }) => {
            if let Phase::Recording { rec, command, target, .. } = std::mem::replace(&mut *phase, Phase::Processing) {
                drop(phase);
                hotkey::set_recording(false);
                let app = app.clone();
                let command = command || from_keys;
                tauri::async_runtime::spawn(async move {
                    let result = process(&app, rec, command, target).await;
                    *app.state::<AppState>().phase.lock().unwrap() = Phase::Idle;
                    match result {
                        Ok(_) => status(&app, "idle", None),
                        Err(e) => status(&app, "error", Some(e.to_string())),
                    }
                });
            }
        }
        (Action::Cancel, Phase::Recording { .. }) => {
            if let Phase::Recording { rec, .. } = std::mem::replace(&mut *phase, Phase::Idle) {
                hotkey::set_recording(false);
                std::thread::spawn(move || drop(rec.stop()));
                status(app, "idle", Some("cancelled".into()));
            }
        }
        _ => {}
    }
}

/// Custom key combos go through the OS shortcut API, which only reports
/// press/release, so hands-free is a quick tap instead of +Space.
fn on_custom_shortcut(app: &AppHandle, pressed: bool) {
    let state = app.state::<AppState>();
    let action = {
        let phase = state.phase.lock().unwrap();
        match (&*phase, pressed) {
            (Phase::Idle, true) => Some(Action::Start),
            (Phase::Recording { hands_free: true, .. }, true) => Some(Action::Finish { command: false }),
            (Phase::Recording { hands_free: false, started, .. }, false) => Some(if started.elapsed() < TAP_THRESHOLD {
                Action::HandsFree
            } else {
                Action::Finish { command: false }
            }),
            _ => None,
        }
    };
    if let Some(a) = action {
        act(app, a);
    }
}

fn llm_target(s: &Settings, for_command: bool) -> Option<(String, Option<String>, String)> {
    let groq = || {
        Some((
            "https://api.groq.com/openai/v1".to_string(),
            Some(s.groq_api_key.trim().to_string()),
            s.groq_cleanup_model.clone(),
        ))
    };
    let ollama = || Some((format!("{}/v1", s.ollama_url.trim_end_matches('/')), None, s.ollama_model.clone()));
    let has_key = !s.groq_api_key.trim().is_empty();
    match s.cleanup.as_str() {
        "groq" if has_key => groq(),
        "ollama" => ollama(),
        // Command mode needs some model even when cleanup is switched off.
        _ if for_command && has_key => groq(),
        _ => None,
    }
}

async fn process(app: &AppHandle, rec: audio::Recorder, command: bool, target: style::Target) -> anyhow::Result<()> {
    status(app, "processing", None);
    let samples = tauri::async_runtime::spawn_blocking(move || rec.stop()).await?;
    if samples.len() < MIN_SAMPLES {
        return Ok(());
    }
    let state = app.state::<AppState>();
    let s = state.settings.lock().unwrap().clone();
    let terms = s.dictionary_terms();

    let (raw, engine) = transcribe_any(app, &s, samples, &terms).await?;
    if raw.is_empty() || is_noise(&raw) {
        return Ok(());
    }
    let hint = style::hint(target.kind);

    let text = if command {
        let Some((base, key, model)) = llm_target(&s, true) else {
            anyhow::bail!("Command mode needs a Groq key or Ollama (see Cleanup)");
        };
        let selection = tauri::async_runtime::spawn_blocking(|| {
            hotkey::wait_modifiers_released(Duration::from_secs(2));
            paste::copy_selection()
        })
        .await??;
        let t = cleanup::Target { base_url: &base, key: key.as_deref(), model: &model };
        cleanup::command(&state.http, t, &raw, selection.as_deref(), hint).await?
    } else {
        let formatted = if s.spoken_formatting { style::spoken_formatting(&raw) } else { raw.clone() };
        let cleaned = match llm_target(&s, false) {
            Some((base, key, model)) => {
                let t = cleanup::Target { base_url: &base, key: key.as_deref(), model: &model };
                cleanup::run(&state.http, t, &formatted, &terms, s.hinglish_roman, hint)
                    .await
                    .unwrap_or_else(|e| {
                        eprintln!("cleanup failed, using raw text: {e}");
                        formatted.clone()
                    })
            }
            None => formatted,
        };
        let expanded = cleanup::expand_snippets(&cleaned, &s.snippet_pairs());
        style::finish(&expanded, target.kind)
    };

    let to_type = text.clone();
    let restore = s.restore_clipboard;
    tauri::async_runtime::spawn_blocking(move || {
        hotkey::wait_modifiers_released(Duration::from_secs(2));
        paste::type_text(&to_type, restore)
    })
    .await??;

    let item = HistoryItem {
        text,
        raw,
        at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
        engine: engine.into(),
        app: target.app,
        command,
    };
    {
        let mut h = state.history.lock().unwrap();
        h.insert(0, item.clone());
        h.truncate(50);
    }
    let _ = app.emit("bolo://history", item);
    Ok(())
}

async fn transcribe_any(
    app: &AppHandle,
    s: &Settings,
    samples: Vec<f32>,
    terms: &[String],
) -> anyhow::Result<(String, &'static str)> {
    let state = app.state::<AppState>();
    let use_groq = match s.engine.as_str() {
        "groq" => true,
        "local" => false,
        _ => !s.groq_api_key.trim().is_empty(),
    };
    if use_groq {
        match transcribe::groq(&state.http, s.groq_api_key.trim(), &s.groq_whisper_model, &samples, &s.language, terms).await {
            Ok(t) => return Ok((t, "groq")),
            Err(e) if s.engine == "auto" && models::path(app, &s.local_model).exists() => {
                eprintln!("Groq failed, falling back to local: {e}");
            }
            Err(e) => return Err(e),
        }
    }
    Ok((transcribe_local(app, s, samples, terms.to_vec()).await?, "local"))
}

async fn transcribe_local(
    app: &AppHandle,
    s: &Settings,
    samples: Vec<f32>,
    terms: Vec<String>,
) -> anyhow::Result<String> {
    let model = models::path(app, &s.local_model);
    if !model.exists() {
        anyhow::bail!("Download a speech model in Bolo's settings first");
    }
    let lang = s.language.clone();
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app2.state::<AppState>();
        transcribe::local(&state.whisper, &model, &samples, &lang, &terms)
    })
    .await?
}

/// Whisper's usual output for silence.
fn is_noise(t: &str) -> bool {
    let t = t.trim().trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
    t.is_empty()
        || matches!(t.as_str(), "blank_audio" | "you" | "thank you" | "thanks for watching" | "music")
}

/// Hold presets ("hold:ctrl+win") use the keyboard hook; anything else is a
/// regular key combo for the OS shortcut API.
fn apply_hotkey(app: &AppHandle, hotkey: &str) -> Result<(), String> {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    match hotkey::Preset::parse(hotkey) {
        Some(p) => {
            hotkey::set_preset(Some(p));
            Ok(())
        }
        None => {
            hotkey::set_preset(None);
            gs.register(hotkey).map_err(|e| format!("Could not use shortcut \"{hotkey}\": {e}"))
        }
    }
}

// ---------- commands for the settings window ----------

#[tauri::command]
fn get_settings(state: tauri::State<AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
fn platform() -> &'static str {
    std::env::consts::OS
}

#[tauri::command]
fn save_settings(app: AppHandle, state: tauri::State<AppState>, new: Settings) -> Result<(), String> {
    let old = state.settings.lock().unwrap().clone();
    if old.hotkey != new.hotkey {
        if let Err(e) = apply_hotkey(&app, &new.hotkey) {
            let _ = apply_hotkey(&app, &old.hotkey);
            return Err(e);
        }
    }
    if old.autostart != new.autostart {
        let al = app.autolaunch();
        let r = if new.autostart { al.enable() } else { al.disable() };
        r.map_err(|e| format!("Start at login: {e}"))?;
    }
    settings::save(&app, &new).map_err(|e| e.to_string())?;
    *state.settings.lock().unwrap() = new;
    Ok(())
}

#[tauri::command]
fn list_models(app: AppHandle) -> Vec<models::ModelStatus> {
    models::list(&app)
}

#[tauri::command]
async fn download_model(app: AppHandle, name: String) -> Result<(), String> {
    let http = app.state::<AppState>().http.clone();
    models::download(&app, &http, &name).await.map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_model(app: AppHandle, name: String) -> Result<(), String> {
    let state = app.state::<AppState>();
    let path = models::path(&app, &name);
    let mut cache = state.whisper.lock().unwrap();
    if cache.as_ref().map(|(p, _)| p == &path).unwrap_or(false) {
        *cache = None;
    }
    std::fs::remove_file(path).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_history(state: tauri::State<AppState>) -> Vec<HistoryItem> {
    state.history.lock().unwrap().clone()
}

// ---------- windows and tray ----------

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn place_pill(app: &AppHandle) {
    let Some(pill) = app.get_webview_window("pill") else { return };
    let monitor = pill.primary_monitor().ok().flatten().or_else(|| pill.current_monitor().ok().flatten());
    if let (Some(m), Ok(size)) = (monitor, pill.outer_size()) {
        let scale = m.scale_factor();
        let x = m.position().x + (m.size().width as i32 - size.width as i32) / 2;
        let y = m.position().y + m.size().height as i32 - size.height as i32 - (72.0 * scale) as i32;
        let _ = pill.set_position(PhysicalPosition::new(x, y));
    }
    let _ = pill.set_ignore_cursor_events(true);
    let _ = pill.show();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main(app)))
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec!["--hidden"])))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    on_custom_shortcut(app, event.state() == ShortcutState::Pressed);
                })
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let handle = app.handle().clone();
            let s = settings::load(&handle);
            let hotkey_str = s.hotkey.clone();
            app.manage(AppState {
                settings: Mutex::new(s),
                phase: Mutex::new(Phase::Idle),
                whisper: Mutex::new(None),
                history: Mutex::new(Vec::new()),
                http: reqwest::Client::builder().timeout(Duration::from_secs(600)).build()?,
            });

            // Keyboard hook → actions, handled on their own thread so the
            // hook callback itself stays instant.
            let (tx, rx) = std::sync::mpsc::channel::<Action>();
            let dispatch = handle.clone();
            std::thread::spawn(move || {
                for action in rx {
                    act(&dispatch, action);
                }
            });
            if let Err(e) = hotkey::start(tx) {
                eprintln!("keyboard hook: {e}");
                status(&handle, "error", Some(e));
            }
            if let Err(e) = apply_hotkey(&handle, &hotkey_str) {
                eprintln!("{e}");
            }

            let open = MenuItem::with_id(app, "open", "Open Bolo", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;
            TrayIconBuilder::with_id("tray")
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("Bolo — hold your shortcut and speak")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, e| match e.id.as_ref() {
                    "open" => show_main(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, e| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                        show_main(tray.app_handle());
                    }
                })
                .build(app)?;

            // Load the offline model now so the first dictation isn't slow.
            let warm = handle.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let state = warm.state::<AppState>();
                let s = state.settings.lock().unwrap().clone();
                let model = models::path(&warm, &s.local_model);
                let wants_local = s.engine == "local" || s.groq_api_key.trim().is_empty();
                if wants_local && model.exists() {
                    let silence = vec![0.0f32; audio::TARGET_RATE as usize];
                    let _ = transcribe::local(&state.whisper, &model, &silence, "en", &[]);
                }
            });

            place_pill(&handle);
            if !std::env::args().any(|a| a == "--hidden") {
                show_main(&handle);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the settings window keeps Bolo running in the tray.
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            platform,
            list_models,
            download_model,
            delete_model,
            get_history
        ])
        .run(tauri::generate_context!())
        .expect("error while running Bolo");
}
