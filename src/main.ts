import { textChanges } from "./text-changes";
import { refreshMeetings } from "./meetings";
import { libraryFromFields, mergeLibrary, parseLibrary, type WordLibrary, type WordFields } from "./word-library";
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
const tabs = Array.from(document.querySelectorAll<HTMLButtonElement>("[data-tab]"));
function selectTab(tab: HTMLButtonElement) {
  stopPlayback();
  if (tab.dataset.tab === "settings") void refreshBudget();
  tabs.forEach(t => { t.setAttribute("aria-selected", String(t === tab)); t.tabIndex = t === tab ? 0 : -1; });
  document.querySelectorAll<HTMLElement>("[data-panel]").forEach(p => p.hidden = p.dataset.panel !== tab.dataset.tab);
  $("#section-label").textContent = tab.textContent;
  window.scrollTo({ top: 0 });
  if (tab.dataset.tab === "history") void renderHistory();
  if (tab.dataset.tab === "meetings") void refreshMeetings();
}
tabs.forEach((tab, index) => {
  tab.addEventListener("click", () => selectTab(tab));
  tab.addEventListener("keydown", e => {
    const next = e.key === "Home" ? 0 : e.key === "End" ? tabs.length - 1
      : ["ArrowRight", "ArrowDown"].includes(e.key) ? (index + 1) % tabs.length
      : ["ArrowLeft", "ArrowUp"].includes(e.key) ? (index + tabs.length - 1) % tabs.length : -1;
    if (next < 0) return;
    e.preventDefault(); tabs[next].focus(); selectTab(tabs[next]);
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
  $("#engine-help").textContent = settings.engine === "local"
    ? "Audio stays on this computer. AI cleanup, if enabled, still sends text to OpenAI."
    : "OpenAI handles speech recognition. Mixed mode uses automatic detection with English, Hindi and Punjabi context.";
  const connected = Boolean(String(settings.openai_api_key ?? "").trim());
  $("#connection-title").textContent = connected ? "OpenAI key saved" : "Connect OpenAI";
  $("#connection-help").textContent = connected
    ? "Your key is stored on this computer. Requests use your OpenAI API balance; availability is checked when you dictate."
    : "Add a key for cloud dictation and Roman Hindi / Punjabi. You can also use offline recognition.";
  $("#roman-help").hidden = !settings.hinglish_roman || (settings.cleanup !== "off" && connected);
  document.querySelectorAll<HTMLElement>("[data-show]").forEach((el) => {
    const [key, vals] = el.dataset.show!.split("=");
    el.hidden = !vals.split(",").includes(String(settings[key]));
  });
}

function bindFields() {
  document.querySelectorAll<HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement>("[data-key]").forEach((el) => {
    const key = el.dataset.key!;
    if (!el.labels?.length && !el.hasAttribute("aria-label")) {
      const label = el.closest(".row")?.querySelector(".label > span")?.textContent
        ?? el.closest(".group")?.querySelector("h2")?.textContent;
      if (label) el.setAttribute("aria-label", label);
    }
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

// ---------- portable word library ----------
let pendingLibrary: WordLibrary | undefined;
const wordFields = (): WordFields => ({
  dictionary: String(settings.dictionary ?? ""), corrections: String(settings.corrections ?? ""),
  snippets: String(settings.snippets ?? ""),
});
function dismissImport() { pendingLibrary = undefined; $("#words-preview").hidden = true; }
for (const key of ["dictionary", "corrections", "snippets"])
  document.querySelector('[data-key="' + key + '"]')!.addEventListener("input", dismissImport);
$("#export-words").addEventListener("click", () => {
  try {
    const library = libraryFromFields(wordFields());
    const url = URL.createObjectURL(new Blob([JSON.stringify(library, null, 2)], { type: "application/json" }));
    const link = document.createElement("a"); link.href = url; link.download = "bolo-words.json";
    document.body.append(link); link.click(); link.remove();
    window.setTimeout(() => URL.revokeObjectURL(url), 30000);
    $("#words-result").textContent = "Word library exported. Share this file only with people who should see these entries.";
  } catch (e) { $("#words-result").textContent = String(e); }
});
$("#import-words").addEventListener("click", () => $<HTMLInputElement>("#words-file").click());
$("#words-file").addEventListener("change", async () => {
  dismissImport(); $("#words-result").textContent = "";
  const input = $<HTMLInputElement>("#words-file"), file = input.files?.[0]; input.value = "";
  if (!file) return;
  try {
    if (file.size > 3 * 1024 * 1024) throw new Error("Choose a word library smaller than 3 MB.");
    const incoming = parseLibrary(JSON.parse(await file.text()));
    const merged = mergeLibrary(wordFields(), incoming);
    pendingLibrary = incoming;
    $("#words-summary").textContent = merged.added + " new entries · " + merged.skipped + " skipped. Existing entries are kept.";
    $("#words-conflicts").replaceChildren(...merged.conflicts.map(trigger => {
      const li = document.createElement("li"); li.textContent = "Kept existing replacement for: " + trigger; return li;
    }));
    $<HTMLButtonElement>("#apply-words").disabled = !merged.added;
    $("#words-preview").hidden = false;
  } catch (e) { $("#words-result").textContent = String(e); }
});
$("#cancel-words").addEventListener("click", dismissImport);
$("#apply-words").addEventListener("click", async () => {
  if (!pendingLibrary) return;
  const before = wordFields(), merged = mergeLibrary(before, pendingLibrary);
  const button = $<HTMLButtonElement>("#apply-words"); button.disabled = true;
  window.clearTimeout(saveTimer);
  const editors = Array.from(document.querySelectorAll<HTMLTextAreaElement>('[data-panel="words"] textarea'));
  editors.forEach(el => el.disabled = true);
  Object.assign(settings, merged.fields);
  try {
    await save();
    for (const key of ["dictionary", "corrections", "snippets"] as const)
      $<HTMLTextAreaElement>('[data-key="' + key + '"]').value = String(settings[key]);
    dismissImport(); $("#words-result").textContent = merged.added + " entries added. Existing entries were kept.";
  } catch (e) {
    Object.assign(settings, before);
    $("#words-result").textContent = "Import could not be saved: " + String(e);
  } finally { button.disabled = false; editors.forEach(el => el.disabled = false); }
});

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
  if (downloading.has(name)) return;
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
let playingAudio: HTMLAudioElement | undefined;
let playbackUrl: string | undefined;
let historyGeneration = 0;
function stopPlayback() {
  if (playingAudio) {
    playingAudio.pause();
    playingAudio.removeAttribute("src");
    playingAudio.load();
    playingAudio.remove();
  }
  if (playbackUrl) URL.revokeObjectURL(playbackUrl);
  playingAudio = undefined; playbackUrl = undefined;
}
async function renderHistory() {
  const generation = ++historyGeneration;
  stopPlayback();
  const items = await invoke<HistoryItem[]>("get_history");
  if (generation !== historyGeneration) return;
  const query = $<HTMLInputElement>("#history-search").value.toLowerCase();
  const list = $("#history");
  list.replaceChildren(...items.filter(h => (h.text + h.raw + h.app).toLowerCase().includes(query)).map(h => {
    const li = document.createElement("li");
    const text = document.createElement("div"); text.className = "transcript"; text.textContent = h.text || h.raw || "Recording saved — retry to transcribe.";
    const when = document.createElement("span"); when.className = "when";
    const statusNames: Record<string,string> = { review:"Review before pasting", pasted:"Pasted", ready:"Ready to copy", failed:"Needs attention", cancelled:"Cancelled", processing:"Processing", interrupted:"Interrupted" };
    when.textContent = [new Date(h.at).toLocaleString(), h.app, statusNames[h.status] ?? h.status].filter(Boolean).join(" · ");
    li.append(text, when);
    if (h.error) { const error = document.createElement("p"); error.className = "error"; error.textContent = h.error; li.append(error); }
    const actions = document.createElement("div"); actions.className = "actions";
    if (h.has_audio) actions.append(actionButton("Listen", async () => {
      const data = await invoke<ArrayBuffer>("history_audio", { id: h.id });
      if (generation !== historyGeneration || li.closest("[hidden]")) return;
      stopPlayback();
      playbackUrl = URL.createObjectURL(new Blob([data], { type: "audio/wav" }));
      playingAudio = document.createElement("audio");
      playingAudio.controls = true;
      playingAudio.setAttribute("aria-label", "Original recording");
      playingAudio.src = playbackUrl;
      li.append(playingAudio);
      showStatus("Original recording · playback uses no API credits");
      playingAudio.focus();
    }));
    if (h.text || h.raw) actions.append(actionButton("Copy", async () => { await navigator.clipboard.writeText(h.text || h.raw); showStatus("Copied"); }));
    if (h.has_audio && !h.command) actions.append(actionButton("Retry", async () => { window.clearTimeout(saveTimer); await save(); await invoke("retry_dictation", { id: h.id }); await renderHistory(); }));
    actions.append(actionButton("Delete", async () => { await invoke("delete_history", { id: h.id }); await renderHistory(); }));
    const details = document.createElement("details"); const summary = document.createElement("summary"); summary.textContent = "Original transcript & corrections"; details.append(summary);
    const raw = document.createElement("p"); raw.textContent = h.raw; raw.className = "transcript"; details.append(raw);
    if(h.raw)details.append(actionButton("Copy original",async()=>{await navigator.clipboard.writeText(h.raw);showStatus("Original recognition copied; formatting and script conversion are not applied.");}));
    if(h.raw&&h.text&&h.raw!==h.text){
      const diff=document.createElement("details");const label=document.createElement("summary");label.textContent="What changed: recognition → final text";diff.append(label);
      const content=document.createElement("p");content.className="transcript";
      for(const part of textChanges(h.raw,h.text)){const node=document.createElement(part.kind==='removed'?'del':part.kind==='added'?'ins':'span');node.textContent=part.text;content.append(node);}diff.append(content);details.append(diff);
    }
    if(h.text){
      const review=document.createElement("details");review.open=h.status==='review';const label=document.createElement("summary");label.textContent="Review and edit";review.append(label);
      const editor=document.createElement("textarea");editor.value=h.text;editor.setAttribute("aria-label","Reviewed dictation");editor.maxLength=200000;review.append(editor);
      review.append(actionButton("Save reviewed text",async()=>{await invoke("save_review",{id:h.id,text:editor.value});h.text=editor.value;text.textContent=editor.value;showStatus("Saved. Return to your text field and use the paste-last shortcut, or copy this draft.");await renderHistory();}));
      review.append(actionButton("Save and copy",async()=>{await invoke("save_review",{id:h.id,text:editor.value});h.text=editor.value;text.textContent=editor.value;await navigator.clipboard.writeText(editor.value);showStatus("Reviewed text copied. Paste it into your chosen field.");await renderHistory();}));details.append(review);if(h.status==='review')details.open=true;
    }
    const source = document.createElement("small"); source.textContent = h.engine ? "Recognition: " + h.engine : ""; details.append(source);
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

listen("bolo://review", () => { selectTab($<HTMLButtonElement>('#tab-history')); });

listen("bolo://history", () => {
  if (!$<HTMLElement>('[data-panel="history"]').hidden) renderHistory();
});

function applyActivity(status: { state: string; message?: string }) {
  void refreshBudget();
  if (status.state === "recording") stopPlayback();
  const busy = status.state === "processing" || status.state === "recording";
  $("#activity").hidden = !busy;
  $("#activity-text").textContent = status.message || (status.state === "recording" ? "Listening…" : "Processing…");
  if (status.state === "error") showStatus(status.message ?? "Error", true);
  else if (status.state === "idle" || status.state === "recording") showStatus("");
}
listen<{ state: string; message?: string }>("bolo://state", (e) => applyActivity(e.payload));


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
type Budget = { month: string; limit_cents: number; used_micros: number };
function displayBudget(b: Budget, updateInput = false) {
  const used = b.used_micros / 1_000_000, limit = b.limit_cents / 100;
  $("#budget-summary").textContent = `${b.month} · Estimated $${used.toFixed(4)} of $${limit.toFixed(2)} · ${limit === 0 ? "Cloud requests paused" : used >= limit ? "Limit reached" : `$${Math.max(0, limit-used).toFixed(2)} remaining`}`;
  $<HTMLProgressElement>("#budget-progress").value = limit > 0 ? Math.min(100, used / limit * 100) : 100;
  if (updateInput) $<HTMLInputElement>("#budget-limit").value = limit.toFixed(2);
}
async function refreshBudget(updateInput = false) {
  try { displayBudget(await invoke<Budget>("get_budget"), updateInput); }
  catch (e) { $("#budget-summary").textContent = String(e); }
}
$("#budget-save").addEventListener("click", async () => {
  const input = $<HTMLInputElement>("#budget-limit"), button = $<HTMLButtonElement>("#budget-save");
  if (!input.value.trim() || !input.reportValidity() || !Number.isFinite(input.valueAsNumber)) return;
  button.disabled = true;
  try {
    const b = await invoke<Budget>("set_budget", { limitCents: Math.round(input.valueAsNumber * 100) });
    displayBudget(b, true); $("#budget-feedback").textContent = "Limit saved. Recorded usage has not been reset.";
  } catch(e) { $("#budget-feedback").textContent = String(e); }
  finally { button.disabled = false; }
});
window.addEventListener("focus", () => { void refreshBudget(); });

// ---------- boot ----------
(async () => {
  settings = await invoke<Settings>("get_settings");
  os = await invoke<string>("platform");
  await refreshBudget(true);
  const initialStatus = await invoke<{ state: string; message?: string }>("get_status");
  applyActivity(initialStatus);

  for (const select of document.querySelectorAll<HTMLSelectElement>('.style-grid select')) {
    for (const [value,label] of [["formal","Formal — Hello, team."],["casual","Casual — Hello, team"],["lowercase","Lowercase — hello, team"],["literal","Literal — preserve wording"]]) select.add(new Option(label,value));
  }
  await loadMicrophones();
  $("#context-row").hidden = os !== "windows";
  bindFields();
  applyVisibility();
  if (!settings.openai_key_saved && !String(settings.openai_api_key ?? "").trim()) {
    $<HTMLDetailsElement>("#connection-details").open = true;
  }
  renderShortcut();
  if(settings.check_updates) void checkForUpdates();
  await renderModels();
  const models = await invoke<Model[]>("list_models");
  if (!models.some((m) => m.downloaded) && !String(settings.openai_api_key).trim()) {
    showStatus("Download a model or configure a speech engine to start");
  }
})();
