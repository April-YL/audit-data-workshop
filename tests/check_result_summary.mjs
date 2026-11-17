import assert from 'node:assert/strict';
import {test} from 'node:test';
import {createRequire} from 'node:module';
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {pathToFileURL} from 'node:url';
const require=createRequire(new URL('../code/desktop/package.json',import.meta.url)),ts=require('typescript');
const out=new URL('../outputs/result-summary-tests/',import.meta.url);await mkdir(out,{recursive:true});
const entries=['project','preview','components/ColumnActionPanel','components/ResultSummaryPanel','components/RowOperationsPanel','components/AggregatePanel'];
for(const entry of entries){const name=entry.split('/').pop();const src=await readFile(new URL('../code/desktop/src/'+entry+(entry.startsWith('components')?'.tsx':'.ts'),import.meta.url),'utf8');let code=ts.transpileModule(src,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ESNext,jsx:ts.JsxEmit.ReactJSX}}).outputText;code=code.replace(/(['"])(react(?:\/jsx-runtime)?|@tauri-apps\/api\/core)\1/g,(_,q,id)=>JSON.stringify(pathToFileURL(require.resolve(id)).href));code=code.replace(/(['"])(?:\.\.\/|\.\/)(project|preview|ColumnActionPanel)\1/g,(_,q,id)=>JSON.stringify('./'+id+'.mjs'));await writeFile(new URL(name+'.mjs',out),code);}
const {blankProject,newNode,applyTableAction}=await import(new URL('project.mjs',out));
const {resultRequestKey}=await import(new URL('preview.mjs',out));
const {default:Summary}=await import(new URL('ResultSummaryPanel.mjs',out));
const {default:RowPanel}=await import(new URL('RowOperationsPanel.mjs',out));
const {default:Aggregate}=await import(new URL('AggregatePanel.mjs',out));
const {createElement}=require('react'),{renderToStaticMarkup}=require('react-dom/server');
function setup(){const p=blankProject(),a=newNode('source',{x:0,y:0});p.workflow.nodes=[a];return {p,a};}
test('summary identity distinguishes rule, category, batch, selection and project',()=>{const t={nodeId:'a',view:'output',category:'normal',session:'s',revision:1,request:1};const key=resultRequestKey(t,'batch',['金额']);for(const changed of [{...t,category:'excluded'},{...t,revision:2},{...t,session:'new'},{...t,nodeId:'b'}])assert.notEqual(key,resultRequestKey(changed,'batch',['金额']));assert.notEqual(key,resultRequestKey(t,'other',['金额']));assert.notEqual(key,resultRequestKey(t,'batch',['另一列']));});
test('sample summary explicitly refuses to report partial totals',()=>{const state={status:'ready',title:'sample',direction:'output',message:'',target:{nodeId:'a',view:'output',category:'normal',session:'s',revision:1,request:1},data:{kind:'sample',columns:['金额'],rows:[['1']],totalRows:null}};const html=renderToStaticMarkup(createElement(Summary,{state,result:null,fieldTypes:{金额:'DECIMAL(28,4)'}}));assert.match(html,/局部样例不参与合计/);assert.doesNotMatch(html,/合计 1/);});
