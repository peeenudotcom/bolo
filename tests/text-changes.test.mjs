import test from 'node:test';
import assert from 'node:assert/strict';
import {textChanges} from '../src/text-changes.ts';
for(const [before,after] of [['Kal milde haan','Kal milte hain.'],['Hello\n\nteam','Hello team.'],['ਕੱਲ੍ਹ ਮਿਲਦੇ ਹਾਂ','Kal milde haan.'],['','Hi'],['Hi',''],['a '.repeat(500),'b '.repeat(500)]]) {
 test(`changes reconstruct both versions (${before.length}/${after.length})`,()=>{
  const diff=textChanges(before,after);
  assert.equal(diff.filter(p=>p.kind!=='added').map(p=>p.text).join(''),before);
  assert.equal(diff.filter(p=>p.kind!=='removed').map(p=>p.text).join(''),after);
 });
}
