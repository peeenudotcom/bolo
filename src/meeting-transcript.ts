export type Turn = {id:string;source:string;start_ms:number;end_ms:number;text:string;complete:boolean;speaker:string|null};
export type Meeting = {id:number;title:string;status:string;error:string;elapsed_ms:number;estimated_micros:number;turns:Turn[];speaker_labels?:Record<string,string>;microphone_only?:boolean;recovered_from?:number|null};
export const time = (ms:number) => `${Math.floor(ms/60000)}:${String(Math.floor(ms/1000)%60).padStart(2,"0")}`;
export const speaker = (turn:Turn,doc?:Meeting):string => doc?.speaker_labels?.[`turn:${turn.id}`] ?? doc?.speaker_labels?.[turn.speaker?`speaker:${turn.speaker}`:`source:${turn.source}`] ?? (turn.speaker ? `${turn.speaker} (estimated)` : turn.source === "microphone" ? "Microphone" : "Computer audio");
export function matchingTurns(turns:Turn[], query:string,doc?:Meeting):Turn[] {
  const needle=query.trim().normalize("NFC").toLocaleLowerCase();
  return [...turns].sort((a,b)=>a.start_ms-b.start_ms).filter(t=>!needle || `${speaker(t,doc)} ${t.text}`.normalize("NFC").toLocaleLowerCase().includes(needle));
}
function timestamp(ms:number):string {
  const value=Math.max(0,Math.floor(ms));
  return `${String(Math.floor(value/3600000)).padStart(2,"0")}:${String(Math.floor(value/60000)%60).padStart(2,"0")}:${String(Math.floor(value/1000)%60).padStart(2,"0")},${String(value%1000).padStart(3,"0")}`;
}
export function exportTranscript(doc:Meeting, format:"txt"|"srt"):string {
  const turns=matchingTurns(doc.turns,"").filter(t=>t.text.trim());
  if(format==="srt") return turns.filter(t=>t.complete).map((t,index)=> {
    // Caption times are estimates. Keep a valid cue even if its final event had no duration.
    const start=Math.max(0,t.start_ms); const end=Math.max(start+1,t.end_ms);
    // SRT interprets markup and blank lines; preserve words without creating unintended cues.
    const text=t.text.replace(/&/g,"&amp;").replace(/</g,"&lt;").replace(/>/g,"&gt;").replace(/\s*\r?\n\s*/g," ").trim();
    return `${index+1}\n${timestamp(start)} --> ${timestamp(end)}\n${speaker(t,doc).replace(/&/g,"&amp;").replace(/</g,"&lt;").replace(/>/g,"&gt;")}: ${text}\n`;
  }).join("\n");
  return [doc.title || "Meeting",new Date(doc.id).toLocaleString(),doc.error,
    ...turns.map(t=>`[${time(t.start_ms)}] ${speaker(t,doc)}${t.complete?"":" (unfinished)"}: ${t.text}`)].filter(Boolean).join("\n\n");
}
