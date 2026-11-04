import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";

type Settings = Record<string, string | boolean> & {
  hotkey: string;
  local_model: string;
  cleanup: string;
};

type Model = { name: string; label: string; size_mb: number; note: string; downloaded: boolean };
type HistoryItem = { id: number; status: string; error: string; has_audio: boolean; text: string; raw: string; at: number; engine: string; app: string; command: boolean };

const $ = <T extends HTMLElement>(sel: string) => document.querySelector<T>(sel)!;
const saveState = $("#save-state");
let settings: Settings;
let saveTimer: number | undefined;
let saveQueue: Promise<void> = Promise.resolve();
const downloading = new Map<string, number>();

// ---------- tabs ----------
document.querySelectorAll<HTMLButtonElement>("[data-tab]").forEach((tab) => {
  tab.addEventListener("click", () => {
    document.querySelectorAll("[data-tab]").forEach((t) => t.setAttribute("aria-selected", String(t === tab)));
    document.querySelectorAll<HTMLElement>("[data-panel]").forEach((p) => (p.hidden = p.dataset.panel !== tab.dataset.tab));
    if (tab.dataset.tab === "history") renderHistory();
  });
});

document.addEventListener("click", (e) => {
  const a = (e.target as HTMLElement).closest<HTMLAnchorElement>("a[data-external]");
  if (a) {
    e.preventDefault();
    openUrl(a.href);
  }
});

// ---------- settings ----------
function showStatus(text: string, error = false) {
  saveState.textContent = text;
  saveState.classList.toggle("error", error);
}

window.addEventListener("unhandledrejection", (event) => {
  event.preventDefault();
  showStatus(String(event.reason), true);
});

async function save() {
  try {
    const snapshot = structuredClone(settings);
    const next = saveQueue.catch(() => {}).then(() => invoke<void>("save_settings", { new: snapshot }));
    saveQueue = next;
    await next;
    showStatus("Saved");
    window.setTimeout(() => saveState.textContent === "Saved" && showStatus(""), 1500);
  } catch (err) {
    showStatus(String(err), true);
    throw err;
  }
}

function queueSave() {
  window.clearTimeout(saveTimer);
  saveTimer = window.setTimeout(() => { void save().catch(() => {}); }, 400);
}

// ---------- shortcut presets ----------
type Preset = { value: string; label: string; keys: string[]; extra: string; help?: string };
let os = "windows";

function presets(): Preset[] {
  if (os === "macos")
    return [
      { value: "hold:fn", label: "Fn / 🌐 (recommended)", keys: ["Fn"], extra: "Option", help: "Set “Press 🌐 key to” to “Do nothing” in System Settings → Keyboard." },
      { value: "hold:right-cmd", label: "Right ⌘ Command", keys: ["Right ⌘"], extra: "Option" },
      { value: "hold:right-alt", label: "Right ⌥ Option", keys: ["Right ⌥"], extra: "Shift" },
    ];
  const win = os === "windows" ? "Win" : "Super";
  return [
    { value: os === "windows" ? "hold:ctrl+win" : "hold:ctrl+super", label: `Ctrl + ${win} (recommended)`, keys: ["Ctrl", win], extra: "Alt" },
    { value: "hold:right-ctrl", label: "Right Ctrl", keys: ["Right Ctrl"], extra: "Alt" },
    { value: "hold:right-alt", label: "Right Alt", keys: ["Right Alt"], extra: "Shift" },
  ];
}

const kbd = (keys: string[]) => keys.map((k) => `<kbd>${k}</kbd>`).join(" + ");

function renderShortcut() {
  const sel = $<HTMLSelectElement>("#hotkey-preset");
  const list = presets();
  if (!sel.options.length) {
    for (const p of [...list, { value: "custom", label: "Custom key combo…" }]) {
      sel.add(new Option(p.label, p.value));
    }
    sel.addEventListener("change", async () => {
      const previous = settings.hotkey;
      settings.hotkey = sel.value === "custom" ? "Ctrl+Shift+Space" : sel.value;
      try {
        await save();
      } catch {
        settings.hotkey = previous;
      }
      renderShortcut();
    });
  }
  const preset = list.find((p) => p.value === settings.hotkey);
  sel.value = preset ? preset.value : "custom";
  $("#custom-row").hidden = !!preset;
  $("#hotkey").textContent = settings.hotkey;
  $("#hotkey-help").textContent = preset?.help ?? "";

  const cheats = $("#cheats");
  const card = (title: string, keys: string, note: string) =>
    `<li><span class="cheat-title">${title}</span><span class="cheat-keys">${keys}</span><span class="cheat-note">${note}</span></li>`;
  if (preset) {
    cheats.innerHTML = [
      card("Dictate", kbd(preset.keys), "Hold, speak, let go"),
      card("Hands-free", kbd([...preset.keys, "Space"]), `Press ${preset.keys.join(" + ")} again to stop`),
      card("Edit by voice", kbd([...preset.keys, preset.extra]), "Select text first, then hold"),
      card("Cancel", kbd(["Esc"]), "While recording or processing"),
    ].join("");
    $("#lede").innerHTML = `Hold ${kbd(preset.keys)} anywhere, speak, and let go.`;
  } else {
    cheats.innerHTML = card("Cancel", kbd(["Esc"]), "While Bolo is listening");
    $("#lede").innerHTML = `Hold <kbd></kbd> anywhere, speak, and let go. Tap it once for hands-free.`;
    $("#lede kbd")!.textContent = settings.hotkey;
  }
}

