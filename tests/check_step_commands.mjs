import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createRequire } from 'node:module';
import { readFile } from 'node:fs/promises';

const require = createRequire(new URL('../code/desktop/package.json', import.meta.url));
const ts = require('typescript');
const source = await readFile(new URL('../code/desktop/src/project.ts', import.meta.url), 'utf8');
const { outputText } = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } });
const { applyStepCommand, applyTableAction } = await import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`);

const node = (id, kind = 'filter', config = {}) => ({ id, kind, config, version: 1, label: id, position: { x: 0, y: 0 } });
const project = {
  schema_version: 1, bindings: {}, workflow: {
    schema_version: 1, name: '分支验证',
    nodes: [node('a', 'source'), node('b'), node('right', 'source'), node('join', 'join', { left_id: 'a', right_id: 'right' })],
    edges: [
      { id: 'left', source: 'a', target: 'join', target_port: 'left' },
      { id: 'right', source: 'right', target: 'join', target_port: 'right' },
      { id: 'branch', source: 'a', target: 'b' },
    ],
  },
};

test('insert changes only the specified connection and preserves the paired join input', () => {
  const before = structuredClone(project);
  const result = applyStepCommand(project, { kind: 'insert', edgeId: 'left', node: node('new'), incomingEdgeId: 'new_edge' });
  assert.deepEqual(project, before);
  assert.deepEqual(result.workflow.edges.slice(1, 3), project.workflow.edges.slice(1));
  assert.deepEqual(result.workflow.edges[0], { ...project.workflow.edges[0], source: 'new', source_port: 'main' });
  assert.deepEqual(result.workflow.nodes.find(n => n.id === 'join').config, { left_id: 'new', right_id: 'right' });
});

test('invalid or stale insert fails without changing the project', () => {
  const before = structuredClone(project);
  for (const edgeId of ['missing', 'left']) {
    assert.throws(() => applyStepCommand(project, { kind: 'insert', edgeId, node: node('new'), incomingEdgeId: 'branch' }));
  }
  assert.deepEqual(project, before);
});

test('table action requires a branch choice and preserves all other connections', () => {
  const action = { kind: 'keep', fields: ['科目'] };
  const before = structuredClone(project);
  assert.throws(() => applyTableAction(project, 'a', action, ['科目']), /请选择/);
  const result = applyTableAction(project, 'a', action, ['科目'], 'left');
  assert.equal(result.project.workflow.edges.find(e => e.id === 'left').source, result.node.id);
  assert.deepEqual(result.project.workflow.edges.find(e => e.id === 'branch'), project.workflow.edges[2]);
  assert.deepEqual(project, before);
});


test('multi-input configuration updates rules and ports atomically and rejects a cycle', () => {
  const configured={...project.workflow.nodes.find(n=>n.id==='join'),version:2,config:{mode:'left',keys:[],columns:[]}};
  const edges=[
    {id:'new_left',source:'a',target:'join',target_port:'left'},
    {id:'new_right',source:'right',target:'join',target_port:'right'},
  ];
  const before=structuredClone(project);
  const next=applyStepCommand(project,{kind:'configure_inputs',node:configured,edges});
  assert.deepEqual(project,before);
  assert.deepEqual(next.workflow.edges.filter(e=>e.target==='join'),edges);
  assert.deepEqual(next.workflow.edges.find(e=>e.id==='branch'),project.workflow.edges.find(e=>e.id==='branch'));
  assert.equal(next.workflow.nodes.find(n=>n.id==='join').version,2);
  assert.throws(()=>applyStepCommand(project,{kind:'configure_inputs',node:configured,edges:[edges[0],{...edges[1],target_port:'left'}]}));
  const cyclic=structuredClone(project);
  cyclic.workflow.edges.push({id:'cycle',source:'join',target:'a'});
  assert.throws(()=>applyStepCommand(cyclic,{kind:'configure_inputs',node:configured,edges}));
  assert.deepEqual(project,before);
});

test('inserting before a new join preserves its port without legacy input parameters', () => {
  const p=structuredClone(project);
  const join=p.workflow.nodes.find(n=>n.id==='join');
  join.version=2;join.config={mode:'left',keys:[],columns:[]};
  const next=applyStepCommand(p,{kind:'insert',edgeId:'left',node:node('trim','text_clean'),incomingEdgeId:'trim_input'});
  assert.deepEqual(next.workflow.nodes.find(n=>n.id==='join').config,join.config);
  assert.equal(next.workflow.edges.find(e=>e.id==='left').target_port,'left');
  assert.equal(next.workflow.edges.find(e=>e.id==='left').source,'trim');
});
