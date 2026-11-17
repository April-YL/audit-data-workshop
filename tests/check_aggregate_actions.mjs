import assert from 'node:assert/strict';
import {test} from 'node:test';
import {createRequire} from 'node:module';
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {pathToFileURL} from 'node:url';
const require=createRequire(new URL('../code/desktop/package.json',import.meta.url)),ts=require('typescript');
const out=new URL('../outputs/aggregate-action-tests/',import.meta.url);await mkdir(out,{recursive:true});
const entries=['project','preview','components/ColumnActionPanel','components/ResultSummaryPanel','components/RowOperationsPanel','components/AggregatePanel'];
for(const entry of entries){const name=entry.split('/').pop();const src=await readFile(new URL('../code/desktop/src/'+entry+(entry.startsWith('components')?'.tsx':'.ts'),import.meta.url),'utf8');let code=ts.transpileModule(src,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ESNext,jsx:ts.JsxEmit.ReactJSX}}).outputText;code=code.replace(/(['"])(react(?:\/jsx-runtime)?|@tauri-apps\/api\/core)\1/g,(_,q,id)=>JSON.stringify(pathToFileURL(require.resolve(id)).href));code=code.replace(/(['"])(?:\.\.\/|\.\/)(project|preview|ColumnActionPanel)\1/g,(_,q,id)=>JSON.stringify('./'+id+'.mjs'));await writeFile(new URL(name+'.mjs',out),code);}
const {blankProject,newNode,applyTableAction}=await import(new URL('project.mjs',out));
const {resultRequestKey}=await import(new URL('preview.mjs',out));
const {default:Summary}=await import(new URL('ResultSummaryPanel.mjs',out));
const {default:RowPanel}=await import(new URL('RowOperationsPanel.mjs',out));
const {default:Aggregate}=await import(new URL('AggregatePanel.mjs',out));
const {createElement}=require('react'),{renderToStaticMarkup}=require('react-dom/server');
function setup(){const p=blankProject(),a=newNode('source',{x:0,y:0});p.workflow.nodes=[a];return {p,a};}
test('aggregate creates a versioned side branch without rewiring detail downstream',()=>{const {p,a}=setup();const down=newNode('filter',{x:250,y:0});p.workflow.nodes.push(down);p.workflow.edges=[{id:'existing',source:a.id,target:down.id}];const r=applyTableAction(p,a.id,{kind:'aggregate',config:{group_by:['公司'],metrics:[{op:'sum',field:'金额',target:'合计'}]}},['公司','金额']);assert.equal(r.node.version,2);assert.deepEqual(r.project.workflow.edges.find(e=>e.id==='existing'),p.workflow.edges[0]);assert.equal(r.project.workflow.edges.filter(e=>e.source===a.id).length,2);assert.ok(r.node.position.y>a.position.y);});
test('aggregate editing explains null handling and branch behavior',()=>{const html=renderToStaticMarkup(createElement(Aggregate,{fields:['公司','金额'],editing:{id:'g',kind:'aggregate',version:2,label:'汇总',position:{x:0,y:0},config:{group_by:['公司'],metrics:[]}},onAction:()=>{}}));assert.match(html,/保留原明细/);assert.match(html,/全空组金额为空/);});
