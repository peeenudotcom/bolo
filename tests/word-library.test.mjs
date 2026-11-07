import test from "node:test";
import assert from "node:assert/strict";
import { parseLibrary, libraryFromFields, mergeLibrary } from "../src/word-library.ts";
const empty = { dictionary: "", corrections: "", snippets: "" };
test("export only includes vocabulary fields, never credentials", () => {
 const result = libraryFromFields({...empty, dictionary:"Saurav", openai_api_key:"not-a-real-key"});
 assert.equal(JSON.stringify(result).includes("not-a-real-key"),false);
 assert.deepEqual(Object.keys(result).sort(),["corrections","dictionary","format","snippets","version"]);
});
test("multiline snippets round-trip without dropping their final line break", () => {
 const fields={...empty,snippets:"signature => Regards,\\nTeam\\n"};
 const library=libraryFromFields(fields);
 assert.equal(library.snippets[0].replacement,"Regards,\nTeam\n");
 assert.deepEqual(mergeLibrary(empty,parseLibrary(JSON.parse(JSON.stringify(library)))).fields,fields);
});
test("imports keep existing choices and report conflicting duplicate triggers", () => {
 const fields={dictionary:"Saurav",corrections:"bolo app => Bolo",snippets:"my email => first@example.com"};
 const library=parseLibrary({format:"bolo-words",version:1,dictionary:[" saurav ","New name"],
 corrections:[{trigger:"BOLO APP",replacement:"Bolo"}],snippets:[
 {trigger:"MY EMAIL",replacement:"different@example.com"},{trigger:"address",replacement:"Office"},
 {trigger:"ADDRESS",replacement:"Other office"}]});
 const result=mergeLibrary(fields,library);
 assert.equal(result.added,2); assert.equal(result.skipped,4);
 assert.deepEqual(result.conflicts,["MY EMAIL","ADDRESS"]);
 assert.equal(result.fields.snippets,"my email => first@example.com\naddress => Office");
});
test("invalid imports are rejected instead of silently erasing words", () => {
 const base={format:"bolo-words",version:1,dictionary:[],corrections:[],snippets:[]};
 for(const invalid of [null,{}, {...base,version:2},{...base,dictionary:"bad"},
 {...base,snippets:[{trigger:"email",replacement:""}]},
 {...base,snippets:[{trigger:"bad => trigger",replacement:"text"}]},
 {...base,corrections:[{trigger:"word",replacement:"line\nbreak"}]},
 {...base,dictionary:Array(1001).fill("word")}])
 assert.throws(()=>parseLibrary(invalid));
 assert.throws(()=>libraryFromFields({...empty,snippets:"unfinished entry"}));
});
test("unknown imported fields cannot overwrite settings", () => {
 const value=parseLibrary({format:"bolo-words",version:1,dictionary:["hello"],corrections:[],snippets:[],engine:"bad",openai_api_key:"bad"});
 assert.deepEqual(mergeLibrary(empty,value).fields,{...empty,dictionary:"hello"});
});
