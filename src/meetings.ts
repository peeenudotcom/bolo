import { showMeetingNotes } from "./meeting-notes";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { exportTranscript, matchingTurns, time, speaker, type Meeting } from "./meeting-transcript";
const el = <T extends HTMLElement>(id:string) => document.getElementById(id) as T;
let current:Meeting|null=null;
let selected:Meeting|null=null;
let busy=false;
let selectionVersion=0;
let devicesReady=false;
let recoveryQuote:{id:number;estimated_micros:number}|null=null;
let recoveryRunning:number|null=null;
function download(doc:Meeting) {
  const format=el<HTMLSelectElement>("meeting-export-format").value === "srt" ? "srt" : "txt";
  const text=exportTranscript(doc,format);
  const url=URL.createObjectURL(new Blob([text],{type:"text/plain;charset=utf-8"}));
  const a=document.createElement("a");a.href=url;a.download=`Bolo-meeting-${doc.id}.${format}`;a.click();setTimeout(()=>URL.revokeObjectURL(url),1000);
}
function render(doc:Meeting|null) {
  showMeetingNotes(doc,!!doc&&current?.id===doc.id,turnId=>{
    el<HTMLInputElement>("meeting-search").value="";render(selected);
    const node=Array.from(el("meeting-transcript").children).find(n=>(n as HTMLElement).dataset.id===turnId) as HTMLElement|undefined;
    if(node){node.tabIndex=-1;node.focus();node.scrollIntoView({block:"center"});}
  });
  el<HTMLButtonElement>("meeting-recovery-estimate").disabled=!doc||!!current||recoveryRunning!==null;
  el<HTMLButtonElement>("meeting-recovery-start").disabled=!doc||!!current||recoveryRunning!==null||recoveryQuote?.id!==doc.id;
  const labels=el<HTMLSelectElement>("meeting-speaker-target");const previous=labels.value;labels.replaceChildren();
  for(const key of new Set((doc?.turns??[]).map(t=>t.speaker?`speaker:${t.speaker}`:`source:${t.source}`))){labels.add(new Option(key.replace(/^[^:]+:/,""),key));}
  if(previous.startsWith("turn:")&&doc?.turns.some(t=>`turn:${t.id}`===previous))labels.add(new Option("Selected caption only",previous));
  if(Array.from(labels.options).some(o=>o.value===previous))labels.value=previous;
  el<HTMLButtonElement>("meeting-speaker-save").disabled=!doc||current?.id===doc.id;
  const list=el("meeting-transcript");
  const nearBottom=list.scrollHeight-list.scrollTop-list.clientHeight<60;
  el("meeting-heading").textContent=doc?.title || "Live transcript";
  el("meeting-meta").textContent=doc?`${doc.status} · ${time(doc.elapsed_ms)} · estimated $${(doc.estimated_micros/1e6).toFixed(4)}`:"Start a meeting to see captions here.";
  el("meeting-error").textContent=doc?.error || "";
  const subtitle=el<HTMLSelectElement>("meeting-export-format").value==="srt";
  el<HTMLButtonElement>("meeting-export").disabled=!doc?.turns.some(t=>t.text.trim()&&(!subtitle||t.complete));
  el<HTMLButtonElement>("meeting-files").disabled=!doc;
  el<HTMLButtonElement>("meeting-copy-text").disabled=!doc?.turns.some(t=>t.complete&&t.text.trim());
  const query=el<HTMLInputElement>("meeting-search").value;
  const turns=matchingTurns(doc?.turns??[],query,doc??undefined);
  el("meeting-search-result").textContent=query.trim()?`${turns.length} matching turns. Export includes the full transcript.`:"";
  const ids=new Set(turns.map(t=>t.id));
  // Index existing rows once: searching the list for every turn was O(n²)
  // and made hour-long meetings lag.
  const rows=new Map<string,HTMLElement>();
  for(const node of Array.from(list.children) as HTMLElement[]) {if(!ids.has(node.dataset.id!))node.remove();else rows.set(node.dataset.id!,node);}
  for(const [index,turn] of turns.entries()) {
    let node=rows.get(turn.id);
    if(!node){node=document.createElement("li");node.dataset.id=turn.id;node.append(document.createElement("small"),document.createElement("p"));const edit=document.createElement("button");edit.className="btn link";edit.textContent="Correct this speaker";edit.onclick=()=>{labels.add(new Option("Selected caption only",`turn:${turn.id}`));labels.value=`turn:${turn.id}`;el<HTMLInputElement>("meeting-speaker-name").focus();};node.append(edit);}
    node.classList.toggle("partial",!turn.complete);
    node.querySelector("small")!.textContent=`${time(turn.start_ms)} · ${speaker(turn,doc??undefined)}${turn.complete?"":" · Live draft"}`;
    const p=node.querySelector("p")!;if(p.textContent!==turn.text)p.textContent=turn.text;
    if(list.children[index]!==node)list.insertBefore(node,list.children[index]??null);
  }
  if(nearBottom&&!query.trim())list.scrollTop=list.scrollHeight;
}
function controls() {
  el<HTMLButtonElement>("meeting-start").disabled=busy||!!current||!devicesReady||recoveryRunning!==null;
  el("meeting-start").textContent=el<HTMLSelectElement>("meeting-mode").value==="long"?"Start long dictation":"Start meeting";
  el<HTMLButtonElement>("meeting-stop").disabled=!current||current.status==="stopping"||current.status==="labelling";
  el<HTMLButtonElement>("meeting-quick-stop").disabled=el<HTMLButtonElement>("meeting-stop").disabled;
  el<HTMLSelectElement>("meeting-output").disabled=!!current||el<HTMLSelectElement>("meeting-mode").value==="long";
  el<HTMLSelectElement>("meeting-mode").disabled=!!current;
  el("meeting-running").hidden=!current;
  el("meeting-running-text").textContent=current?`Meeting ${current.status} · ${time(current.elapsed_ms)} · ${current.microphone_only?"microphone only":"microphone + computer audio"}${current.elapsed_ms>=55*60*1000?` · Stops in ${Math.max(0,Math.ceil((60*60*1000-current.elapsed_ms)/60000))} min`:""}`:"";
}
export async function refreshMeetings() {
  try {
    current=await invoke<Meeting|null>("meeting_current");controls();
    if(current){selected=current;render(current);}
    else if(selected){
      const id=selected.id; const version=selectionVersion;
      try { const saved=await invoke<Meeting>("meeting_get",{id});
        if(version===selectionVersion&&selected?.id===id){selected=saved;render(saved);}
      } catch(e) {if(version===selectionVersion)el("meeting-error").textContent=String(e);}
    }
    const docs=await invoke<Meeting[]>("meeting_list"); const list=el("meeting-list");list.replaceChildren();
    for(const doc of docs){
      const li=document.createElement("li");const open=document.createElement("button");open.className="btn";
      open.textContent=`${doc.title||"Meeting"} · ${new Date(doc.id).toLocaleString()} · ${doc.status}`;
      open.onclick=async()=>{
        const version=++selectionVersion;
        try{const saved=await invoke<Meeting>("meeting_get",{id:doc.id});if(version!==selectionVersion)return;selected=saved;render(saved);}
        catch(e){if(version!==selectionVersion)return;selected=doc;render(doc);el("meeting-error").textContent=String(e);}
      };
      const remove=document.createElement("button");remove.className="btn link";remove.textContent="Delete";
      remove.onclick=async()=>{
        if(!window.confirm("Delete this meeting's transcript and both saved recordings?"))return;
        try{++selectionVersion;await invoke("meeting_delete",{id:doc.id});if(selected?.id===doc.id){selected=null;render(null);}await refreshMeetings();}
        catch(e){el("meeting-error").textContent=String(e);}
      };li.append(open,remove);list.append(li);
    }
  }catch(e){el("meeting-error").textContent=String(e);}
}
async function devices(){
  devicesReady=false;controls();
  try{const names=await invoke<string[]>("meeting_devices");const select=el<HTMLSelectElement>("meeting-output");const old=select.value;select.replaceChildren(new Option(navigator.platform.includes("Mac")?"All computer audio":"System default speakers / headphones",""));for(const name of names)select.add(new Option(name,name));select.value=names.includes(old)?old:"";devicesReady=true;controls();}
  catch(e){el("meeting-error").textContent=String(e);el<HTMLButtonElement>("meeting-start").disabled=true;}
}
el("meeting-refresh-devices").onclick=()=>void devices();
el("meeting-start").onclick=async()=>{
  busy=true;controls();el("meeting-error").textContent="";
  try{await invoke("meeting_start",{title:el<HTMLInputElement>("meeting-title").value,output:el<HTMLSelectElement>("meeting-output").value,microphoneOnly:el<HTMLSelectElement>("meeting-mode").value==="long"});await refreshMeetings();el("meeting-heading").scrollIntoView({block:"start"});}
  catch(e){el("meeting-error").textContent=String(e);}finally{busy=false;controls();}
};
async function stop(){try{await invoke("meeting_stop");}catch(e){el("meeting-error").textContent=String(e);}}
el("meeting-stop").onclick=()=>void stop();el("meeting-quick-stop").onclick=()=>void stop();
el("meeting-export").onclick=()=>{if(selected)download(selected);};
listen<Meeting>("bolo://meeting",e=>{current=e.payload;controls();if(!selected||selected.id===current.id||current.status==="connecting"){if(current.status==="connecting")++selectionVersion;selected=current;render(current);}});
listen("bolo://meetings-changed",()=>{current=null;controls();void refreshMeetings();});
el("meeting-search").oninput=()=>render(selected);
el("meeting-export-format").onchange=()=>render(selected);
void devices();void refreshMeetings();
async function speakerModels(){
  const button=el<HTMLButtonElement>("meeting-speaker-models");
  const ready=await invoke<boolean>("meeting_speaker_models");button.disabled=ready;button.textContent=ready?"Speaker labels ready":"Download speaker labels · 41 MB";
}
el("meeting-speaker-models").onclick=async()=>{
  const button=el<HTMLButtonElement>("meeting-speaker-models");button.disabled=true;button.textContent="Downloading speaker models…";
  try{await invoke("meeting_download_speaker_models");await speakerModels();}
  catch(e){el("meeting-error").textContent=String(e);button.disabled=false;button.textContent="Retry speaker model download";}
};
void speakerModels().catch(e=>el("meeting-error").textContent=String(e));

