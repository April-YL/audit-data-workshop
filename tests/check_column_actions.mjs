import assert from 'node:assert/strict';
import {test} from 'node:test';
import {createRequire} from 'node:module';
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {pathToFileURL} from 'node:url';
const require=createRequire(new URL('../code/desktop/package.json',import.meta.url)),ts=require('typescript');
const dir=new URL('../outputs/column-action-tests/',import.meta.url);await mkdir(dir,{recursive:true});
for(const [file,out] of [['project.ts','project.mjs'],['components/ColumnActionPanel.tsx','ColumnActionPanel.mjs']]){
 const src=await readFile(new URL('../code/desktop/src/'+file,import.meta.url),'utf8');
 await writeFile(new URL(out,dir),ts.transpileModule(src,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ESNext,jsx:ts.JsxEmit.ReactJSX}}).outputText.replaceAll('"react/jsx-runtime"',JSON.stringify(pathToFileURL(require.resolve('react/jsx-runtime')).href)).replaceAll("'react'",JSON.stringify(pathToFileURL(require.resolve('react')).href)));
}
const {blankProject,newNode,applyTableAction}=await import(new URL('project.mjs',dir));
const {default:Panel,isNumericType}=await import(new URL('ColumnActionPanel.mjs',dir));
const {createElement}=require('react'),{renderToStaticMarkup}=require('react-dom/server');
const cols=['编码','借方','贷方'];
function apply(action){const p=blankProject(),n=newNode('source',{x:0,y:0});p.workflow.nodes=[n];const before=JSON.stringify(p);const r=applyTableAction(p,n.id,action,cols);assert.equal(JSON.stringify(p),before);assert.equal(r.project.workflow.nodes.length,2);assert.equal(r.project.workflow.edges.length,1);return r.node;}
test('five column operations record real rules and preserve input fields',()=>{
 assert.deepEqual(apply({kind:'decimal',fields:[{source:'借方',target:'借方_金额'},{source:'贷方',target:'贷方_金额'}]}).config.fields.map(m=>m.source),['借方','贷方']);
 for(const mode of ['trim','replace'])assert.equal(apply({kind:'text_clean',fields:[{source:'编码',target:'清理编码'}],mode,find:'-',replacement:''}).config.mode,mode);
 assert.equal(apply({kind:'text_extract',source:'编码',target:'提取编码',pattern:'([0-9]+)',group:1}).config.pattern,'([0-9]+)');

 for(const op of ['numeric_gt','numeric_gte','numeric_lt','numeric_lte','numeric_eq','numeric_neq'])assert.equal(apply({kind:'filter',condition:{field:'借方',operator:op,value:'0.0001'}}).config.condition.value,'0.0001');
});
test('new-column actions reject collisions, reserved names, duplicate selections and invalid parameters',()=>{
 for(const fields of [[{source:'借方',target:'贷方'}],[{source:'不存在',target:'新列'}],[{source:'借方',target:'COMPANY'}],[{source:'借方',target:'__row'}],[{source:'借方',target:'新列'},{source:'贷方',target:'新列'}]])assert.throws(()=>apply({kind:'decimal',fields}));
 assert.throws(()=>apply({kind:'text_clean',fields:[{source:'编码',target:'新列'}],mode:'replace',find:'',replacement:'x'}));
 assert.throws(()=>apply({kind:'text_extract',source:'编码',target:'新列',pattern:'',group:1}));
});
test('numeric eligibility follows authoritative SQL types, never values or column names',()=>{
 assert.ok(isNumericType('DECIMAL(28,4)'));assert.ok(isNumericType('BIGINT'));
 for(const t of ['VARCHAR','未知','金额','',undefined])assert.equal(isNumericType(t),false);
});
test('panel presents all five actions but disables them when no columns are selected',()=>{
 const html=renderToStaticMarkup(createElement(Panel,{fields:cols,selected:[],onAction:()=>{}}));
 for(const label of ['转换为金额','去首尾空格','普通文本替换','文本提取','数值筛选'])assert.ok(html.includes(label));
 assert.equal((html.match(/disabled=""/g)||[]).length,5);
 assert.ok(!html.includes("两列相减"));
});
