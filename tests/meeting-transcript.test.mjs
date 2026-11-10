import test from 'node:test';
import assert from 'node:assert/strict';
import {matchingTurns, exportTranscript} from '../src/meeting-transcript.ts';
const turn=(values={})=>({id:'a',source:'computer',start_ms:1000,end_ms:2500,text:'Kal milde haan.',complete:true,speaker:null,...values});
const doc=(turns)=>({id:1,title:'Office',status:'saved',error:'',elapsed_ms:3000,estimated_micros:0,turns});
test('search preserves mixed language, sorts without changing saved turns',()=>{
 const turns=[turn({id:'b',start_ms:2000,text:'Please SEND report'}),turn({text:'ਕੱਲ੍ਹ ਮਿਲਦੇ ਹਾਂ'})];
 assert.equal(matchingTurns(turns,' send ')[0].id,'b');
 assert.equal(matchingTurns(turns,'ਕੱਲ੍ਹ')[0].id,'a');
 assert.deepEqual(matchingTurns(turns,'').map(t=>t.id),['a','b']);
 assert.equal(turns[0].id,'b');
 assert.equal(matchingTurns(turns,'missing').length,0);
});
test('text export retains unfinished captions and errors with honest speaker labels',()=>{
 const meeting=doc([turn({speaker:'Speaker 1',complete:false})]); meeting.error='Connection interrupted';
 const text=exportTranscript(meeting,'txt');
 assert.match(text,/Connection interrupted/);
 assert.match(text,/Speaker 1 \(estimated\) \(unfinished\): Kal milde haan\./);
});
test('subtitle export excludes drafts, preserves languages and valid cue times',()=>{
 const meeting=doc([turn({complete:false,text:'draft'}),turn({id:'b',start_ms:3600123,end_ms:3600123,text:'ਕੱਲ੍ਹ ਮਿਲਦੇ ਹਾਂ\n\nreport <draft> & notes'})]);
 assert.equal(exportTranscript(meeting,'srt'),'1\n01:00:00,123 --> 01:00:00,124\nComputer audio: ਕੱਲ੍ਹ ਮਿਲਦੇ ਹਾਂ report &lt;draft&gt; &amp; notes\n');
});
test('empty and whitespace-only transcripts export no subtitle cues',()=>{
 assert.equal(exportTranscript(doc([]),'srt'),'');
 assert.equal(exportTranscript(doc([turn({text:'  '})]),'srt'),'');
});
test('user speaker corrections override estimates in search and exports',()=>{
 const meeting=doc([turn({speaker:'Speaker 1'})]);
 meeting.speaker_labels={'speaker:Speaker 1':'Priya'};
 assert.equal(matchingTurns(meeting.turns,'Priya',meeting).length,1);
 assert.match(exportTranscript(meeting,'txt'),/Priya: Kal milde/);
 meeting.speaker_labels['turn:a']='Alex <test>';
 assert.match(exportTranscript(meeting,'srt'),/Alex &lt;test&gt;:/);
 assert.equal(meeting.turns[0].speaker,'Speaker 1');
});