function applyVisibility() {
  document.querySelectorAll<HTMLElement>("[data-show]").forEach((el) => {
    const [key, vals] = el.dataset.show!.split("=");
    el.hidden = !vals.split(",").includes(String(settings[key]));
  });
}

function bindFields() {
  document.querySelectorAll<HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement>("[data-key]").forEach((el) => {
    const key = el.dataset.key!;
    if (el instanceof HTMLInputElement && el.type === "checkbox") {
      el.checked = Boolean(settings[key]);
      el.addEventListener("change", () => {
        settings[key] = el.checked;
        applyVisibility();
        queueSave();
      });
    } else {
      el.value = String(settings[key] ?? "");
      el.addEventListener("input", () => {
        settings[key] = el.value;
        applyVisibility();
        queueSave();
      });
    }
  });
}

// ---------- shortcut recorder ----------
const hotkeyBtn = $<HTMLButtonElement>("#hotkey");
let listening = false;

function keyName(e: KeyboardEvent): string | null {
  const c = e.code;
  if (/^(Control|Shift|Alt|Meta|OS)(Left|Right)?$/.test(c) || ["ControlLeft", "ControlRight"].includes(c)) return null;
  if (c.startsWith("Key")) return c.slice(3);
  if (c.startsWith("Digit")) return c.slice(5);
  return c;
}

hotkeyBtn.addEventListener("click", () => {
  listening = true;
  hotkeyBtn.classList.add("listening");
  hotkeyBtn.textContent = "Press keys…";
});

hotkeyBtn.addEventListener("blur", () => {
  if (!listening) return;
  listening = false;
  hotkeyBtn.classList.remove("listening");
  hotkeyBtn.textContent = settings.hotkey;
});

