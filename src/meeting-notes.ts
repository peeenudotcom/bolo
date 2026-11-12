import {invoke} from '@tauri-apps/api/core';
import {time,type Meeting} from './meeting-transcript';
type Citation={turn:number;quote:string};
type Note={created_ms:number;question:string;items:{kind:string;text:string;citations:Citation[]}[]};
const el=<T extends HTMLElement>(id:string)=>document.getElementById(id) as T;
let selected:Meeting|null=null;
let notes:Note[]=[];
let loadedId:number|null=null;
let epoch=0;
const pending=new Set<number>();
let active=false;
let navigate:(id:string)=>void=()=>{};
function controls(){
 const disabled=!selected||active||pending.has(selected.id)||!selected.turns.some(t=>t.complete&&t.text.trim());
 el<HTMLButtonElement>('meeting-notes-generate').disabled=disabled;
 el<HTMLButtonElement>('meeting-notes-ask').disabled=disabled||!el<HTMLInputElement>('meeting-notes-question').value.trim();
 el<HTMLButtonElement>('meeting-notes-export').disabled=!notes.length;
 el<HTMLButtonElement>('meeting-notes-copy').disabled=!notes.length;
 el('meeting-notes-status').textContent=selected&&pending.has(selected.id)?'Generating from saved captions…':active?'Stop this meeting to generate notes.':'';
}
function render(){
 const list=el('meeting-notes-list');list.replaceChildren();
 for(const note of [...notes].reverse()){
  const article=document.createElement('article');const heading=document.createElement('h3');heading.textContent=note.question||'Meeting recap';article.append(heading);
  const date=document.createElement('small');date.textContent=new Date(note.created_ms).toLocaleString();article.append(date);
  if(!note.items.length){const p=document.createElement('p');p.textContent='No supported answer or notes were found in the final captions.';article.append(p);}
  for(const item of note.items){
   const p=document.createElement('p');p.textContent=`${item.kind}: ${item.text}`;article.append(p);
   for(const citation of item.citations){
    const turn=selected?.turns[citation.turn];const button=document.createElement('button');button.className='btn note-source';button.textContent=`${turn?time(turn.start_ms):'Source'} — “${citation.quote}”`;
    button.disabled=!turn;button.onclick=()=>{if(turn)navigate(turn.id);};article.append(button);
   }
  }list.append(article);
 }
 controls();
}
export function showMeetingNotes(doc:Meeting|null,isActive:boolean,onNavigate:(id:string)=>void){
 selected=doc;active=isActive;navigate=onNavigate;controls();
 if(loadedId===(doc?.id??null))return;
 loadedId=doc?.id??null;notes=[];render();el('meeting-notes-error').textContent='';
 const version=++epoch;if(!doc)return;
 invoke<Note[]>('meeting_notes_get',{id:doc.id}).then(result=>{if(epoch===version){notes=result;render();}}).catch(e=>{if(epoch===version)el('meeting-notes-error').textContent=String(e);});
}
async function generate(question:string){
 const doc=selected;if(!doc||active||pending.has(doc.id))return;
 pending.add(doc.id);++epoch;controls();el('meeting-notes-error').textContent='';
 try{const result=await invoke<Note[]>('meeting_notes_generate',{id:doc.id,question});if(selected?.id===doc.id){notes=result;render();}}
 catch(e){if(selected?.id===doc.id)el('meeting-notes-error').textContent=String(e);}
 finally{pending.delete(doc.id);controls();}
}
el('meeting-notes-generate').onclick=()=>void generate('');
el('meeting-notes-ask').onclick=()=>void generate(el<HTMLInputElement>('meeting-notes-question').value.trim());
el('meeting-notes-question').oninput=controls;
// The latest result, grouped and bulleted so it pastes cleanly into WhatsApp,
// email or Slack. Source quotes are left out of the shared copy.
export function shareText(title:string,date:Date,note:Note):string{
 const groups=new Map<string,string[]>();
 for(const item of note.items){const list=groups.get(item.kind)??[];list.push(item.text);groups.set(item.kind,list);}
 const lines=[`*${title||'Meeting'}* (${date.toLocaleDateString()})`];
 if(note.question)lines.push(`Q: ${note.question}`);
 for(const [kind,items] of groups){lines.push('',`*${kind}*`,...items.map(t=>`• ${t}`));}
 if(!note.items.length)lines.push('','No supported notes were found in the final captions.');
 return lines.join('\n');
}
el('meeting-notes-copy').onclick=async()=>{
 if(!selected||!notes.length)return;
 try{await navigator.clipboard.writeText(shareText(selected.title,new Date(selected.id),notes[notes.length-1]));el('meeting-notes-status').textContent='Latest notes copied. Paste them into WhatsApp, email or Slack.';}
 catch(e){el('meeting-notes-error').textContent=String(e);}
};
el('meeting-notes-export').onclick=()=>{
 if(!selected||!notes.length)return;
 const text=[selected.title||'Meeting','AI-generated notes. Check source quotes before sharing.',...notes.map(n=>[n.question||'Meeting recap',...n.items.flatMap(i=>[`${i.kind}: ${i.text}`,...i.citations.map(c=>`  [${time(selected!.turns[c.turn]?.start_ms??0)}] ${c.quote}`)])].join('\n'))].join('\n\n');
 const url=URL.createObjectURL(new Blob([text],{type:'text/plain;charset=utf-8'}));const a=document.createElement('a');a.href=url;a.download=`Bolo-meeting-${selected.id}-notes.txt`;a.click();setTimeout(()=>URL.revokeObjectURL(url),1000);
};