el("meeting-files").onclick=async()=>{if(selected)try{await invoke("meeting_show_files",{id:selected.id});}catch(e){el("meeting-error").textContent=String(e);}};
el("meeting-mode").onchange=controls;
el("meeting-speaker-save").onclick=async()=>{
 if(!selected)return;const id=selected.id;
 try{const doc=await invoke<Meeting>("meeting_label_speaker",{id,target:el<HTMLSelectElement>("meeting-speaker-target").value,name:el<HTMLInputElement>("meeting-speaker-name").value});if(selected?.id===id){selected=doc;render(doc);}}
 catch(e){el("meeting-error").textContent=String(e);}
};

el("meeting-recovery-estimate").onclick=async()=>{
 if(!selected)return;const id=selected.id;recoveryQuote=null;
 try{const quote=await invoke<{estimated_micros:number;remaining_seconds:number;recovered_id:number|null}>("meeting_recovery_quote",{id});if(selected?.id!==id)return;
   if(quote.remaining_seconds){recoveryQuote={id,estimated_micros:quote.estimated_micros};el("meeting-recovery-status").textContent=`Up to approximately $${(quote.estimated_micros/1e6).toFixed(4)} for ${Math.ceil(quote.remaining_seconds/60)} remaining audio minutes. Click Recover / resume to send this audio.`;}
   else el("meeting-recovery-status").textContent="Recovery is already complete. Open the recovered transcript in Saved meetings.";
 }catch(e){el("meeting-recovery-status").textContent=String(e);}render(selected);
};
el("meeting-recovery-start").onclick=async()=>{
 if(!selected||!recoveryQuote||recoveryQuote.id!==selected.id)return;const quote=recoveryQuote;recoveryQuote=null;recoveryRunning=quote.id;controls();render(selected);el<HTMLButtonElement>("meeting-recovery-cancel").disabled=false;
 try{const id=await invoke<number>("meeting_recover",{id:quote.id,maxEstimatedMicros:quote.estimated_micros});el("meeting-recovery-status").textContent="Recovery saved as a separate transcript.";selected=await invoke<Meeting>("meeting_get",{id});}
 catch(e){el("meeting-recovery-status").textContent=String(e);}
 finally{recoveryRunning=null;controls();el<HTMLButtonElement>("meeting-recovery-cancel").disabled=true;await refreshMeetings();render(selected);}
};
el("meeting-recovery-cancel").onclick=async()=>{if(recoveryRunning!==null){await invoke("meeting_recovery_cancel",{id:recoveryRunning});el("meeting-recovery-status").textContent="Stopping after the current request; completed work will be saved.";}};
listen<{id:number;remaining_seconds:number}>("bolo://recovery",e=>{if(recoveryRunning===e.payload.id)el("meeting-recovery-status").textContent=`Recovering audio… ${Math.ceil(e.payload.remaining_seconds/60)} audio minutes remaining.`;});