window.addEventListener("keydown", async (e) => {
  if (!listening) return;
  e.preventDefault();
  if (e.key === "Escape") {
    hotkeyBtn.blur();
    return;
  }
  const key = keyName(e);
  if (!key) return; // still waiting for a non-modifier key
  const mods = [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Super"].filter(Boolean);
  if (mods.length === 0 && !/^F\d+$/.test(key)) {
    hotkeyBtn.textContent = "Add Ctrl, Alt or Shift";
    return;
  }
  const previous = settings.hotkey;
  settings.hotkey = [...mods, key].join("+");
  listening = false;
  hotkeyBtn.classList.remove("listening");
  try {
    await save();
  } catch {
    settings.hotkey = previous;
  }
  renderShortcut();
  hotkeyBtn.blur();
});

// ---------- models ----------
async function renderModels() {
  const models = await invoke<Model[]>("list_models");
  const list = $("#models");
  list.replaceChildren(
    ...models.map((m) => {
      const li = document.createElement("li");
      li.className = "model" + (settings.local_model === m.name ? " active" : "");
      const pct = downloading.get(m.name);

      const radio = document.createElement("input");
      radio.type = "radio";
      radio.name = "model";
      radio.checked = settings.local_model === m.name;
      radio.disabled = !m.downloaded;
      radio.setAttribute("aria-label", `Use ${m.label}`);
      radio.addEventListener("change", () => {
        settings.local_model = m.name;
        queueSave();
        renderModels();
      });

      const meta = document.createElement("div");
      meta.className = "meta";
      meta.innerHTML = `<span>${m.label} <small>· ${m.size_mb} MB</small></span><small>${m.note}</small>`;

      const actions = document.createElement("div");
      actions.className = "actions";
      if (pct !== undefined) {
        actions.innerHTML = `<div class="progress" role="progressbar" aria-valuenow="${pct}"><div style="width:${pct}%"></div></div>`;
      } else if (m.downloaded) {
        const del = document.createElement("button");
        del.className = "btn link";
        del.textContent = "Remove";
        del.addEventListener("click", async () => {
          await invoke("delete_model", { name: m.name });
          renderModels();
        });
        actions.append(del);
      } else {
        const dl = document.createElement("button");
        dl.className = "btn primary";
        dl.textContent = "Download";
        dl.addEventListener("click", () => download(m.name));
        actions.append(dl);
      }
      li.append(radio, meta, actions);
      return li;
    }),
  );
}

async function download(name: string) {
  downloading.set(name, 0);
  renderModels();
  try {
    await invoke("download_model", { name });
    const models = await invoke<Model[]>("list_models");
    const current = models.find((m) => m.name === settings.local_model);
    if (!current?.downloaded) {
      settings.local_model = name;
      await save();
    }
  } catch (err) {
    showStatus(`Download failed: ${err}`, true);
  } finally {
    downloading.delete(name);
    renderModels();
  }
}

listen<{ name: string; downloaded: number; total: number }>("bolo://download", (e) => {
  const { name, downloaded, total } = e.payload;
  const pct = total ? Math.round((downloaded / total) * 100) : 0;
  if (downloading.get(name) === pct) return;
  downloading.set(name, pct);
  renderModels();
});

// ---------- history ----------

function actionButton(label: string, fn: () => Promise<unknown>) {
  const button = document.createElement("button"); button.className = "btn"; button.textContent = label;
  button.addEventListener("click", async () => { button.disabled = true; try { await fn(); } catch (e) { showStatus(String(e), true); } finally { button.disabled = false; } });
  return button;
}
async function renderHistory() {
  const items = await invoke<HistoryItem[]>("get_history");
  const query = $<HTMLInputElement>("#history-search").value.toLowerCase();
  const list = $("#history");
  list.replaceChildren(...items.filter(h => (h.text + h.raw + h.app).toLowerCase().includes(query)).map(h => {
    const li = document.createElement("li");
    const text = document.createElement("div"); text.className = "transcript"; text.textContent = h.text || h.raw || "Recording saved — retry to transcribe.";
    const when = document.createElement("span"); when.className = "when";
    when.textContent = [new Date(h.at).toLocaleString(), h.app, h.engine, h.status].filter(Boolean).join(" · ");
    li.append(text, when);
    if (h.error) { const error = document.createElement("p"); error.className = "error"; error.textContent = h.error; li.append(error); }
    const actions = document.createElement("div"); actions.className = "actions";
    if (h.text || h.raw) actions.append(actionButton("Copy", async () => { await navigator.clipboard.writeText(h.text || h.raw); showStatus("Copied"); }));
    if (h.has_audio && !h.command) actions.append(actionButton("Retry", async () => { await invoke("retry_dictation", { id: h.id }); await renderHistory(); }));
    actions.append(actionButton("Delete", async () => { await invoke("delete_history", { id: h.id }); await renderHistory(); }));
    const details = document.createElement("details"); const summary = document.createElement("summary"); summary.textContent = "Raw transcript & spelling correction"; details.append(summary);
    const raw = document.createElement("p"); raw.textContent = h.raw; raw.className = "transcript"; details.append(raw);
    const heard = document.createElement("input"); heard.type = "text"; heard.placeholder = "Heard as"; heard.setAttribute("aria-label", "Incorrect word or phrase"); heard.maxLength = 120;
    const wanted = document.createElement("input"); wanted.type = "text"; wanted.placeholder = "Always write as"; wanted.setAttribute("aria-label", "Preferred spelling"); wanted.maxLength = 120;
    const teach = document.createElement("div"); teach.className = "actions"; teach.append(heard, wanted, actionButton("Remember correction", async () => {
      window.clearTimeout(saveTimer); await save();
      await invoke("teach_correction", { heard: heard.value, wanted: wanted.value });
      const latest = await invoke<Settings>("get_settings"); settings.corrections = latest.corrections;
      $<HTMLTextAreaElement>('[data-key="corrections"]').value = String(latest.corrections);
      showStatus("Correction remembered"); heard.value = ""; wanted.value = "";
    })); details.append(teach); li.append(actions, details); return li;
  }));
  if (!list.childElementCount) { const empty=document.createElement("li"); empty.className="empty"; empty.textContent = items.length ? "No matching dictations." : "Nothing yet. Hold your shortcut and speak."; list.append(empty); }
}
$("#history-search").addEventListener("input", () => void renderHistory());
$("#clear-history").addEventListener("click", async () => {
  const button = $("#clear-history");
  if (button.textContent !== "Confirm clear") { button.textContent = "Confirm clear"; window.setTimeout(() => button.textContent = "Clear history", 4000); return; }
  try { await invoke("delete_history", { id: null }); await renderHistory(); } catch(e) { showStatus(String(e), true); }
  button.textContent = "Clear history";
});

listen("bolo://history", () => {
  if (!$<HTMLElement>('[data-panel="history"]').hidden) renderHistory();
});

listen<{ state: string; message?: string }>("bolo://state", (e) => {

  const busy = e.payload.state === "processing" || e.payload.state === "recording";
  $("#activity").hidden = !busy;
  $("#activity-text").textContent = e.payload.message || (e.payload.state === "recording" ? "Listening…" : "Processing…");
  if (e.payload.state === "error") { showStatus(e.payload.message ?? "Error", true); }

});


$("#cancel-processing").addEventListener("click", () => void invoke("cancel_processing"));
let testingMic = false;
let micTimer: number | undefined;
void listen("bolo://mic-test-ended", () => {
  window.clearTimeout(micTimer);
  testingMic = false;
  $("#test-microphone").textContent = "Test microphone";
  $<HTMLMeterElement>("#mic-level").value = 0;
});
async function loadMicrophones() {
  try {
    const names = await invoke<string[]>("list_microphones"); const select=$<HTMLSelectElement>("#microphone");
    select.replaceChildren(new Option("System default", ""), ...names.map(n => new Option(n,n)));
    const selected=String(settings.microphone ?? "");
    if (selected && !names.includes(selected)) select.add(new Option(selected + " (unavailable)", selected));
    select.value=selected;
  } catch(e) { showStatus(String(e),true); }
}
$("#refresh-microphones").addEventListener("click", () => void loadMicrophones());
async function stopMicTest() {
  window.clearTimeout(micTimer);
  await invoke("microphone_test", { start:false }); testingMic=false;
  $("#test-microphone").textContent="Test microphone"; $<HTMLMeterElement>("#mic-level").value=0;
}
$("#test-microphone").addEventListener("click", async () => {
  try {
    if(testingMic) { await stopMicTest(); return; }
    window.clearTimeout(saveTimer); await save(); await invoke("microphone_test", { start:true });
    testingMic=true; $("#test-microphone").textContent="Stop test";
    micTimer = window.setTimeout(() => { if(testingMic) void stopMicTest().catch(e=>showStatus(String(e),true)); },5000);
  } catch(e) { showStatus(String(e),true); }
});
listen<number>("bolo://level", e => { if(testingMic) $<HTMLMeterElement>("#mic-level").value=Math.min(1,Math.sqrt(e.payload)*3.2); });
async function checkForUpdates() {
  const button=$<HTMLButtonElement>("#check-update"); button.disabled=true;
  $("#update-result").textContent="Checking GitHub releases…";
  try {
    const result=await invoke<{current:string;latest:string;available:boolean;url:string}>("check_update");
    $("#update-result").textContent=result.available ? 'Update '+result.latest+' is available. ' : 'Installed: '+result.current+'. Latest published: '+result.latest+'. No newer release.';
    if(result.available) { const link=document.createElement("button"); link.className="btn"; link.textContent="Open release"; link.addEventListener("click",()=>void openUrl(result.url)); $("#update-result").append(link); }
  } catch { $("#update-result").textContent="Could not check updates. Try again when you are online."; }
  finally { button.disabled=false; }
}
$("#check-update").addEventListener("click", () => void checkForUpdates());
// ---------- boot ----------
(async () => {
  settings = await invoke<Settings>("get_settings");
  os = await invoke<string>("platform");
  const initialStatus = await invoke<{ state: string; message?: string }>("get_status");
  if (initialStatus.state === "error") showStatus(initialStatus.message ?? "Bolo could not start", true);

  for (const select of document.querySelectorAll<HTMLSelectElement>('.style-grid select')) {
    for (const [value,label] of [["formal","Formal — Hello, team."],["casual","Casual — Hello, team"],["lowercase","Lowercase — hello, team"],["literal","Literal — preserve wording"]]) select.add(new Option(label,value));
  }
  await loadMicrophones();
  $("#context-row").hidden = os !== "windows";
  bindFields();
  applyVisibility();
  renderShortcut();
  if(settings.check_updates) void checkForUpdates();
  await renderModels();
  const models = await invoke<Model[]>("list_models");
  if (!models.some((m) => m.downloaded) && !String(settings.groq_api_key).trim()) {
    showStatus("Download a model or add a Groq key to start");
  }
})();
