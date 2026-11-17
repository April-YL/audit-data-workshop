import assert from 'node:assert/strict';
import {test} from 'node:test';
import {createRequire} from 'node:module';
import {readFile,mkdir,writeFile} from 'node:fs/promises';
const require=createRequire(new URL('../code/desktop/package.json',import.meta.url));
const ts=require('typescript');const dir=new URL('../outputs/poc-action-tests/',import.meta.url);await mkdir(dir,{recursive:true});
const source=await readFile(new URL('../code/desktop/src/project.ts',import.meta.url),'utf8');
await writeFile(new URL('project.mjs',dir),ts.transpileModule(source,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ESNext}}).outputText);
const {blankProject,newNode,applyTableAction,projectSnapshot,hasUnsavedProject}=await import(new URL('project.mjs',dir));
function setup(){const p=blankProject();const a=newNode('source',{x:0,y:0});const b=newNode('filter',{x:250,y:0});p.workflow.nodes=[a,b];p.workflow.edges=[{id:'original',source:a.id,target:b.id}];return {p,a,b};}
test('keep columns creates a real map and reconnects downstream in one immutable edit',()=>{const {p,a,b}=setup();const snapshot=projectSnapshot(p);const r=applyTableAction(p,a.id,{kind:'keep',fields:['科目编号']},['原始行','__row','科目编号','科目名称']);assert.equal(projectSnapshot(p),snapshot);assert.deepEqual(r.node.config.mappings,[{source:'科目编号',target:'科目编号'}]);assert.ok(r.project.workflow.edges.some(e=>e.source===a.id&&e.target===r.node.id));assert.ok(r.project.workflow.edges.some(e=>e.source===r.node.id&&e.target===b.id));assert.equal(r.project.workflow.edges.filter(e=>e.target===b.id).length,1);});
test('rename preserves other columns; filtering records the real rule without deleting values in JS',()=>{const {p,a}=setup();const r=applyTableAction(p,a.id,{kind:'rename',field:'科目编号',name:'复核编码'},['科目编号','科目名称']);assert.deepEqual(r.node.config.mappings,[{source:'科目编号',target:'复核编码'},{source:'科目名称',target:'科目名称'}]);const f=applyTableAction(r.project,r.node.id,{kind:'filter',condition:{field:'复核编码',operator:'contains',value:'1'}},['复核编码','科目名称']);assert.equal(f.node.kind,'filter');assert.equal(f.node.config.condition.field,'复核编码');});
test('empty selection, duplicate/reserved names and absent fields are rejected',()=>{const {p,a}=setup();for(const action of [{kind:'keep',fields:[]},{kind:'rename',field:'科目编号',name:'科目名称'},{kind:'rename',field:'科目编号',name:'__row'},{kind:'keep',fields:['不存在']}])assert.throws(()=>applyTableAction(p,a.id,action,['科目编号','科目名称']));});
test('dirty protection includes data, layout and pending reading, but successful snapshot clears dirty',()=>{const {p}=setup();const saved=projectSnapshot(p);assert.equal(hasUnsavedProject(p,saved,false),false);assert.equal(hasUnsavedProject(p,saved,true),true);const changed=structuredClone(p);changed.workflow.nodes[0].position.x++;assert.equal(hasUnsavedProject(changed,saved,false),true);assert.equal(hasUnsavedProject(changed,projectSnapshot(changed),false),false);});

test('source business fields retain existing columns and require real provenance',()=>{
  const {p,a}=setup();
  const r=applyTableAction(p,a.id,{kind:'source_fields',fields:[{source:'__row',target:'原始行号'}]},['__row','科目编号']);
  assert.deepEqual(r.node.config.mappings,[{source:'科目编号',target:'科目编号'},{source:'__row',target:'原始行号'}]);
  assert.throws(()=>applyTableAction(p,a.id,{kind:'source_fields',fields:[{source:'__left__row',target:'左行号'}]},['__row','科目编号']));
  assert.throws(()=>applyTableAction(p,a.id,{kind:'source_fields',fields:[{source:'__row',target:'科目编号'}]},['__row','科目编号']));
});

test('editing a filter updates the same step and preserves its upstream history',()=>{
  const {p,a,b}=setup();
  b.config={match:'all',conditions:[{field:'科目编号',operator:'equals',value:'001'}]};
  const originalEdges=structuredClone(p.workflow.edges);
  const r=applyTableAction(p,b.id,{kind:'update_filter',config:{match:'all',conditions:[],cleared:true}},['科目编号']);
  assert.equal(r.node.id,b.id);
  assert.equal(r.project.workflow.nodes.length,p.workflow.nodes.length);
  assert.deepEqual(r.project.workflow.edges,originalEdges);
  assert.deepEqual(r.project.workflow.nodes.find(n=>n.id===a.id),a);
  assert.equal(p.workflow.nodes.find(n=>n.id===b.id).config.conditions.length,1);
  assert.equal(r.node.config.cleared,true);
});
