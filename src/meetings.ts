import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
type Turn = {id:string;source:string;start_ms:number;end_ms:number;text:string;complete:boolean;speaker:string|null};
type Meeting = {id:number;title:string;status:string;error:string;elapsed_ms:number;estimated_micros:number;turns:Turn[]};
const el = <T extends HTMLElement>(id:string) => document.getElementById(id) as T;
let current:Meeting|null=null;
let selected:Meeting|null=null;
let busy=false;
const time=(ms:number)=>`${Math.floor(ms/60000)}:${String(Math.floor(ms/1000)%60).padStart(2,"0")}`;
function download(doc:Meeting) {
  const text=[doc.title || "Meeting",new Date(doc.id).toLocaleString(),doc.error,
    ...[...doc.turns].sort((a,b)=>a.start_ms-b.start_ms).map(t=>`[${time(t.start_ms)}] ${t.speaker ?? (t.source==="microphone"?"Microphone":"Computer audio")}${t.complete?"":" (unfinished)"}: ${t.text}`)].filter(Boolean).join("\n\n");
  const url=URL.createObjectURL(new Blob([text],{type:"text/plain;charset=utf-8"}));
  const a=document.createElement("a");a.href=url;a.download=`Bolo-meeting-${doc.id}.txt`;a.click();setTimeout(()=>URL.revokeObjectURL(url),1000);
}
function render(doc:Meeting|null) {
  const list=el("meeting-transcript");
  const nearBottom=list.scrollHeight-list.scrollTop-list.clientHeight<60;
  el("meeting-heading").textContent=doc?.title || "Live transcript";
  el("meeting-meta").textContent=doc?`${doc.status} · ${time(doc.elapsed_ms)} · estimated $${(doc.estimated_micros/1e6).toFixed(4)}`:"Start a meeting to see captions here.";
  el("meeting-error").textContent=doc?.error || "";
  el<HTMLButtonElement>("meeting-export").disabled=!doc?.turns.length;
  el<HTMLButtonElement>("meeting-files").disabled=!doc;
  const turns=[...(doc?.turns??[])].sort((a,b)=>a.start_ms-b.start_ms);
  const ids=new Set(turns.map(t=>t.id));
  for(const node of Array.from(list.children)) if(!ids.has((node as HTMLElement).dataset.id!))node.remove();
  for(const [index,turn] of turns.entries()) {
    let node=Array.from(list.children).find(n=>(n as HTMLElement).dataset.id===turn.id) as HTMLElement|undefined;
    if(!node){node=document.createElement("li");node.dataset.id=turn.id;node.append(document.createElement("small"),document.createElement("p"));}
    node.classList.toggle("partial",!turn.complete);
    node.querySelector("small")!.textContent=`${time(turn.start_ms)} · ${turn.speaker ? turn.speaker+" (estimated)" : (turn.source==="microphone"?"Microphone":"Computer audio")}${turn.complete?"":" · Live draft"}`;
    const p=node.querySelector("p")!;if(p.textContent!==turn.text)p.textContent=turn.text;
    if(list.children[index]!==node)list.insertBefore(node,list.children[index]??null);
  }
  if(nearBottom)list.scrollTop=list.scrollHeight;
}
function controls() {
  el<HTMLButtonElement>("meeting-start").disabled=busy||!!current;
  el<HTMLButtonElement>("meeting-stop").disabled=!current||current.status==="stopping"||current.status==="labelling";
  el<HTMLButtonElement>("meeting-quick-stop").disabled=el<HTMLButtonElement>("meeting-stop").disabled;
  el<HTMLSelectElement>("meeting-output").disabled=!!current;
  el("meeting-running").hidden=!current;
  el("meeting-running-text").textContent=current?`Meeting ${current.status} · ${time(current.elapsed_ms)} · microphone + computer audio`:"";
}
export async function refreshMeetings() {
  try {
    current=await invoke<Meeting|null>("meeting_current");controls();
    if(current){selected=current;render(current);}
    const docs=await invoke<Meeting[]>("meeting_list"); const list=el("meeting-list");list.replaceChildren();
    for(const doc of docs){
      const li=document.createElement("li");const open=document.createElement("button");open.className="btn";
      open.textContent=`${doc.title||"Meeting"} · ${new Date(doc.id).toLocaleString()} · ${doc.status}`;
      open.onclick=async()=>{try{selected=await invoke<Meeting>("meeting_get",{id:doc.id});render(selected);}catch(e){selected=doc;render(doc);el("meeting-error").textContent=String(e);}};
      const remove=document.createElement("button");remove.className="btn link";remove.textContent="Delete";
      remove.onclick=async()=>{
        if(!window.confirm("Delete this meeting's transcript and both saved recordings?"))return;
        try{await invoke("meeting_delete",{id:doc.id});if(selected?.id===doc.id){selected=null;render(null);}await refreshMeetings();}
        catch(e){el("meeting-error").textContent=String(e);}
      };li.append(open,remove);list.append(li);
    }
  }catch(e){el("meeting-error").textContent=String(e);}
}
async function devices(){
  try{const names=await invoke<string[]>("meeting_devices");const select=el<HTMLSelectElement>("meeting-output");const old=select.value;select.replaceChildren(new Option(navigator.platform.includes("Mac")?"All computer audio":"System default speakers / headphones",""));for(const name of names)select.add(new Option(name,name));select.value=names.includes(old)?old:"";}
  catch(e){el("meeting-error").textContent=String(e);el<HTMLButtonElement>("meeting-start").disabled=true;}
}
el("meeting-refresh-devices").onclick=()=>void devices();
el("meeting-start").onclick=async()=>{
  busy=true;controls();el("meeting-error").textContent="";
  try{await invoke("meeting_start",{title:el<HTMLInputElement>("meeting-title").value,output:el<HTMLSelectElement>("meeting-output").value});await refreshMeetings();el("meeting-heading").scrollIntoView({block:"start"});}
  catch(e){el("meeting-error").textContent=String(e);}finally{busy=false;controls();}
};
async function stop(){try{await invoke("meeting_stop");}catch(e){el("meeting-error").textContent=String(e);}}
el("meeting-stop").onclick=()=>void stop();el("meeting-quick-stop").onclick=()=>void stop();
el("meeting-export").onclick=()=>{if(selected)download(selected);};
listen<Meeting>("bolo://meeting",e=>{current=e.payload;controls();if(!selected||selected.id===current.id||current.status==="connecting"){selected=current;render(current);}});
listen("bolo://meetings-changed",()=>{current=null;controls();void refreshMeetings();});
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