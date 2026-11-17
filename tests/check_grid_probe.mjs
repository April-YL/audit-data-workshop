import assert from 'node:assert/strict';
import {test} from 'node:test';
import {createRequire} from 'node:module';
import {readFile,mkdir,writeFile} from 'node:fs/promises';
const require=createRequire(new URL('../code/desktop/package.json',import.meta.url));
const ts=require('typescript');
const output=new URL('../outputs/step1-foundation/component-probe/',import.meta.url);
await mkdir(output,{recursive:true});
const source=await readFile(new URL('../code/desktop/src/grid_probe_data.ts',import.meta.url),'utf8');
await writeFile(new URL('grid_probe_data.mjs',output),ts.transpileModule(source,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ESNext}}).outputText);
const {probePage,probeDatasource,PROBE_FIELDS}=await import(new URL('grid_probe_data.mjs',output));
test('all blocks cover 2401 stable unique rows and preserve leading zeros/null/empty',()=>{
 const rows=[];for(let start=0;start<2401;start+=200)rows.push(...probePage(start,start+200));
 assert.equal(rows.length,2401);assert.equal(new Set(rows.map(r=>r.id)).size,2401);assert.equal(PROBE_FIELDS.length,24);
 assert.equal(rows[0]['科目编码'],'00000');assert.equal(rows[0]['字段4'],null);assert.equal(rows[19]['字段4'],'');assert.equal(probePage(2400,2600).length,1);
 assert.deepEqual(probePage(0,200),rows.slice(0,200));assert.throws(()=>probePage(-1,2));assert.throws(()=>probePage(0,501));
});
test('destroyed generation cannot deliver late records; live generation reports full row count',async()=>{
 let delivered=0;const old=probeDatasource();old.getRows({startRow:0,endRow:200,successCallback(){delivered++;},failCallback(){assert.fail();}});old.destroy();await Promise.resolve();assert.equal(delivered,0);
 const live=probeDatasource();await new Promise(resolve=>live.getRows({startRow:2400,endRow:2600,successCallback(rows,total){assert.equal(rows.length,1);assert.equal(total,2401);resolve();},failCallback(){assert.fail();}}));
});
