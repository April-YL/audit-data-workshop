import assert from 'node:assert/strict';
import {test} from 'node:test';
import {createRequire} from 'node:module';
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {pathToFileURL} from 'node:url';
const require=createRequire(new URL('../code/desktop/package.json',import.meta.url)),ts=require('typescript');
const out=new URL('../outputs/formula-column-tests/',import.meta.url);await mkdir(out,{recursive:true});
const entries=['project','preview','components/ColumnActionPanel','components/ResultSummaryPanel','components/RowOperationsPanel','components/AggregatePanel'];
for(const entry of entries){const name=entry.split('/').pop();const src=await readFile(new URL('../code/desktop/src/'+entry+(entry.startsWith('components')?'.tsx':'.ts'),import.meta.url),'utf8');let code=ts.transpileModule(src,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ESNext,jsx:ts.JsxEmit.ReactJSX}}).outputText;code=code.replace(/(['"])(react(?:\/jsx-runtime)?|@tauri-apps\/api\/core)\1/g,(_,q,id)=>JSON.stringify(pathToFileURL(require.resolve(id)).href));code=code.replace(/(['"])(?:\.\.\/|\.\/)(project|preview|ColumnActionPanel)\1/g,(_,q,id)=>JSON.stringify('./'+id+'.mjs'));await writeFile(new URL(name+'.mjs',out),code);}
const {blankProject,newNode,applyTableAction,toolCatalog}=await import(new URL('project.mjs',out));
const {resultRequestKey}=await import(new URL('preview.mjs',out));
const {default:Summary}=await import(new URL('ResultSummaryPanel.mjs',out));
const {default:RowPanel}=await import(new URL('RowOperationsPanel.mjs',out));
const {default:Aggregate}=await import(new URL('AggregatePanel.mjs',out));
const {createElement}=require('react'),{renderToStaticMarkup}=require('react-dom/server');
function setup(){const p=blankProject(),a=newNode('source',{x:0,y:0});p.workflow.nodes=[a];return {p,a};}
test('derive and placement create one immutable transaction with ordered map',()=>{const {p,a}=setup();const before=JSON.stringify(p);const r=applyTableAction(p,a.id,{kind:'derive',expression:'[金额]+1',target:'计算结果',placement:'before',anchor:'公司'},['公司','金额']);assert.equal(JSON.stringify(p),before);assert.equal(r.project.workflow.nodes.length,3);assert.equal(r.node.kind,'map');assert.deepEqual(r.node.config.mappings.map(m=>m.source),['计算结果','公司','金额','计算结果__error']);assert.equal(r.project.workflow.nodes.find(n=>n.kind==='derive').config.expression,'[金额]+1');assert.equal(r.project.workflow.edges.length,2);});
test('move rejects lost fields and formula targets reject collisions',()=>{const {p,a}=setup();assert.throws(()=>applyTableAction(p,a.id,{kind:'reorder',fields:['公司']},['公司','金额']));assert.throws(()=>applyTableAction(p,a.id,{kind:'derive',expression:'1',target:'金额',placement:'first'},['公司','金额']));const r=applyTableAction(p,a.id,{kind:'reorder',fields:['金额','公司']},['公司','金额']);assert.deepEqual(r.node.config.mappings.map(m=>m.target),['金额','公司']);});

test('new steps offer general formula and no standalone subtraction',()=>{
    assert.equal(toolCatalog.find(t=>t.kind==='derive')?.label,'公式计算');
    assert.ok(!toolCatalog.some(t=>t.kind==='calculate'));
});