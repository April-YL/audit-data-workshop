// Structural state tests only; no customer records or financial values are embedded.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createRequire } from 'node:module';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { fileURLToPath, pathToFileURL } from 'node:url';
const require = createRequire(new URL('../code/desktop/package.json', import.meta.url));
const ts = require('typescript');
const resolveComponentDependency = id => {
  const entry = require.resolve(id);
  return pathToFileURL(id.startsWith('ag-grid-') ? entry.replace(/\.cjs\.js$/, '.esm.mjs') : entry).href;
};
const output = new URL('../outputs/preview-state-tests/', import.meta.url);
await mkdir(output, { recursive: true });
for (const name of ['project', 'preview']) {
  const source = await readFile(new URL(`../code/desktop/src/${name}.ts`, import.meta.url), 'utf8');
  const compiled = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText.replaceAll("'./project'", "'./project.mjs'");
  await writeFile(new URL(`${name}.mjs`, output), compiled);
}
const { resolvePreview, acceptsResponse, dataSignature, sourceIdentity } = await import(new URL('preview.mjs', output));
const { incomingLinks } = await import(new URL('project.mjs', output));
const actionSource=await readFile(new URL('../code/desktop/src/components/ColumnActionPanel.tsx',import.meta.url),'utf8');
await writeFile(new URL('ColumnActionPanel.mjs',output),ts.transpileModule(actionSource,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ESNext,jsx:ts.JsxEmit.ReactJSX}}).outputText.replaceAll('"react/jsx-runtime"',JSON.stringify(pathToFileURL(require.resolve('react/jsx-runtime')).href)).replaceAll("'react'",JSON.stringify(pathToFileURL(require.resolve('react')).href)));
for(const name of ['ResultSummaryPanel','FormulaColumnPanel','RowOperationsPanel','AggregatePanel','ResultGrid','PivotPanel','StepEditDialog','ExportPanel']){const src=await readFile(new URL('../code/desktop/src/components/'+name+'.tsx',import.meta.url),'utf8');let code=ts.transpileModule(src,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ESNext,jsx:ts.JsxEmit.ReactJSX}}).outputText;code=code.replace(/(['"])(react(?:\/jsx-runtime)?|@tauri-apps\/api\/core|@tauri-apps\/plugin-dialog|ag-grid-react|ag-grid-community)\1/g,(_,q,id)=>JSON.stringify(resolveComponentDependency(id))).replace(/(['"])(?:\.\.\/|\.\/)(project|preview|ColumnActionPanel|StepEditDialog)\1/g,(_,q,id)=>JSON.stringify('./'+id+'.mjs'));await writeFile(new URL(name+'.mjs',output),code);}
const componentSource = await readFile(new URL('../code/desktop/src/components/NodePreviewPanel.tsx', import.meta.url),'utf8');
const componentJs = ts.transpileModule(componentSource,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ESNext,jsx:ts.JsxEmit.ReactJSX}}).outputText
  .replaceAll("'@tauri-apps/api/core'",JSON.stringify(pathToFileURL(require.resolve('@tauri-apps/api/core')).href))
  .replaceAll('"react/jsx-runtime"',JSON.stringify(pathToFileURL(require.resolve('react/jsx-runtime')).href))
  .replaceAll("'react'",JSON.stringify(pathToFileURL(require.resolve('react')).href))
  .replaceAll("'../project'","'./project.mjs'").replace(/(['"])(?:\.\/)(ColumnActionPanel|ResultSummaryPanel|FormulaColumnPanel|RowOperationsPanel|AggregatePanel|ResultGrid|PivotPanel|StepEditDialog|ExportPanel)\1/g,(_,q,id)=>JSON.stringify('./'+id+'.mjs'));
await writeFile(new URL('NodePreviewPanel.mjs',output),componentJs);
const {default:NodePreviewPanel}=await import(new URL('NodePreviewPanel.mjs',output));
const {createElement}=require('react');
const {renderToStaticMarkup}=require('react-dom/server');
const node = (id, kind, config={}) => ({ id, kind, label:id, version:1, position:{x:0,y:0}, config });
const project = { schema_version:1, bindings:{input:{path:'approved-local.xlsx',fields:{},sheet:'B'}}, workflow:{ schema_version:1,name:'预览归属检查',nodes:[node('input','source',{normalize_structure:true}),node('map','map'),node('filter','filter')],edges:[{id:'a',source:'input',target:'map'},{id:'b',source:'map',target:'filter'}]} };
const sample = {headers:['科目编号','余额 / 金额'],rows:[],physical_rows:[],sheet:'B',truncated:true};
const raw = {columns:['第1列','第2列'],rows:[],physical_rows:[],sheet:'B',truncated:true};
const sources = {input:{raw,applied:sample,appliedKey:sourceIdentity(project,'input'),prepared:sample,preparedFresh:true,confirmed:true}};
const context = {project,sources,run:null};
const query = {nodeId:'map',view:'auto',category:'normal'};
test('original regression: map input uses confirmed prepared fields, never the raw grid',()=>{
  const result=resolvePreview(context,query);
  assert.equal(result.status,'ready'); assert.equal(result.direction,'input'); assert.equal(result.sourceId,'input');
  assert.deepEqual(result.data.columns,['原始行',...sample.headers]); assert.ok(!result.data.columns.includes('第1列'));
});
test('uncomputed output and downstream of an uncomputed map are explicit, not disguised input',()=>{
  assert.equal(resolvePreview(context,{...query,view:'output'}).status,'pending');
  assert.equal(resolvePreview(context,{...query,nodeId:'filter'}).status,'pending');
});
test('another node kind uses the shared input and pending-output contract without its own fallback',()=>{
  const p=structuredClone(project);p.workflow.nodes[1].kind='decimal';
  const c={...context,project:p};
  assert.equal(resolvePreview(c,query).direction,'input');
  assert.equal(resolvePreview(c,query).status,'ready');
  assert.equal(resolvePreview(c,{...query,view:'output'}).status,'pending');
});
test('source raw is explicit and draft samples cannot pass into downstream',()=>{
  assert.equal(resolvePreview(context,{...query,nodeId:'input',view:'raw'}).data.kind,'raw');
  assert.notEqual(resolvePreview({...context,sources:{input:{...sources.input,confirmed:false}}},query).status,'ready');
  assert.notEqual(resolvePreview({...context,sources:{input:{...sources.input,appliedKey:'old'}}},query).status,'ready');
});
test('running results resolve selected input/output and result category to the exact table',()=>{
  const run={database:'local.duckdb',run_dir:'local-run',reviews:[{node_id:'input',label:'input',table:'n_input',rows:0,excluded:0,exceptions:0},{node_id:'map',label:'map',table:'n_map',rows:0,excluded:0,exceptions:0},{node_id:'filter',label:'filter',table:'n_filter',rows:0,excluded:1,exceptions:0}],input_fingerprints:{}};
  const c={...context,run};
  assert.equal(resolvePreview(c,query).table,'n_map');
  assert.equal(resolvePreview(c,{...query,view:'input'}).table,'n_input');
  assert.equal(resolvePreview(c,{...query,nodeId:'filter',view:'output',category:'excluded'}).table,'n_filter__excluded');
  assert.equal(resolvePreview(c,{...query,view:'output',category:'exceptions'}).status,'unsupported');
  assert.equal(resolvePreview({...c,sources:{}},{...query,nodeId:'input'}).table,'n_input');
});
test('multiple inputs require an actual edge choice and do not silently select the first',()=>{
  const p=structuredClone(project); p.workflow.edges.push({id:'c',source:'filter',target:'map',target_port:'right'});
  assert.equal(incomingLinks(p,'map').length,2);
  assert.equal(resolvePreview({...context,project:p},query).status,'pending');
  assert.equal(resolvePreview({...context,project:p},{...query,inputEdgeId:'missing'}).status,'pending');
});
test('responses are rejected after node, direction, port, category, revision, project or request changes',()=>{
  const target={session:'one',revision:1,nodeId:'map',view:'input',category:'normal',inputEdgeId:'a',request:2};
  assert.equal(acceptsResponse(target,target),true);
  for(const change of [{session:'two'},{revision:2},{nodeId:'filter'},{view:'output'},{category:'excluded'},{inputEdgeId:'b'},{request:3},{offset:200}]) assert.equal(acceptsResponse(target,{...target,...change}),false);
});
test('display changes preserve data identity; rules, bindings, graph and restored revisions do not',()=>{
  const moved=structuredClone(project);moved.workflow.nodes[0].position.x=100;moved.workflow.nodes[0].label='输入';
  assert.equal(dataSignature(project),dataSignature(moved));
  for(const mutate of [p=>p.workflow.nodes[1].config.mappings=[],p=>p.bindings.input.sheet='另一表',p=>p.workflow.edges.pop()]) {const p=structuredClone(project);mutate(p);assert.notEqual(dataSignature(project),dataSignature(p));}
  const renamed=structuredClone(project);renamed.workflow.nodes[1].config.mappings=[{source:'科目编号',target:'复核编码'}];
  assert.equal(sourceIdentity(renamed,'input'),sourceIdentity(project,'input'),'editing downstream must not remove a still-valid source field list');
  assert.equal(acceptsResponse({session:'one',revision:3,request:1},{session:'one',revision:1,request:1}),false);
});
test('actual panel clears old records in loading/error/stale/pending/unsupported states',()=>{
  const plan=resolvePreview(context,query);
  const target={...query,session:'one',revision:1,request:1};
  const props={query,node:project.workflow.nodes[1],links:[],reviews:[],change(){},select(){},close(){},rawRow(){},headerRows:[],focus:null};
  const ready=renderToStaticMarkup(createElement(NodePreviewPanel,{...props,state:{...plan,target}}));
  assert.ok(ready.includes('科目编号')); assert.ok(ready.includes('本步输入'));
  for(const status of ['loading','error','stale','pending','unsupported']) {
    const html=renderToStaticMarkup(createElement(NodePreviewPanel,{...props,state:{...plan,target,status,message:'待复核'}}));
    assert.ok(!html.includes('<table')); assert.ok(!html.includes('科目编号'));
  }
});
console.log(`Preview state checks: ${fileURLToPath(output)}`);
