export type Change={kind:'same'|'removed'|'added';text:string};
// Bound quadratic work on large dictations; never approximate or lose their text.
export function textChanges(before:string,after:string):Change[]{
 if(before===after)return [{kind:'same',text:before}];
 const a=before.match(/\s+|[^\s]+/gu)??[],b=after.match(/\s+|[^\s]+/gu)??[];
 if(a.length*b.length>160000)return [{kind:'removed',text:before},{kind:'added',text:after}];
 const rows=Array.from({length:a.length+1},()=>new Uint16Array(b.length+1));
 for(let i=a.length-1;i>=0;i--)for(let j=b.length-1;j>=0;j--)rows[i][j]=a[i]===b[j]?1+rows[i+1][j+1]:Math.max(rows[i+1][j],rows[i][j+1]);
 const result:Change[]=[];
 const add=(kind:Change['kind'],text:string)=>{const last=result[result.length-1];if(last?.kind===kind)last.text+=text;else result.push({kind,text});};
 let i=0,j=0;while(i<a.length||j<b.length){
  if(i<a.length&&j<b.length&&a[i]===b[j]){add('same',a[i++]);j++;}
  else if(i<a.length&&(j===b.length||rows[i+1][j]>=rows[i][j+1]))add('removed',a[i++]);
  else add('added',b[j++]);
 }return result;
}
