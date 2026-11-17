// Verify the real golden graph gains a readable slot without losing branches.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {createRequire} from 'node:module';
import {fileURLToPath} from 'node:url';
import path from 'node:path';

const project=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const require=createRequire(path.join(project,'code','desktop','package.json'));
const ts=require('typescript');
const source=readFileSync(path.join(project,'code','desktop','src','canvas_layout.ts'),'utf8');
const js=ts.transpileModule(source,{compilerOptions:{module:ts.ModuleKind.ESNext,target:ts.ScriptTarget.ES2022}}).outputText;
const {makeInsertSpace,localNeighborhood}=await import(`data:text/javascript,${encodeURIComponent(js)}`);
const flow=JSON.parse(readFileSync(path.join(project,'tests','golden','workflow.json'),'utf8'));

const one=makeInsertSpace(flow.nodes,'je_group','compare');
const byId=new Map(one.nodes.map(node=>[node.id,node]));
assert.equal(one.position.y,flow.nodes.find(node=>node.id==='je_group').position.y);
assert.ok(one.position.x>=byId.get('je_group').position.x+220);
assert.ok(byId.get('compare').position.x>=one.position.x+220);
assert.equal(byId.get('tb_source').position.x,flow.nodes.find(node=>node.id==='tb_source').position.x);
assert.equal(byId.get('tb_group').position.y,flow.nodes.find(node=>node.id==='tb_group').position.y);
assert.ok(byId.get('output').position.x>byId.get('compare').position.x);
assert.deepEqual(new Set(localNeighborhood('compare',flow.edges)),new Set(['compare','je_group','tb_group','check']));

const two=makeInsertSpace([...one.nodes,{id:'inserted',position:one.position}], 'je_group','inserted');
const after=new Map(two.nodes.map(node=>[node.id,node]));
assert.ok(after.get('inserted').position.x>=two.position.x+220);
assert.ok(after.get('compare').position.x>after.get('inserted').position.x);
console.log('Canvas layout: two sequential insertions preserve both input branches and readable order.');
