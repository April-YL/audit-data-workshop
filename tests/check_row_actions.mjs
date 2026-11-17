import assert from 'node:assert/strict';
import {test} from 'node:test';
import {createRequire} from 'node:module';
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {pathToFileURL} from 'node:url';
const require=createRequire(new URL('../code/desktop/package.json',import.meta.url)),ts=require('typescript');
const out=new URL('../outputs/row-action-tests/',import.meta.url);await mkdir(out,{recursive:true});
const entries=['project','preview','components/ColumnActionPanel','components/ResultSummaryPanel','components/RowOperationsPanel','components/AggregatePanel'];
for(const entry of entries){const name=entry.split('/').pop();const src=await readFile(new URL('../code/desktop/src/'+entry+(entry.startsWith('components')?'.tsx':'.ts'),import.meta.url),'utf8');let code=ts.transpileModule(src,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ESNext,jsx:ts.JsxEmit.ReactJSX}}).outputText;code=code.replace(/(['"])(react(?:\/jsx-runtime)?|@tauri-apps\/api\/core)\1/g,(_,q,id)=>JSON.stringify(pathToFileURL(require.resolve(id)).href));code=code.replace(/(['"])(?:\.\.\/|\.\/)(project|preview|ColumnActionPanel)\1/g,(_,q,id)=>JSON.stringify('./'+id+'.mjs'));await writeFile(new URL(name+'.mjs',out),code);}
const {blankProject,newNode,applyTableAction}=await import(new URL('project.mjs',out));
const {resultRequestKey}=await import(new URL('preview.mjs',out));
const {default:Summary}=await import(new URL('ResultSummaryPanel.mjs',out));
const {default:RowPanel}=await import(new URL('RowOperationsPanel.mjs',out));
const {default:Aggregate}=await import(new URL('AggregatePanel.mjs',out));
const {createElement}=require('react'),{renderToStaticMarkup}=require('react-dom/server');
function setup(){const p=blankProject(),a=newNode('source',{x:0,y:0});p.workflow.nodes=[a];return {p,a};}
test('row operations record explicit typed configs and preserve single input',()=>{for(const [kind,config] of [['sort',{order:[{field:'金额',direction:'desc'}]}],['duplicates',{keys:['公司'],target:'重复',count_target:'条数',rank_target:'序号'}],['deduplicate',{keys:['公司'],keep:'last'}],['multi_filter',{match:'all',conditions:[{field:'金额',operator:'numeric_gt',value:'0'}]}]]){const {p,a}=setup();const r=applyTableAction(p,a.id,{kind,config},['公司','金额']);assert.deepEqual(r.node.config,config);assert.equal(r.node.kind,kind==='multi_filter'?'filter':kind);assert.equal(r.project.workflow.edges.filter(e=>e.target===r.node.id).length,1);}});
test('row panel editing labels make removal destination explicit',()=>{const html=renderToStaticMarkup(createElement(RowPanel,{fields:['公司','金额'],editing:{id:'d',kind:'deduplicate',version:1,label:'去重',position:{x:0,y:0},config:{keys:['公司'],keep:'first'}},onAction:()=>{}}));assert.match(html,/其他记录进入未纳入/);assert.match(html,/当前顺序首条/);});
