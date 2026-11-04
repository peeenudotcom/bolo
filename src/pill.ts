import { listen } from "@tauri-apps/api/event";

type Status = { state: "idle" | "recording" | "processing" | "error"; message?: string | null };

const pill = document.getElementById("pill")!;
const msg = document.getElementById("msg")!;
const bars = Array.from(pill.querySelectorAll<HTMLSpanElement>(".bars span"));
// Middle bars move most, like a voice waveform.
const weights = [0.45, 0.7, 0.9, 1, 0.9, 0.7, 0.45];
let errorTimer: number | undefined;

function setState(s: Status) {
  window.clearTimeout(errorTimer);
  pill.className = `pill ${s.state}`;
  if (s.state === "recording" && s.message?.includes("hands-free")) pill.classList.add("hands-free");
  if (s.state === "recording" && s.message?.includes("command")) pill.classList.add("command");
  if (s.state === "error") {
    msg.textContent = s.message ?? "Something went wrong";
    msg.title = msg.textContent;
    errorTimer = window.setTimeout(() => setState({ state: "idle" }), 4000);
  }
  if (s.state !== "recording") bars.forEach((b) => (b.style.height = "4px"));
}

listen<Status>("bolo://state", (e) => setState(e.payload));

listen<number>("bolo://level", (e) => {
  if (!pill.classList.contains("recording")) return;
  // Speech RMS is usually 0.01–0.2; map it onto a perceptual scale.
  const level = Math.min(1, Math.sqrt(e.payload) * 3.2);
  bars.forEach((b, i) => {
    const jitter = 0.75 + Math.random() * 0.5;
    b.style.height = `${4 + level * 16 * weights[i] * jitter}px`;
  });
});