// ---------- "meeting detected" reminder ----------
listen<{app:string}>("bolo://meeting-detected",e=>{
  if(current)return;
  el("meeting-detected-text").textContent=`${e.payload.app} is using your microphone. Record and transcribe this meeting?`;
  el("meeting-detected").hidden=false;
});
el("meeting-detected-dismiss").onclick=()=>{el("meeting-detected").hidden=true;};
el("meeting-detected-start").onclick=()=>{
  el("meeting-detected").hidden=true;
  document.querySelector<HTMLButtonElement>('[data-tab="meetings"]')?.click();
  el<HTMLSelectElement>("meeting-mode").value="meeting";controls();
  if(!el<HTMLButtonElement>("meeting-start").disabled)el("meeting-start").click();
};

// ---------- search all saved meetings (local, free) ----------
type Hit={id:number;title:string;turn_id:string;start_ms:number;speaker:string;text:string};
let searchTimer:number|undefined;let searchVersion=0;
function focusTurn(turnId:string){
  el<HTMLInputElement>("meeting-search").value="";render(selected);
  const node=el("meeting-transcript").querySelector<HTMLElement>(`li[data-id="${CSS.escape(turnId)}"]`);
  if(node){node.tabIndex=-1;node.focus();node.scrollIntoView({block:"center"});}
}
el("meeting-search-all").oninput=()=>{
  window.clearTimeout(searchTimer);
  searchTimer=window.setTimeout(async()=>{
    const query=el<HTMLInputElement>("meeting-search-all").value;const version=++searchVersion;
    const results=el("meeting-search-all-results");const status=el("meeting-search-all-status");
    if(query.trim().length<2){results.replaceChildren();status.textContent="";return;}
    try{
      const hits=await invoke<Hit[]>("meeting_search_all",{query});if(version!==searchVersion)return;
      status.textContent=hits.length?`${hits.length}${hits.length>=100?"+":""} matching captions`:"No saved meeting mentions that.";
      results.replaceChildren(...hits.map(hit=>{
        const li=document.createElement("li");const open=document.createElement("button");open.className="btn search-hit";
        const meta=document.createElement("small");meta.textContent=`${hit.title||"Meeting"} · ${new Date(hit.id).toLocaleDateString()} · ${time(hit.start_ms)} · ${hit.speaker}`;
        const text=document.createElement("span");text.textContent=hit.text;
        open.append(meta,text);
        open.onclick=async()=>{
          const v=++selectionVersion;
          try{const saved=await invoke<Meeting>("meeting_get",{id:hit.id});if(v!==selectionVersion)return;selected=saved;render(saved);focusTurn(hit.turn_id);}
          catch(e){el("meeting-error").textContent=String(e);}
        };
        li.append(open);return li;
      }));
    }catch(e){status.textContent=String(e);}
  },250);
};

el("meeting-copy-text").onclick=async()=>{if(selected)try{await navigator.clipboard.writeText(matchingTurns(selected.turns,"").filter(t=>t.complete).map(t=>t.text).join("\n\n"));el("meeting-meta").textContent="Final text copied. Live drafts are excluded.";}catch(e){el("meeting-error").textContent=String(e);}};
