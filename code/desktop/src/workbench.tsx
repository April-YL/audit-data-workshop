import React, { useEffect, useMemo, useRef, useState } from 'react';
import { ReactFlow, Background, Controls, Handle, Position, type Connection, type Edge, type Node as FlowNode, type NodeChange, type EdgeChange, type NodeProps, type ReactFlowInstance } from '@xyflow/react';
import { invoke } from '@tauri-apps/api/core';
import { open, save } from '@tauri-apps/plugin-dialog';
import { listen } from '@tauri-apps/api/event';
import '@xyflow/react/dist/style.css';
import './style.css';
import SourceReadingPanel from './components/SourceReadingPanel';
import NodePreviewPanel from './components/NodePreviewPanel';
import ColumnActionPanel from './components/ColumnActionPanel';
import FormulaColumnPanel from './components/FormulaColumnPanel';
import RowOperationsPanel from './components/RowOperationsPanel';
import AggregatePanel from './components/AggregatePanel';
import JoinAppendPanel from './components/JoinAppendPanel';
import PivotPanel from './components/PivotPanel';
import type {NodeContract} from './preview';
import ProjectGuardDialog from './components/ProjectGuardDialog';
import { acceptsResponse, dataSignature, sourceIdentity, resolvePreview, type PreviewQuery, type PreviewState, type PreviewTarget, type SourceEvidence } from './preview';
import { incomingLinks } from './project';
import { applyTableAction, hasUnsavedProject, projectSnapshot, type TableAction } from './project';
import { applyStepCommand } from './step_commands';
import { blankProject, incomingNode, newNode, toolCatalog, visibleFields, type Condition, type Kind, type Mapping, type Preview, type ProjectFile, type RawSourceSample, type Review, type RunResult, type SourceBinding, type SourceSample, type StructureReport, type ReadingIssue, type ReadingCheck, type TableLayout, type TextOptions, type WorkNode } from './project';

function SourceNode({ data }: NodeProps) {
  return <div className="node-body" onDoubleClick={event => { event.stopPropagation(); (data.onOpen as (() => void) | undefined)?.(); }}>{data.label as React.ReactNode}<Handle type="source" id="main" position={Position.Right} /></div>;
}
function StepNode({ data }: NodeProps) {
  return <div className="node-body"><Handle type="target" id="in" position={Position.Left} />{data.label as React.ReactNode}<Handle type="source" id="main" position={Position.Right} /></div>;
}
function MultiInputNode({ data }: NodeProps) {
  const ports = data.ports as string[];
  return <div className="node-body" style={{minHeight:Math.max(80,ports.length*28)}}>{ports.map((port,index)=><Handle key={port} type="target" id={port} position={Position.Left} style={{top:`${(index+1)*100/(ports.length+1)}%`}} />)}{data.label as React.ReactNode}<Handle type="source" id="main" position={Position.Right} /></div>;
}
function ExportNode({ data }: NodeProps) {
  return <div className="node-body"><Handle type="target" id="in" position={Position.Left} />{data.label as React.ReactNode}</div>;
}
const nodeTypes = { source: SourceNode, step: StepNode, multi: MultiInputNode, export: ExportNode };
const fileName = (path: string) => path.split(/[\\/]/).pop() || path;
const errorText = (error: unknown) => error instanceof Error ? error.message : String(error);
const loadFilterValues = (batch: string, nodeId: string, field: string, search: string, offset: number,filterConfig?:Record<string,unknown>) =>
  invoke<(string | null)[]>('distinct_result_values', {batch,nodeId,field,search,offset,filterConfig});
const operators = [
  ['equals', '等于'], ['not_equals', '不等于'], ['contains', '包含'], ['is_empty', '为空'], ['is_not_empty', '不为空'],
  ['numeric_gt','数值大于'], ['numeric_gte','数值大于等于'], ['numeric_lt','数值小于'], ['numeric_lte','数值小于等于'], ['numeric_eq','数值等于'], ['numeric_neq','数值不等于'],
  ['date_gt','日期晚于'],['date_gte','日期不早于'],['date_lt','日期早于'],['date_lte','日期不晚于'],['date_eq','日期等于'],['date_neq','日期不等于'],
];


export default function Workbench() {
  const [project, setProject] = useState<ProjectFile>(blankProject);
  const [projectPath, setProjectPath] = useState('');
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [selectedEdgeId, setSelectedEdgeId] = useState<string | null>(null);
  const [tableInsertEdgeId, setTableInsertEdgeId] = useState('');
  const [workView, setWorkView] = useState<'split' | 'table' | 'canvas'>('split');
  const [group, setGroup] = useState('输入');
  const [search, setSearch] = useState('');
  const [toolOpen, setToolOpen] = useState(true);
  const [samples, setSamples] = useState<Record<string, SourceSample>>({});
  const [rawSamples, setRawSamples] = useState<Record<string, RawSourceSample>>({});
  const [inputErrors, setInputErrors] = useState<Record<string, string>>({});
  const [sampleKeys, setSampleKeys] = useState<Record<string,string>>({});
  const [sourceLoading, setSourceLoading] = useState<Record<string,boolean>>({});
  const [previewChoice, setPreviewChoice] = useState<PreviewQuery | null>(null);
  const [nodePreview, setNodePreview] = useState<PreviewState | null>(null);
  const [resultsOpen, setResultsOpen] = useState(false);
  const [sheets, setSheets] = useState<Record<string, string[]>>({});
  const [structures, setStructures] = useState<Record<string, StructureReport>>({});
  const [drafts, setDrafts] = useState<Record<string, TableLayout>>({});
  const [configDrafts,setConfigDrafts]=useState<Record<string,{snapshot:string;config:Record<string,unknown>}>>({});
  const configDraftRef=useRef(configDrafts);configDraftRef.current=configDrafts;
  const [checkedLayouts, setCheckedLayouts] = useState<Record<string, string>>({});
  const [pendingBindings, setPendingBindings] = useState<Record<string, SourceBinding>>({});
  const [readingDifferences, setReadingDifferences] = useState<Record<string, ReadingIssue[]>>({});
  const [focusCell, setFocusCell] = useState<{nodeId:string;row: number | null; col: number | null} | null>(null);
  const [pickRow, setPickRow] = useState<'header_start' | 'header_end' | 'data_start' | null>(null);
  const [run, setRun] = useState<RunResult | null>(null);
  const [runFields, setRunFields] = useState<Record<string,{key:string;headers:string[]}>>({});
  const [contracts,setContracts]=useState<{key:string;values:Record<string,NodeContract>}>({key:'',values:{}});
  const [busy, setBusy] = useState(false);
  const [guardOpen,setGuardOpen]=useState(false);
  const [guardBusy,setGuardBusy]=useState(false);
  const savedSnapshotRef=useRef(projectSnapshot(project));
  const guardedActionRef=useRef<null|(()=>void|Promise<void>)>(null);
  const closeHandlerRef=useRef<()=>void>(()=>{});
  const [status, setStatus] = useState('新建项目为空白画布。选择文件开始，或从工具库添加操作。');
  const [flowApi, setFlowApi] = useState<ReactFlowInstance | null>(null);
  // Display-only split position: never enters ProjectFile or dataSignature.
  const workAreaRef = useRef<HTMLDivElement>(null);
  const [workAreaHeight, setWorkAreaHeight] = useState(0);
  const [resultsFraction, setResultsFraction] = useState(0.45);
  const [resultsExpanded, setResultsExpanded] = useState(false);
  const [resizingResults, setResizingResults] = useState(false);
  const resultsDrag = useRef<{pointerId:number;y:number;bottom:number}|null>(null);
  useEffect(()=>{
    const area=workAreaRef.current;if(!area)return;
    const observer=new ResizeObserver(()=>setWorkAreaHeight(area.clientHeight));
    setWorkAreaHeight(area.clientHeight);observer.observe(area);
    return()=>observer.disconnect();
  },[]);
  const resultsMax=Math.max(0,workAreaHeight-104);
  const resultsMin=Math.min(260,resultsMax);
  const clampResults=(height:number)=>Math.max(resultsMin,Math.min(resultsMax,height));
  const resultsHeight=clampResults(resultsExpanded?resultsMax:workAreaHeight*resultsFraction);
  const resizeResults=(height:number)=>{
    if(workAreaHeight>0)setResultsFraction(clampResults(height)/workAreaHeight);
    setResultsExpanded(false);
  };
  const resetResults=()=>{setResultsFraction(0.45);setResultsExpanded(false);};
  const startResultsDrag=(event:React.PointerEvent<HTMLDivElement>)=>{
    if(event.button!==0||workAreaHeight<=0)return;
    event.preventDefault();event.currentTarget.setPointerCapture(event.pointerId);
    resultsDrag.current={pointerId:event.pointerId,y:event.clientY,bottom:resultsHeight};
    setResizingResults(true);
  };
  const moveResultsDrag=(event:React.PointerEvent<HTMLDivElement>)=>{
    const drag=resultsDrag.current;if(!drag||drag.pointerId!==event.pointerId)return;
    resizeResults(drag.bottom+drag.y-event.clientY);
  };
  const endResultsDrag=(event:React.PointerEvent<HTMLDivElement>)=>{
    if(resultsDrag.current?.pointerId!==event.pointerId)return;
    resultsDrag.current=null;setResizingResults(false);
    if(event.currentTarget.hasPointerCapture(event.pointerId))event.currentTarget.releasePointerCapture(event.pointerId);
  };
  const projectRef = useRef(project);
  const projectRevisionRef = useRef(0);
  const sessionRef = useRef(crypto.randomUUID());
  const dataRevisionRef = useRef(0);
  const previewRequestRef = useRef(0);
  const previewTargetRef = useRef<PreviewTarget | null>(null);
  const selectedIdRef = useRef(selectedId);
  if (selectedIdRef.current !== selectedId) { selectedIdRef.current = selectedId; previewTargetRef.current = null; }
  const runSignatureRef = useRef<string | null>(null);
  const inspectSerialRef = useRef<Record<string, number>>({});
  const reviewSerialRef = useRef(0);
  const draftSerialRef = useRef<Record<string, number>>({});
  const dirtyReadingRef = useRef(new Set<string>());
  const pastRef = useRef<ProjectFile[]>([]);
  const futureRef = useRef<ProjectFile[]>([]);
  const [, setHistoryVersion] = useState(0);

  const selected = project.workflow.nodes.find(node => node.id === selectedId);
  const review = run?.reviews.find(item => item.node_id === selectedId);
  const input = selected ? incomingNode(project, selected.id) : undefined;
  const fieldSamples = useMemo(()=>({...samples,...Object.fromEntries(Object.entries(runFields).filter(([id,value])=>value.key===sourceIdentity(project,id)).map(([id,{headers}])=>[id,{headers,rows:[],physical_rows:[],truncated:true,sheet:''}]))}),[samples,runFields,project]);
  const contractKey=JSON.stringify([sessionRef.current,dataSignature(project)]);
  const activeContracts=contracts.key===contractKey?contracts.values:{};
  const inputSchema=input?activeContracts[input.id]?.outputs.normal:undefined;
  const fields = inputSchema ? inputSchema.filter(f=>!f.name.startsWith('__')&&f.name!=='company').map(f=>f.name) : input ? visibleFields(project, input.id, fieldSamples) : [];
  useEffect(()=>{
    let current=true;
    if(!selected)return;
    const ids=['join','union'].includes(selected.kind)
      ? project.workflow.nodes.filter(node=>node.kind!=='export').map(node=>node.id)
      : [selected.id,...(input?[input.id]:[])];
    void Promise.all(ids.map(async id=>{
      const currentRun=run&&runSignatureRef.current===dataSignature(project)&&run.reviews.some(review=>review.node_id===id)?run:null;
      const contract=currentRun
        ? await invoke<NodeContract>('infer_result_outputs',{batch:currentRun.run_dir,nodeId:id})
        : await invoke<NodeContract>('infer_outputs',{project,nodeId:id});
      return [id,contract] as const;
    })).then(values=>{
      if(current)setContracts({key:contractKey,values:Object.fromEntries(values)});
    }).catch(()=>{if(current)setContracts({key:contractKey,values:{}});});
    return ()=>{current=false;};
  },[contractKey,selectedId,input?.id,run]);
  const selectedConfig=selected&&configDrafts[selected.id]?.snapshot===JSON.stringify([selected.version,selected.config])?configDrafts[selected.id].config:selected?.config;
  const mapping = selected?.kind === 'map' ? (selectedConfig?.mappings as Mapping[] | undefined) ?? [] : [];
  const condition = selected?.kind === 'filter' ? (selectedConfig?.condition as Condition | undefined) ?? { field: '', operator: 'equals', value: '' } : null;
  const modernMap = selected?.kind === 'map' && Array.isArray(selected.config.mappings);
  const modernFilter = selected?.kind === 'filter' && Boolean(selected.config.condition);
  const showResults = resultsOpen && Boolean(selected);
  useEffect(()=>{if(!showResults){resultsDrag.current=null;setResizingResults(false);}},[showResults]);
  const invalidate = () => { if(runSignatureRef.current)setStatus('输入或处理规则已变化，旧运行结果已失效，请重新运行。'); dataRevisionRef.current += 1; void invoke('invalidate_result', {revision:dataRevisionRef.current}).catch(error=>setStatus(`结果失效同步失败：${errorText(error)}`)); previewTargetRef.current = null; reviewSerialRef.current += 1; runSignatureRef.current = null; setRun(null); setNodePreview(null); };
  const replaceProject = (next: ProjectFile) => { sessionRef.current = crypto.randomUUID(); configDraftRef.current={};setConfigDrafts({});invalidate(); setRunFields({}); setPreviewChoice(null); setSampleKeys({}); setSourceLoading({}); projectRevisionRef.current += 1; projectRef.current = next; setProject(next); pastRef.current = []; futureRef.current = []; setHistoryVersion(v => v + 1); };
  const editProject = (change: (previous: ProjectFile) => ProjectFile) => {
    const previous = projectRef.current;
    const next = change(previous);
    if (next === previous) return;
    projectRevisionRef.current += 1;
    pastRef.current = [...pastRef.current, previous].slice(-60);
    futureRef.current = [];
    projectRef.current = next;
    setProject(next);
    setHistoryVersion(v => v + 1);
    if (dataSignature(previous) !== dataSignature(next)) invalidate();
  };
  const clearPreviews = (id: string) => {
    setRunFields(previous=>{const next={...previous};delete next[id];return next;});
    dirtyReadingRef.current.delete(id);
    draftSerialRef.current[id] = (draftSerialRef.current[id] || 0) + 1;
    setReadingDifferences(previous => { const next = {...previous}; delete next[id]; return next; });
    setFocusCell(null);
    setStructures(previous => { const next = { ...previous }; delete next[id]; return next; });
    setDrafts(previous => { const next = { ...previous }; delete next[id]; return next; });
    setCheckedLayouts(previous => { const next = { ...previous }; delete next[id]; return next; });
    inspectSerialRef.current[id] = (inspectSerialRef.current[id] || 0) + 1;
    setSamples(previous => { const next = { ...previous }; delete next[id]; return next; });
    setRawSamples(previous => { const next = { ...previous }; delete next[id]; return next; });
    setInputErrors(previous => { const next = { ...previous }; delete next[id]; return next; });
    setSampleKeys(previous => {const next={...previous};delete next[id];return next;});
    setSourceLoading(previous => ({...previous,[id]:false}));
  };
  const restoreProject = (direction: 'undo' | 'redo') => {
    const from = direction === 'undo' ? pastRef.current : futureRef.current;
    const to = direction === 'undo' ? futureRef : pastRef;
    const previous = from.pop();
    if (!previous) return;
    projectRevisionRef.current += 1;
    to.current.push(projectRef.current);
    projectRef.current = previous;
    setProject(previous);
    dirtyReadingRef.current.clear(); setPendingBindings({}); setReadingDifferences({}); setFocusCell(null); setStructures({}); setDrafts({}); setCheckedLayouts({}); setPickRow(null); setSamples({}); setRawSamples({}); setInputErrors({}); setResultsOpen(false); invalidate();
    setSelectedId(null); setSelectedEdgeId(null);
    setHistoryVersion(v => v + 1);
    setStatus(direction === 'undo' ? '已撤销；输入预览需重新读取。' : '已重做；输入预览需重新读取。');
  };
  const deleteSelection = () => {
    if (selectedEdgeId) {
      editProject(previous => applyStepCommand(previous, { kind: 'disconnect', edgeId: selectedEdgeId }));
      setSelectedEdgeId(null); setStatus('已断开连接；两端步骤仍在画布上。');
    } else if (selectedId) {
      const id = selectedId;
      editProject(previous => applyStepCommand(previous, { kind: 'remove', nodeId: id }));
      setPendingBindings(previous => { const next = {...previous}; delete next[id]; return next; });
      clearPreviews(id); setSelectedId(null); setStatus('已删除步骤及关联连线；本机原始文件未删除。');
    }
  };
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const target = event.target;
      if (target instanceof HTMLElement && target.closest('input, textarea, select, [contenteditable="true"]')) return;
      if (event.ctrlKey && !event.altKey && event.key.toLowerCase() === 'z') { event.preventDefault(); restoreProject('undo'); }
      else if (event.ctrlKey && !event.altKey && event.key.toLowerCase() === 'y') { event.preventDefault(); restoreProject('redo'); }
      else if (event.key === 'Delete' && !event.ctrlKey && !event.altKey && (selectedId || selectedEdgeId)) { event.preventDefault(); deleteSelection(); }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });
  const editNode = (nodeId: string, change: (node: WorkNode) => WorkNode) => editProject(previous => {
    const node = previous.workflow.nodes.find(item => item.id === nodeId);
    return node ? applyStepCommand(previous, { kind: 'update', node: change(node) }) : previous;
  });
  const editConfig = (nodeId: string, config: Record<string, unknown>) => editNode(nodeId, node => ({ ...node, config: { ...node.config, ...config } }));
  const stageConfig=(nodeId:string,change:Record<string,unknown>)=>{
    const node=projectRef.current.workflow.nodes.find(node=>node.id===nodeId);
    if(!node)return;
    setConfigDrafts(previous=>{
      const snapshot=JSON.stringify([node.version,node.config]);
      const base=previous[nodeId]?.snapshot===snapshot?previous[nodeId].config:node.config;
      const config={...base,...change};
      const next={...previous};
      if(JSON.stringify(config)===JSON.stringify(node.config))delete next[nodeId];
      else next[nodeId]={snapshot,config};
      return next;
    });
  };
  const discardConfig=(nodeId:string)=>setConfigDrafts(previous=>{const next={...previous};delete next[nodeId];return next;});
  const applyConfigDraft=(nodeId:string)=>{
    const node=projectRef.current.workflow.nodes.find(node=>node.id===nodeId);
    const draft=configDraftRef.current[nodeId];
    if(!node||!draft)return;
    if(draft.snapshot!==JSON.stringify([node.version,node.config])){setStatus('规则已变化，请放弃此草稿后重新编辑。');return;}
    editNode(nodeId,node=>({...node,config:draft.config}));
    discardConfig(nodeId);
    setStatus('规则已应用，撤销可恢复本次修改。请重新计算。');
  };
  const editBinding = (nodeId: string, binding: SourceBinding) => editProject(previous => ({ ...previous, bindings: { ...previous.bindings, [nodeId]: binding } }));

  const flowNodes: FlowNode[] = useMemo(() => project.workflow.nodes.map(node => {
    const binding = project.bindings[node.id];
    const subtitle = node.kind === 'source' ? (inputErrors[node.id] ? '读取设置待核对' : binding?.path ? fileName(binding.path) : '尚未选择文件')
      : node.kind === 'export' ? (String(node.config.output_root || '') ? '已设置输出目录' : '尚未设置输出目录')
      : node.kind === 'map' ? `${((node.config.mappings as Mapping[] | undefined) ?? []).length} 个字段`
      : node.kind === 'filter' ? String((node.config.condition as Condition | undefined)?.field || '尚未设置条件') : node.kind;
    const incomplete = node.kind === 'source' && (!binding?.path || Boolean(inputErrors[node.id])) || node.kind === 'map' && !((node.config.mappings as Mapping[] | undefined)?.length)
      || node.kind === 'filter' && !(node.config.condition as Condition | undefined)?.field || node.kind === 'export' && !node.config.output_root;
    return {
      id: node.id, position: node.position, type: node.kind === 'source' ? 'source' : node.kind === 'export' ? 'export' : ['join', 'union'].includes(node.kind) ? 'multi' : 'step',
      selected: selectedId === node.id,
      data: { ports:node.kind==='union'&&node.version===2?node.config.ports:node.kind==='join'&&node.version===2?['left','right']:['left','in','right'], onOpen: node.kind === 'source' ? () => openSourceFromCanvas(node.id) : undefined, label: <><small>{toolCatalog.find(tool => tool.kind === node.kind)?.group ?? '旧流程操作'}</small><strong>{node.label}</strong><em>{subtitle}</em>{incomplete && <span className="node-warning">待配置</span>}</> },
    };
  }), [project.workflow.nodes, project.bindings, selectedId, inputErrors]);
  const flowEdges: Edge[] = useMemo(() => project.workflow.edges.map(edge => ({
    id: edge.id, source: edge.source, target: edge.target, sourceHandle: edge.source_port || 'main', targetHandle: edge.target_port || 'in',
    selected: selectedEdgeId === edge.id,
  })), [project.workflow.edges, selectedEdgeId]);

  function addTool(kind: Kind) {
    const previous = project.workflow.nodes.at(-1);
    const position = previous ? { x: previous.position.x + 250, y: previous.position.y } : { x: 120, y: 150 };
    const node = newNode(kind, position);
    editProject(current => applyStepCommand(current, { kind: 'add', node }));
    setSelectedId(node.id);
    setStatus(`已添加${node.label}；请配置并与前后步骤连线。`);
    requestAnimationFrame(() => { void flowApi?.fitView({ nodes: [{ id: node.id }], padding: 0.6, maxZoom: 1.1, duration: 250 }); });
    return node;
  }

  function connect(connection: Connection) {
    if (!connection.source || !connection.target) return;
    const target = project.workflow.nodes.find(node => node.id === connection.target);
    const source = project.workflow.nodes.find(node => node.id === connection.source);
    if (!target || !source || target.kind === 'source' || source.kind === 'export') {
      setStatus('这两个端口不能连接。'); return;
    }
    if ((target.kind === 'join' || target.kind === 'union') && target.version !== 2) {
      setStatus('旧版双输入节点可读取和运行；此轮尚不支持在新画布中修改其左右输入。'); return;
    }
    try {
      editProject(current => applyStepCommand(current, { kind: 'connect', edge: {
        id: 'edge_' + crypto.randomUUID().replaceAll('-', '').slice(0, 12), source: source.id, target: target.id, source_port: 'main',
        target_port:['join','union'].includes(target.kind)?connection.targetHandle||undefined:undefined,
      } }));
    } catch (error) {
      setStatus(errorText(error));
      return;
    }
    setStatus(`已连接 ${source.label} → ${target.label}。`);
  }

  function changeNodes(changes: NodeChange[]) {
    const active = changes.find(change => change.type === 'select' && change.selected);
    if (active?.type === 'select') setSelectedId(active.id);
    const persistent = changes.filter(change => change.type === 'remove' || (change.type === 'position' && change.dragging !== true));
    if (!persistent.length) return;
    const removed = new Set(persistent.filter(change => change.type === 'remove').map(change => change.id));
    editProject(previous => {
      const bindings = { ...previous.bindings };
      for (const id of removed) delete bindings[id];
      const nodes = previous.workflow.nodes.filter(node => !removed.has(node.id)).map(node => {
        const moved = persistent.find(change => change.type === 'position' && change.id === node.id);
        return moved?.type === 'position' && moved.position ? { ...node, position: moved.position } : node;
      });
      return { ...previous, bindings, workflow: { ...previous.workflow, nodes,
        edges: previous.workflow.edges.filter(edge => !removed.has(edge.source) && !removed.has(edge.target)) } };
    });
    if (selectedId && removed.has(selectedId)) setSelectedId(null);
    setPendingBindings(previous => Object.fromEntries(Object.entries(previous).filter(([id]) => !removed.has(id))));
    for (const id of removed) clearPreviews(id);
  }

  function changeEdges(changes: EdgeChange[]) {
    const removed = new Set(changes.filter(change => change.type === 'remove').map(change => change.id));
    if (!removed.size) return;
    editProject(previous => ({ ...previous, workflow: { ...previous.workflow,
      edges: previous.workflow.edges.filter(edge => !removed.has(edge.id)) } }));
  }

  function sourceStatus(nodeId: string, message: string) { if(selectedIdRef.current===nodeId)setStatus(message); }

  async function checkStructure(nodeId: string, binding: SourceBinding, requested?: TableLayout) {
    if (runSignatureRef.current) invalidate();
    setRunFields(previous=>{const next={...previous};delete next[nodeId];return next;});
    const revision = projectRevisionRef.current;
    const serial = (inspectSerialRef.current[nodeId] || 0) + 1;
    inspectSerialRef.current[nodeId] = serial;
    const draftSerial = draftSerialRef.current[nodeId] || 0;
    setSourceLoading(previous=>({...previous,[nodeId]:true}));
    const current = () => projectRevisionRef.current === revision && inspectSerialRef.current[nodeId] === serial && (draftSerialRef.current[nodeId] || 0) === draftSerial;
    setCheckedLayouts(previous => { const next = { ...previous }; delete next[nodeId]; return next; });
    try {
      const inspectedBinding = { ...binding, layout: requested ?? binding.layout, ...(requested ? {reading_baseline: null} : {}) };
      const check = inspectedBinding.layout ? await invoke<ReadingCheck>('check_source_reading', {binding: inspectedBinding, full: false}) : null;
      const report = check ? check.report : await invoke<StructureReport>('inspect_source_structure', { binding: inspectedBinding });
      if (!report) throw new Error(check?.diagnostics.map(d => d.message).join('；') || '无法读取资料');
      if (!current()) return;
      const candidate = report.active || report.candidates[0];
      setReadingDifferences(previous => ({...previous,[nodeId]: check?.diagnostics.filter(d => !candidate?.diagnostics.some(c => c.code === d.code && c.row === d.row && c.col === d.col)) || []}));
      setStructures(previous => ({ ...previous, [nodeId]: { ...report, active: candidate || null } }));
      setRawSamples(previous => ({ ...previous, [nodeId]: report.raw }));
      if (candidate) {
        setDrafts(previous => ({ ...previous, [nodeId]: candidate.layout }));
        setCheckedLayouts(previous => ({ ...previous, [nodeId]: JSON.stringify(candidate.layout) }));
      }

      if (projectRef.current.workflow.nodes.find(node => node.id === nodeId)?.config.normalize_structure === true && !requested && candidate?.ready && (!check || check.compatible) && binding.path === projectRef.current.bindings[nodeId]?.path && binding.sheet === projectRef.current.bindings[nodeId]?.sheet) {
        setSamples(previous => ({ ...previous, [nodeId]: candidate.preview }));
        setSampleKeys(previous=>({...previous,[nodeId]:sourceIdentity(projectRef.current,nodeId)}));
        setInputErrors(previous => { const next = { ...previous }; delete next[nodeId]; return next; });
      } else if (!requested) {
        try {
          if (binding.layout) throw new Error('请确认结构后应用；旧结构流程保持原样');
          const sample = await invoke<SourceSample>('inspect_source_file', { binding, limit: 20 });
          if (!current()) return;
          setSamples(previous => ({ ...previous, [nodeId]: sample }));
          setSampleKeys(previous=>({...previous,[nodeId]:sourceIdentity(projectRef.current,nodeId)}));
          setInputErrors(previous => { const next = { ...previous }; delete next[nodeId]; return next; });
        } catch {
          if (!current()) return;
          setSamples(previous => { const next = { ...previous }; delete next[nodeId]; return next; });
          setInputErrors(previous => ({ ...previous, [nodeId]: '表格结构待确认；请检查整理预览及疑点后应用。' }));
        }
      }
      if (current()) sourceStatus(nodeId,'结构建议仅依据局部原始行；请核实字段归属及明细范围，不会自动剔除合计或补值。');
    } catch (error) {
      if (!current()) return;
      if (!requested) setStructures(previous => { const next = { ...previous }; delete next[nodeId]; return next; });
      setInputErrors(previous => ({ ...previous, [nodeId]: errorText(error) }));
      sourceStatus(nodeId,`结构检查失败：${errorText(error)}`);
    } finally {
      if (inspectSerialRef.current[nodeId]===serial) setSourceLoading(previous=>({...previous,[nodeId]:false}));
    }
  }

  function applyStructure(nodeId: string) {
    const binding = pendingBindings[nodeId] || projectRef.current.bindings[nodeId];
    const draft = drafts[nodeId];
    const candidate = structures[nodeId]?.active;
    if (!binding || !draft || !candidate?.ready || readingDifferences[nodeId]?.length || checkedLayouts[nodeId] !== JSON.stringify(draft)) return;
    editProject(previous => ({ ...previous, bindings: { ...previous.bindings, [nodeId]: { ...binding, layout: draft, reading_baseline: candidate.baseline } }, workflow: { ...previous.workflow, nodes: previous.workflow.nodes.map(node => node.id === nodeId ? { ...node, config: { ...node.config, normalize_structure: true } } : node) } }));
    inspectSerialRef.current[nodeId] = (inspectSerialRef.current[nodeId] || 0) + 1;
    setSamples(previous => ({ ...previous, [nodeId]: candidate.preview }));
    setSampleKeys(previous=>({...previous,[nodeId]:sourceIdentity(projectRef.current,nodeId)}));
    setInputErrors(previous => { const next = { ...previous }; delete next[nodeId]; return next; });
    dirtyReadingRef.current.delete(nodeId);
    setPendingBindings(previous => {const next={...previous};delete next[nodeId];return next;});
     setPickRow(null);
    setStatus('已应用结构：全部选定列保留原值；下游字段及旧运行结果须重新检查。');
  }

  function chooseRawRow(row: number) {
    if (!selected || !pickRow) return;
    const d = drafts[selected.id]; if (!d) return;
    dirtyReadingRef.current.add(selected.id);
    invalidate();
    const start = pickRow === 'header_start' ? row : d.header_rows[0];
    const end = pickRow === 'header_end' ? row : Math.max(row, d.header_rows.at(-1) || row);
    draftSerialRef.current[selected.id] = (draftSerialRef.current[selected.id] || 0) + 1;
    setDrafts(previous => ({ ...previous, [selected.id]: pickRow === 'data_start' ? { ...d, data_start_row: row } : { ...d, header_rows: Array.from({length: Math.max(1, end - start + 1)}, (_, i) => start + i) } }));
    setPickRow(null); setStatus('已点选原始行，请检查整理预览后再应用。');
  }

  async function inspect(nodeId: string, binding: SourceBinding) {
    if (runSignatureRef.current) invalidate();
    setRunFields(previous=>{const next={...previous};delete next[nodeId];return next;});
    if (binding.path.toLowerCase().endsWith('.xlsx')) { await checkStructure(nodeId, binding); return; }
    if (!binding.path) return;
    const revision = projectRevisionRef.current;
    const serial = (inspectSerialRef.current[nodeId] || 0) + 1;
    setSourceLoading(previous=>({...previous,[nodeId]:true}));
    inspectSerialRef.current[nodeId] = serial;
    const current = () => projectRevisionRef.current === revision && inspectSerialRef.current[nodeId] === serial;
    try {
      const raw = await invoke<RawSourceSample>('inspect_raw_source_file', { binding, limit: 20 });
      if (!current()) return;
      setRawSamples(previous => ({ ...previous, [nodeId]: raw }));


      try {
        const sample = await invoke<SourceSample>('inspect_source_file', { binding, limit: 20 });
        if (!current()) return;
        setSamples(previous => ({ ...previous, [nodeId]: sample }));
        setSampleKeys(previous=>({...previous,[nodeId]:sourceIdentity(projectRef.current,nodeId)}));
        setInputErrors(previous => { const next = { ...previous }; delete next[nodeId]; return next; });
        sourceStatus(nodeId,`已读取 ${fileName(binding.path)} 的原始行和字段；这不是全量核对。`);
      } catch (error) {
        if (!current()) return;
        const message = `原始行已展示，但尚不能作为表处理：${errorText(error)}`;
        setSamples(previous => { const next = { ...previous }; delete next[nodeId]; return next; });
        setInputErrors(previous => ({ ...previous, [nodeId]: message }));
        sourceStatus(nodeId,message);
      }
    } catch (error) {
      if (!current()) return;
      const message = `无法读取原始行：${errorText(error)}`;
      clearPreviews(nodeId);
      setInputErrors(previous => ({ ...previous, [nodeId]: message }));
      sourceStatus(nodeId,message);
    } finally {
      if (inspectSerialRef.current[nodeId]===serial) setSourceLoading(previous=>({...previous,[nodeId]:false}));
    }
  }

  async function chooseSource(nodeId: string, reuse = false) {
    const revision = projectRevisionRef.current;
    try {
      const path = await open({ multiple: false, directory: false, filters: [{ name: '数据文件', extensions: ['csv', 'txt', 'tsv', 'xlsx'] }] });
      if (typeof path !== 'string' || projectRevisionRef.current !== revision) return;
      const names = await invoke<string[]>('list_source_sheets', { path });
      if (projectRevisionRef.current !== revision || !projectRef.current.workflow.nodes.some(node => node.id === nodeId)) return;
      const previousBinding = projectRef.current.bindings[nodeId];
      if (reuse && previousBinding?.layout && previousBinding.path.toLowerCase().endsWith('.xlsx') && path.toLowerCase().endsWith('.xlsx')) {
        const expectedSheet = previousBinding.sheet || previousBinding.reading_baseline?.sheet;
        const replacement = {...previousBinding, path, ...(expectedSheet ? {sheet:expectedSheet} : {})};
        clearPreviews(nodeId);
        invalidate();
        setPendingBindings(previous => ({...previous,[nodeId]:replacement}));
        setSheets(previous => ({...previous,[nodeId]:names}));
        if (expectedSheet && !names.includes(expectedSheet)) {
          setInputErrors(previous => ({...previous,[nodeId]:'原工作表不存在，请选择替代工作表后重新核实。'}));
          setStatus('原工作表不存在；新资料尚未替换已确认输入。'); return;
        }
        await checkStructure(nodeId,replacement); return;
      }
      if (reuse) {setStatus('沿用读取方案目前要求已确认区域的 XLSX；请使用浏览／更换重新识别。');return;}
      setPendingBindings(previous => {const next={...previous};delete next[nodeId];return next;});
      const extension = path.split('.').at(-1)?.toLowerCase();
      const old = projectRef.current.bindings[nodeId];
      const sameFormat = old?.path.split('.').at(-1)?.toLowerCase() === extension;
      const text: TextOptions | undefined = extension === 'xlsx' ? undefined : sameFormat && old?.text ? old.text : { encoding: 'utf-8', delimiter: extension === 'tsv' ? '\t' : ',', header_record: 1 };
      const sheet = sameFormat && old?.sheet && names.includes(old.sheet) ? old.sheet : names[0];
      const binding: SourceBinding = { path, fields: sameFormat ? old?.fields || {} : {}, ...(sheet ? { sheet } : {}), ...(text ? { text } : {}), ...(old?.path === path && old?.layout ? { layout: old.layout, reading_baseline: old.reading_baseline } : {}) };
      editProject(previous => ({ ...previous, bindings: { ...previous.bindings, [nodeId]: binding }, workflow: { ...previous.workflow, nodes: previous.workflow.nodes.map(node => node.id === nodeId ? { ...node, config: { ...node.config, normalize_structure: old?.path === path && Boolean(binding.layout) && node.config.normalize_structure === true } } : node) } }));
      clearPreviews(nodeId);
      setStatus(`正在读取 ${fileName(path)} 的原始行和字段…`);
      setSheets(previous => ({ ...previous, [nodeId]: names }));
      await inspect(nodeId, binding);
    } catch (error) { if (projectRevisionRef.current !== revision) return; const message = `选择文件失败：${errorText(error)}`; setInputErrors(previous => ({ ...previous, [nodeId]: message })); setStatus(message); }
  }

  function openSourceFromCanvas(nodeId: string) {
    setSelectedId(nodeId); setSelectedEdgeId(null);
    const binding = projectRef.current.bindings[nodeId];
    if (!binding?.path) { void chooseSource(nodeId); return; }
    setResultsOpen(true); setPreviewChoice({nodeId,view:'raw',category:'normal'});
    setStatus(`正在刷新 ${fileName(binding.path)} 的局部预览…`);
    void inspect(nodeId, binding);
  }

  async function locateReadingIssue(nodeId: string, issue: ReadingIssue) {
    const binding=pendingBindings[nodeId] || projectRef.current.bindings[nodeId];
    if(!binding) return;
    const revision=projectRevisionRef.current;
    const serial=(inspectSerialRef.current[nodeId]||0)+1;inspectSerialRef.current[nodeId]=serial;
    try {
      const raw=await invoke<RawSourceSample>('inspect_raw_source_window',{binding,startRow:Math.max(1,(issue.row||1)-3),startCol:Math.max(1,(issue.col||1)-3)});
      if(projectRevisionRef.current!==revision || inspectSerialRef.current[nodeId]!==serial)return;
      setRawSamples(previous=>({...previous,[nodeId]:raw}));setFocusCell({nodeId,row:issue.row,col:issue.col});
      if (selectedIdRef.current!==nodeId) return;
      setPreviewChoice({nodeId,view:'raw',category:'normal'});setResultsOpen(true);
      setStatus(`已定位原始资料${issue.row ? `第${issue.row}行` : ''}${issue.col ? `第${issue.col}列` : ''}；只展示附近局部，不改变读取范围。`);
    } catch(error){if(projectRevisionRef.current===revision && inspectSerialRef.current[nodeId]===serial)setStatus(`定位失败：${errorText(error)}`);}
  }

  async function viewSourceWindow(nodeId:string,row:number,column:number,tail:boolean){
    const binding=pendingBindings[nodeId]||projectRef.current.bindings[nodeId];
    if(!binding)return;
    const revision=projectRevisionRef.current;
    const serial=(inspectSerialRef.current[nodeId]||0)+1;inspectSerialRef.current[nodeId]=serial;
    setSourceLoading(previous=>({...previous,[nodeId]:true}));
    setRawSamples(previous=>{const next={...previous};delete next[nodeId];return next;});
    setPreviewChoice({nodeId,view:'raw',category:'normal'});setResultsOpen(true);
    setStatus(tail?'正在扫描资料尾部，局部展示不代表扫描成本恒定。':'正在读取指定位置的完整记录。');
    try{
      const raw=await invoke<RawSourceSample>('inspect_raw_source_window',{binding,startRow:row,startCol:column,tail});
      if(projectRevisionRef.current!==revision||inspectSerialRef.current[nodeId]!==serial)return;
      setRawSamples(previous=>({...previous,[nodeId]:raw}));setFocusCell(null);
      setStatus('已展示原始资料局部窗口。文本按完整记录显示，物理行号保留；读取范围未改变。');
    }catch(error){
      if(projectRevisionRef.current===revision&&inspectSerialRef.current[nodeId]===serial)setStatus(`局部查看失败：${errorText(error)}`);
    }finally{
      if(projectRevisionRef.current===revision&&inspectSerialRef.current[nodeId]===serial)setSourceLoading(previous=>({...previous,[nodeId]:false}));
    }
  }

  function discardReplacement(nodeId: string) {
    setPendingBindings(previous=>{const next={...previous};delete next[nodeId];return next;});
    clearPreviews(nodeId);setPickRow(null);
    const binding=projectRef.current.bindings[nodeId];
    if(binding) void inspect(nodeId,binding);
    setStatus('已放弃待确认的读取修改，恢复项目中原有的文件与规则。');
  }

  function hasUnappliedReading() {
    return projectRef.current.workflow.nodes.some(node=>Boolean(configDraftRef.current[node.id])) || dirtyReadingRef.current.size>0 || Object.keys(pendingBindings).length>0 || projectRef.current.workflow.nodes.some(n=>n.config.normalize_structure===true && drafts[n.id] && JSON.stringify(drafts[n.id])!==JSON.stringify(projectRef.current.bindings[n.id]?.layout));
  }

  function guardAction(action:()=>void|Promise<void>) {
    if(hasUnsavedProject(projectRef.current,savedSnapshotRef.current,hasUnappliedReading())) {guardedActionRef.current=action;setGuardOpen(true);}
    else void action();
  }
  async function guardChoice(choice:'save'|'discard'|'cancel') {
    if(choice==='cancel'){setGuardOpen(false);guardedActionRef.current=null;return;}
    if(choice==='save'&&hasUnappliedReading()){
      setGuardOpen(false);guardedActionRef.current=null;setStatus('请先应用或放弃步骤草稿和读取方案，再保存。');return;
    }
    setGuardBusy(true);
    try {
      if(choice==='save' && !await saveCurrent())return;
      const action=guardedActionRef.current;guardedActionRef.current=null;setGuardOpen(false);await action?.();
    } finally {setGuardBusy(false);}
  }
  closeHandlerRef.current=()=>guardAction(async()=>{await invoke('confirm_project_close');});
  useEffect(()=>{
    let unlisten:(()=>void)|undefined;let disposed=false;
    void listen('project-close-requested',()=>closeHandlerRef.current()).then(stop=>{if(disposed)stop();else unlisten=stop;});
    return ()=>{disposed=true;unlisten?.();};
  },[]);

  useEffect(()=>{
    if(!selectedId || !drafts[selectedId] || checkedLayouts[selectedId]===JSON.stringify(drafts[selectedId]))return;
    const binding=pendingBindings[selectedId] || projectRef.current.bindings[selectedId];
    if(!binding?.path.toLowerCase().endsWith('.xlsx'))return;
    const timer=window.setTimeout(()=>{void checkStructure(selectedId,binding,drafts[selectedId]);},600);
    return ()=>window.clearTimeout(timer);
  },[drafts,selectedId,pendingBindings]);

  async function refreshSheets(nodeId: string, path: string) {
    const revision = projectRevisionRef.current;
    try {
      const names = await invoke<string[]>('list_source_sheets', { path });
      if (projectRevisionRef.current !== revision || projectRef.current.bindings[nodeId]?.path !== path) return;
      setSheets(previous => ({ ...previous, [nodeId]: names }));
      if (!names.length) setStatus('该资料没有可选择的工作表。');
    } catch (error) { if (projectRevisionRef.current !== revision) return; const message = `读取工作表失败：${errorText(error)}`; setInputErrors(previous => ({ ...previous, [nodeId]: message })); setStatus(message); }
  }

  async function chooseOutput(nodeId: string) {
    const revision = projectRevisionRef.current;
    try {
      const path = await open({ multiple: false, directory: true });
      if (typeof path === 'string' && projectRevisionRef.current === revision && projectRef.current.workflow.nodes.some(node => node.id === nodeId)) stageConfig(nodeId, { output_root: path });
    } catch (error) { if (projectRevisionRef.current === revision) setStatus(`选择输出目录失败：${errorText(error)}`); }
  }

  async function saveCurrent():Promise<boolean> {
    if(hasUnappliedReading()){setStatus('请先应用或放弃步骤草稿和读取方案，再保存项目。');return false;}
    try {
      const path = projectPath || await save({ defaultPath: '审计数工坊项目.json', filters: [{ name: '处理项目', extensions: ['json'] }] });
      if (typeof path !== 'string') return false;
      const revision = projectRevisionRef.current;
      const snapshot=projectSnapshot(projectRef.current);
      await invoke('save_project', { path, project: projectRef.current });
      if (projectRevisionRef.current !== revision) {setStatus('已保存当时版本；当前仍有新更改，请再次保存。');return false;}
      setProjectPath(path);
      savedSnapshotRef.current=snapshot;
      setStatus(`项目已保存：${path}`);
      return true;
    } catch (error) { setStatus(`保存失败，当前工作仍保留：${errorText(error)}`);return false; }
  }

  async function openCurrent() {
    const revision = projectRevisionRef.current;
    try {
      const path = await open({ multiple: false, directory: false, filters: [{ name: '处理项目', extensions: ['json'] }] });
      if (typeof path !== 'string' || projectRevisionRef.current !== revision) return;
      const loaded = await invoke<ProjectFile>('load_project', { path });
      if (projectRevisionRef.current !== revision) return;
      replaceProject(loaded); savedSnapshotRef.current=projectSnapshot(loaded); setProjectPath(path); setSelectedId(null); setSelectedEdgeId(null); dirtyReadingRef.current.clear(); setPendingBindings({}); setReadingDifferences({}); setFocusCell(null); setStructures({}); setDrafts({}); setCheckedLayouts({}); setPickRow(null); setSamples({}); setRawSamples({}); setInputErrors({}); setSheets({}); setResultsOpen(false); invalidate();
      setStatus(`已打开处理项目：${path}。运行前请核实本地输入文件。`);
    } catch (error) { if (projectRevisionRef.current === revision) setStatus(`打开失败：${errorText(error)}`); }
  }

  async function importLegacy() {
    const revision = projectRevisionRef.current;
    try {
      const path = await open({ multiple: false, directory: true });
      if (typeof path !== 'string' || projectRevisionRef.current !== revision) return;
      const loaded = await invoke<ProjectFile>('import_legacy_project', { path });
      if (projectRevisionRef.current !== revision) return;
      replaceProject(loaded); savedSnapshotRef.current="imported-unsaved"; setProjectPath(''); setSelectedId(null); setSelectedEdgeId(null); dirtyReadingRef.current.clear(); setPendingBindings({}); setReadingDifferences({}); setFocusCell(null); setStructures({}); setDrafts({}); setCheckedLayouts({}); setPickRow(null); setSamples({}); setRawSamples({}); setInputErrors({}); setSheets({}); setResultsOpen(false); invalidate();
      setStatus('已导入旧模板及公司绑定；请检查文件路径，并在导出节点选择输出目录后另存新项目。');
    } catch (error) { if (projectRevisionRef.current === revision) setStatus(`导入旧模板失败：${errorText(error)}`); }
  }

  async function validate() {
    const revision = projectRevisionRef.current;
    try {
      const order = await invoke<string[]>('validate_project', { project });
      if (projectRevisionRef.current !== revision) return;
      setStatus(`校验通过：${order.length} 个步骤，输入资料及处理规则有效。`);
    } catch (error) { if (projectRevisionRef.current === revision) setStatus(`校验未通过：${errorText(error)}`); }
  }

  function showReview(_result: RunResult, item: Review, mode: 'normal' | 'excluded' | 'exceptions' = 'normal') {
    setSelectedId(item.node_id); setPreviewChoice({nodeId:item.node_id,view:'output',category:mode}); setResultsOpen(true);
  }

  async function runCurrent(nextProject:ProjectFile=projectRef.current,nodeId:string|null=selectedIdRef.current):Promise<RunResult|null> {
    if(busy){setStatus('当前仍在处理，请等待完成。');return null;}
    if(!nodeId){setStatus('请先选择要计算的步骤。');return null;}
    if(hasUnappliedReading()){setStatus('请先应用或放弃步骤草稿和读取方案，再计算。');return null;}
    invalidate();
    const runSession=sessionRef.current;const runRevision=dataRevisionRef.current;
    const needed=new Set<string>();const pending=[nodeId];
    while(pending.length){const id=pending.pop()!;if(needed.has(id))continue;needed.add(id);for(const e of nextProject.workflow.edges)if(e.target===id)pending.push(e.source);}
    setBusy(true);setStatus('正在计算当前步骤及其上游…');
    try {
      for(const node of nextProject.workflow.nodes.filter(n=>needed.has(n.id)&&n.kind==='source'&&n.config.normalize_structure===true)) {
        const binding=nextProject.bindings[node.id];if(!binding?.path.toLowerCase().endsWith('.xlsx'))continue;
        const check=await invoke<ReadingCheck>('check_source_reading',{binding,full:true});
        if(sessionRef.current!==runSession || dataRevisionRef.current!==runRevision)return null;
        if(!check.compatible){
          setReadingDifferences(previous=>({...previous,[node.id]:check.diagnostics.filter(issue=>!check.report?.active?.diagnostics.some(local=>local.code===issue.code&&local.row===issue.row&&local.col===issue.col))}));
          if(check.report){setStructures(previous=>({...previous,[node.id]:check.report!}));if(check.report.active){setDrafts(previous=>({...previous,[node.id]:check.report!.active!.layout}));setCheckedLayouts(previous=>({...previous,[node.id]:JSON.stringify(check.report!.active!.layout)}));}}
          setSelectedId(node.id);setInputErrors(previous=>({...previous,[node.id]:'读取检查未通过，请处理定位到的疑点。'}));
          if(check.diagnostics[0])await locateReadingIssue(node.id,check.diagnostics[0]);
          setStatus('全量读取检查未通过，尚未执行处理链。');return null;
        }
      }
      const result=await invoke<RunResult>('process_project',{project:nextProject,nodeId,revision:runRevision});
      if(sessionRef.current!==runSession || dataRevisionRef.current!==runRevision)return null;
      const schemas=await Promise.all(result.reviews.filter(item=>nextProject.workflow.nodes.some(n=>n.id===item.node_id&&n.kind==='source')).map(async item=>{
        const table=await invoke<Preview>('preview_table',{batch:result.run_dir,nodeId:item.node_id,category:'normal',limit:0});
        return [item.node_id,{key:sourceIdentity(nextProject,item.node_id),headers:table.columns.filter(column=>!column.startsWith('__'))}] as const;
      }));
      if(sessionRef.current!==runSession || dataRevisionRef.current!==runRevision)return null;
      runSignatureRef.current=dataSignature(projectRef.current);setRunFields(Object.fromEntries(schemas));setRun(result);setResultsOpen(true);
      if(selectedIdRef.current===nodeId)setPreviewChoice({nodeId,view:'output',category:'normal'});
      setStatus('当前步骤计算完成；可继续在表格上操作，或直接导出。');return result;
    } catch(error){if(sessionRef.current===runSession && dataRevisionRef.current===runRevision)setStatus(`计算失败：${errorText(error)}`);return null;}
    finally{setBusy(false);}
  }

  const tableDownstream = project.workflow.edges.filter(edge => edge.source === selected?.id && (!edge.source_port || edge.source_port === 'main'));
  const tableInsertEdge = tableDownstream.find(edge => edge.id === tableInsertEdgeId);

  async function recordTableAction(action:TableAction):Promise<boolean> {
    if(!selected || !visiblePreview.data || !canTableAction){setStatus('请先确认读取，或计算并查看本步输出。');return false;}
    try {
      const next=applyTableAction(projectRef.current,selected.id,action,visiblePreview.data.columns,tableInsertEdge?.id);
      setTableInsertEdgeId('');
      editProject(()=>next.project);setSelectedId(next.node.id);setPreviewChoice({nodeId:next.node.id,view:'output',category:'normal'});
      return Boolean(await runCurrent(next.project,next.node.id));
    } catch(error){setStatus(`操作未应用：${errorText(error)}`);return false;}
  }

  async function exportCurrent() {
    if(!selectedIdRef.current || busy)return;
    if(hasUnappliedReading()){setStatus('请先应用或放弃步骤草稿和读取方案，再导出。');return;}
    const exportId=visiblePreview.direction==='input'?visiblePreview.sourceId || selectedIdRef.current:selectedIdRef.current;
    const category=visiblePreview.category || activeQuery.category;
    const session=sessionRef.current;const revision=dataRevisionRef.current;
    const label=projectRef.current.workflow.nodes.find(n=>n.id===exportId)?.label || '处理结果';
    try {
      const path=await save({defaultPath:`${label.replace(/[\\/:*?"<>|]/g,'_')}_${category==='normal'?'结果':category==='excluded'?'未纳入':'异常'}.csv`,filters:[{name:'CSV数据',extensions:['csv']}]});
      if(typeof path!=='string')return;
      if(sessionRef.current!==session || dataRevisionRef.current!==revision){setStatus('处理规则已变化，请重新发起导出。');return;}
      let result=validRun;
      if(!result?.reviews.some(r=>r.node_id===exportId))result=await runCurrent(projectRef.current,exportId);
      if(!result || sessionRef.current!==session || dataRevisionRef.current!==revision)return;
      setBusy(true);
      const receipt=await invoke<{path:string;rows:number}>('export_node',{batch:result.run_dir,nodeId:exportId,category,path});
      if(sessionRef.current===session && dataRevisionRef.current===revision)setStatus(`已导出全量 ${receipt.rows} 行：${receipt.path}`);
    } catch(error){setStatus(`导出失败：${errorText(error)}`);}finally{setBusy(false);}
  }

  async function legacyExport() {
    if(busy || hasUnappliedReading())return;
    invalidate();
    setBusy(true);const session=sessionRef.current;const revision=dataRevisionRef.current;
    try {const result=await invoke<RunResult>('run_project',{project:projectRef.current,revision});if(sessionRef.current===session&&dataRevisionRef.current===revision){runSignatureRef.current=dataSignature(projectRef.current);setRun(result);setStatus(`已按导出节点规则生成批次：${result.run_dir}`);}}catch(error){setStatus(`导出失败：${errorText(error)}`);}finally{setBusy(false);}
  }

  function newCurrent() {
    const next=blankProject();replaceProject(next);savedSnapshotRef.current=projectSnapshot(next);setProjectPath('');setSelectedId(null);setSelectedEdgeId(null);
    dirtyReadingRef.current.clear();setPendingBindings({});setReadingDifferences({});setFocusCell(null);setStructures({});setDrafts({});setCheckedLayouts({});setPickRow(null);setSamples({});setRawSamples({});setInputErrors({});setSheets({});setResultsOpen(false);setStatus('已新建空白项目。');
  }

  const mappingSources = [...fields.filter(field => !field.startsWith('__') && field !== 'company'),
    ...mapping.filter(item=>item.source.startsWith('__')&&inputSchema?.some(field=>field.name===item.source)).map(item=>item.source)];
  const selectedBinding = selected?.kind === 'source' ? pendingBindings[selected.id] || project.bindings[selected.id] : undefined;
  const structure = selected ? structures[selected.id] : undefined;
  const draft = selected ? drafts[selected.id] : undefined;
  const checked = selected && draft ? checkedLayouts[selected.id] === JSON.stringify(draft) : false;
  const updateDraft = (change: Partial<TableLayout>) => { if (selected && draft) { invalidate(); dirtyReadingRef.current.add(selected.id); draftSerialRef.current[selected.id] = (draftSerialRef.current[selected.id] || 0) + 1; setDrafts(previous => ({ ...previous, [selected.id]: { ...draft, ...change } })); } };

  const activeQuery = useMemo<PreviewQuery>(() => previewChoice?.nodeId===selectedId ? previewChoice : {nodeId:selectedId || '',view:'auto',category:'normal'},[selectedId,previewChoice]);
  const validRun = run && runSignatureRef.current===dataSignature(project) && !hasUnappliedReading() ? run : null;
  const [resultPage, setResultPage] = useState({ key: '', offset: 0 });
  const resultPageKey = JSON.stringify([validRun?.run_dir, activeQuery]);
  const resultOffset = resultPage.key === resultPageKey ? resultPage.offset : 0;
  const previewContext = useMemo(() => {
    const sources:Record<string,SourceEvidence>={};
    for(const node of project.workflow.nodes.filter(n=>n.kind==='source')) {
      const id=node.id;
      const readingApplied=!project.bindings[id]?.path.toLowerCase().endsWith('.xlsx') || node.config.normalize_structure===true && JSON.stringify(project.bindings[id]?.layout)===JSON.stringify(drafts[id]);
      const confirmed=readingApplied && !pendingBindings[id] && !dirtyReadingRef.current.has(id) && !inputErrors[id]
        && sampleKeys[id]===sourceIdentity(project,id);
      sources[id]={raw:rawSamples[id],prepared:structures[id]?.active?.preview,
        preparedFresh:Boolean(drafts[id] && checkedLayouts[id]===JSON.stringify(drafts[id])),
        applied:samples[id],appliedKey:sampleKeys[id],confirmed,loading:sourceLoading[id],error:inputErrors[id]};
    }
    return {project,sources,run:validRun};
  },[project,samples,sampleKeys,sourceLoading,rawSamples,structures,drafts,checkedLayouts,pendingBindings,inputErrors,validRun]);
  useEffect(()=>{setResultsOpen(Boolean(selectedId));},[selectedId]);
  useEffect(()=>{
    if(!selectedId) {previewTargetRef.current=null;setNodePreview(null);return;}
    const target:PreviewTarget={...activeQuery,session:sessionRef.current,revision:dataRevisionRef.current,request:++previewRequestRef.current,offset:resultOffset};
    previewTargetRef.current=target;
    const plan=resolvePreview(previewContext,activeQuery);
    if(plan.status!=='table') {setNodePreview({...plan,target});return;}
    setNodePreview({...plan,status:'loading',message:'正在载入当前步骤的运行结果…',target});
    const current=()=>previewTargetRef.current && acceptsResponse(previewTargetRef.current,target)
      && sessionRef.current===target.session && dataRevisionRef.current===target.revision && selectedIdRef.current===target.nodeId;
    void invoke<Preview>('preview_table',{batch:plan.batch,nodeId:plan.sourceId,category:plan.category,limit:200,offset:resultOffset}).then(data=>{
      if(!current())return;
      setNodePreview({...plan,status:'ready',target,data:{kind:'run',columns:data.columns,rows:data.rows,totalRows:data.total_rows}});
    }).catch(error=>{
      if(current())setNodePreview({...plan,status:'error',target,message:`当前步骤预览失败：${errorText(error)}`});
    });
    return ()=>{if(previewTargetRef.current && acceptsResponse(previewTargetRef.current,target))previewTargetRef.current=null;};
  },[previewContext,activeQuery,selectedId,resultOffset]);
  const visiblePreview:PreviewState = nodePreview && previewTargetRef.current && acceptsResponse(nodePreview.target,previewTargetRef.current)
    && nodePreview.target.nodeId===selectedId && nodePreview.target.session===sessionRef.current && nodePreview.target.revision===dataRevisionRef.current
    && nodePreview.target.view===activeQuery.view && nodePreview.target.category===activeQuery.category && nodePreview.target.inputEdgeId===activeQuery.inputEdgeId && nodePreview.target.offset===resultOffset
    ? nodePreview : {target:{...activeQuery,session:sessionRef.current,revision:dataRevisionRef.current,request:previewRequestRef.current},
      status:'loading',title:`${selected?.label || '步骤'} · 预览`,direction:'output',message:'正在准备当前步骤预览…'};
  const canTableAction=Boolean(!busy && !hasUnappliedReading() && selected?.kind!=='export' && visiblePreview.status==='ready' && visiblePreview.direction==='output' && visiblePreview.data?.kind!=='raw'
    && (selected?.kind!=='source' || previewContext.sources[selected.id]?.confirmed || visiblePreview.data?.kind==='run') && (visiblePreview.category || activeQuery.category)==='normal');


  return <div className="app-shell">
    <header className="app-header">
      <div className="app-title"><span className="app-mark">数</span><strong>审计数工坊</strong><span className="project-name">{project.workflow.name}</span></div>
      <div className="work-view-picker" aria-label="工作台视图">
        {([['table','表格'],['canvas','画布'],['split','分屏']] as const).map(([id,label]) => <button key={id} aria-pressed={workView === id} onClick={()=>{
          setWorkView(id);
          if (id !== 'canvas' && selectedId) setResultsOpen(true);
        }}>{label}</button>)}
        {workView === 'table' && <label>当前步骤 <select aria-label="表格当前步骤" value={selectedId ?? ''} onChange={event=>{
          setSelectedId(event.target.value || null);
          setSelectedEdgeId(null);
        }}><option value="">选择步骤</option>{project.workflow.nodes.map(node=><option key={node.id} value={node.id}>{node.label}</option>)}</select></label>}
      </div>
      <div className="ribbon">
        <nav>{['输入', '整理', '行处理', '多表', '统计', '输出'].map(name => <button key={name} className={group === name ? 'active' : ''} onClick={() => { setGroup(name); setToolOpen(true); }}>{name}</button>)}</nav>
        <div className="ribbon-actions"><button onClick={()=>guardAction(newCurrent)}>新建</button><button onClick={()=>guardAction(openCurrent)}>打开</button><button onClick={()=>guardAction(importLegacy)}>导入旧模板</button><button onClick={()=>{void saveCurrent();}}>保存</button><button onClick={validate}>校验</button><button className="primary" disabled={busy} onClick={()=>{void runCurrent();}}>{busy?'处理中…':'计算当前步骤'}</button></div>
      </div>
    </header>
    <div ref={workAreaRef} className={`work-area view-${workView} ${showResults && workView==='split' ? 'with-results' : ''} ${selected ? 'with-inspector' : ''} ${toolOpen ? '' : 'tools-closed'} ${resizingResults?'resizing-results':''}`}
      style={workView==='split'&&showResults&&workAreaHeight>0?{gridTemplateRows:`minmax(0, 1fr) 8px ${resultsHeight}px`}:undefined}>
      {toolOpen ? <aside className="tool-shelf"><div className="panel-head"><strong>工具库</strong><button title="收起工具库" onClick={() => setToolOpen(false)}>‹</button></div><input aria-label="搜索工具" placeholder="搜索工具…" value={search} onChange={event => setSearch(event.target.value)} /><div className="tool-list">{toolCatalog.filter(tool => search ? `${tool.label}${tool.hint}`.includes(search) : tool.group === group).map(tool => <button key={tool.kind} onClick={() => addTool(tool.kind)}><b>{tool.label}</b><small>{tool.hint}</small><span>＋</span></button>)}</div><p>工具分类只用于查找，不规定处理顺序。</p></aside> : <button className="open-tools" onClick={() => setToolOpen(true)}>工具库 ›</button>}
      {workView==='table'&&!showResults&&<p className="table-view-empty">从工具库添加步骤，或在上方选择已有步骤查看表格。{selectedId&&<button onClick={()=>setResultsOpen(true)}>显示表格</button>}</p>}
      <main className="canvas-area"><div className="canvas-heading"><span>处理画布 <small>{project.workflow.nodes.length} 个操作 · {project.workflow.edges.length} 条连接</small></span><div className="canvas-actions"><button disabled={!pastRef.current.length} onClick={() => restoreProject('undo')} title="Ctrl+Z">撤销</button><button disabled={!futureRef.current.length} onClick={() => restoreProject('redo')} title="Ctrl+Y">重做</button><button disabled={!selectedId && !selectedEdgeId} onClick={deleteSelection} title="选中后按 Delete">{selectedEdgeId ? '断开连接' : '删除步骤'}</button><button onClick={() => { void flowApi?.fitView({ padding: 0.2, duration: 250 }); }}>总览</button></div></div>
        <div className="flow-wrap"><ReactFlow nodes={flowNodes} edges={flowEdges} nodeTypes={nodeTypes} deleteKeyCode={null} onInit={setFlowApi} onNodesChange={changeNodes} onEdgesChange={changeEdges} onConnect={connect} onNodeClick={(_, node) => { setSelectedId(node.id); setSelectedEdgeId(null); }} onNodeDoubleClick={(_, node) => { if (node.type === 'source') openSourceFromCanvas(node.id); }} onEdgeClick={(_, edge) => { setSelectedEdgeId(edge.id); setSelectedId(null); }} onPaneClick={() => { setSelectedId(null); setSelectedEdgeId(null); }} fitView fitViewOptions={{ padding: 0.25 }}><Background gap={20} color="#dfe4ed" /><Controls /></ReactFlow>
          {!project.workflow.nodes.length && <div className="empty-canvas"><span>＋</span><strong>选择文件开始</strong><small>或从工具库添加操作</small><button onClick={() => { const node = addTool('source'); void chooseSource(node.id); }}>选择文件开始</button></div>}
        </div></main>
      {selected && <aside className="inspector"><div className="panel-head"><strong>{selected.label}</strong><button title="关闭配置" onClick={() => setSelectedId(null)}>×</button></div><div className="inspector-body">
        {selected.kind === 'source' && <>
          <p>先确认表格，再按需添加处理；无需逐列配置。资料只在本机处理。</p>
          <label>本地文件</label>
          <div className="input-action"><input readOnly value={selectedBinding?.path || ''} placeholder="尚未选择 CSV／TXT／TSV／XLSX" /><button onClick={() => chooseSource(selected.id)}>浏览／更换</button></div>
          {project.bindings[selected.id]?.layout && <button className="secondary-action" onClick={()=>{void chooseSource(selected.id,true);}}>更换文件并沿用读取方案</button>}
          {selectedBinding?.path && <button className="text-action" onClick={() => { setPendingBindings(previous=>{const next={...previous};delete next[selected.id];return next;}); editBinding(selected.id, { path: '', fields: {} }); clearPreviews(selected.id); setSheets(previous => { const next = { ...previous }; delete next[selected.id]; return next; }); setStatus('已清除本次文件绑定；输入步骤仍在画布上，原始文件未删除。'); }}>清除文件选择</button>}
          {selectedBinding?.path.toLowerCase().endsWith('.xlsx') && <button className="secondary-action" onClick={() => { if (selectedBinding) void refreshSheets(selected.id, selectedBinding.path); }}>选择工作表</button>}
          {Boolean(sheets[selected.id]?.length) && <><label>工作表</label><select value={selectedBinding?.sheet || ''} onChange={event => { const next = { ...(selectedBinding || { path: '', fields: {} }), sheet: event.target.value }; clearPreviews(selected.id); if(pendingBindings[selected.id]) {setPendingBindings(previous=>({...previous,[selected.id]:next})); void checkStructure(selected.id,next);} else {const reset={...next,layout:undefined,reading_baseline:null}; editNode(selected.id, node => ({...node, config: {...node.config, normalize_structure: false}})); editBinding(selected.id,reset);void inspect(selected.id,reset);} }}><option value="">请选择工作表</option>{sheets[selected.id].map(name => <option key={name} value={name}>{name}</option>)}</select></>}
          {selectedBinding?.path && !selectedBinding.path.toLowerCase().endsWith('.xlsx') && <>
            <label>文字编码</label><select value={selectedBinding.text?.encoding || 'utf-8'} onChange={event => { const next = { ...selectedBinding, text: { ...(selectedBinding.text || { encoding: 'utf-8', delimiter: ',', header_record: 1 }), encoding: event.target.value as TextOptions['encoding'] } }; editBinding(selected.id, next); clearPreviews(selected.id); void inspect(selected.id, next); }}><option value="utf-8">UTF-8</option><option value="gb18030">GB18030（常见中文导出）</option></select>
            <label>分隔符</label><select value={selectedBinding.text?.delimiter || (selectedBinding.path.toLowerCase().endsWith('.tsv') ? '\t' : ',')} onChange={event => { const next = { ...selectedBinding, text: { ...(selectedBinding.text || { encoding: 'utf-8', delimiter: ',', header_record: 1 }), delimiter: event.target.value as TextOptions['delimiter'] } }; editBinding(selected.id, next); clearPreviews(selected.id); void inspect(selected.id, next); }}><option value=",">逗号</option><option value={'\t'}>Tab</option><option value=";">分号</option><option value="|">竖线</option></select>
            <label>第几条记录是表头</label><input type="number" min="1" value={selectedBinding.text?.header_record || 1} onChange={event => { const next = { ...selectedBinding, text: { ...(selectedBinding.text || { encoding: 'utf-8', delimiter: ',', header_record: 1 }), header_record: Number(event.target.value) } }; editBinding(selected.id, next); clearPreviews(selected.id); }} onBlur={() => { const current = projectRef.current.bindings[selected.id]; if (current?.path) void inspect(selected.id, current); }} />
          </>}
          {structure && !draft && <button onClick={()=>setDrafts(previous=>({...previous,[selected.id]:{first_col:1,last_col:Math.max(1,structure.raw.columns.length),header_rows:[1],data_start_row:2,fill_merged_data_cols:[],strict_columns:true}}))}>手动选择读取区域</button>}
          {structure && <SourceReadingPanel key={selected.id} report={structure} draft={draft} checked={Boolean(checked)} applied={!pendingBindings[selected.id] && selected.config.normalize_structure === true && JSON.stringify(project.bindings[selected.id]?.layout) === JSON.stringify(draft)} pending={Boolean(pendingBindings[selected.id])} differences={readingDifferences[selected.id] || []} previewView={activeQuery.view} update={updateDraft} select={index => {
            const item=structure.candidates[index]; invalidate(); dirtyReadingRef.current.add(selected.id); draftSerialRef.current[selected.id]=(draftSerialRef.current[selected.id]||0)+1;
            setDrafts(previous=>({...previous,[selected.id]:item.layout}));setReadingDifferences(previous=>({...previous,[selected.id]:[]}));
            setStructures(previous=>({...previous,[selected.id]:{...structure,active:item}}));setCheckedLayouts(previous=>({...previous,[selected.id]:JSON.stringify(item.layout)}));
          }} locate={issue => {void locateReadingIssue(selected.id,issue);}} view={value=>{setPreviewChoice({nodeId:selected.id,view:value==='raw'?'raw':'prepared',category:'normal'});setResultsOpen(true);}} pick={mode=>{setPickRow(mode);setPreviewChoice({nodeId:selected.id,view:'raw',category:'normal'});setResultsOpen(true);setStatus('请点击原始表格对应行。');}} check={()=>{if(selectedBinding && draft) void checkStructure(selected.id,selectedBinding,draft);}} apply={()=>applyStructure(selected.id)} discard={()=>discardReplacement(selected.id)} />}
          {!structure && selectedBinding?.path.toLowerCase().endsWith('.xlsx') && pendingBindings[selected.id] && <button onClick={()=>discardReplacement(selected.id)}>放弃替换，返回原资料</button>}
          <button className="secondary-action" disabled={!selectedBinding?.path} onClick={() => { if (selectedBinding) void inspect(selected.id, selectedBinding); }}>读取／刷新表格</button>
          {rawSamples[selected.id] && <p>已展示原始 {rawSamples[selected.id].rows.length} 行；表头确认后可在下游选择字段。预览不代表全量。</p>}
          {inputErrors[selected.id] && <div className="inline-error" role="alert">{inputErrors[selected.id]}</div>}
        </>}
        {modernMap && <><p>从上一步实际字段中选择需要保留的列，并指定输出列名。原始位置列自动保留。</p>{!input && <div className="inline-error">请先连接上游节点。</div>}{input && !mappingSources.length && <div className="inline-error">尚未取得上游字段结构，请在文件输入刷新，或运行后查看。</div>}{mappingSources.length > 0 && mapping.some(item => !mappingSources.includes(item.source)) && <div className="inline-error">新文件缺少原映射字段：{mapping.filter(item => !mappingSources.includes(item.source)).map(item => item.source).join('、')}。请修正规则后再运行。</div>}{mappingSources.length > 0 && <button className="secondary-action" onClick={() => stageConfig(selected.id, { mappings: mappingSources.map(source => mapping.find(item => item.source === source) || {source, target: source}) })}>保留全部列（沿用已有更名）</button>}{mappingSources.length > 0 && <div className="mapping-table"><div className="mapping-head"><span>保留</span><span>输入字段</span><span>输出字段名</span></div>{mappingSources.map(source => { const current = mapping.find(item => item.source === source); return <div className="mapping-row" key={source}><input type="checkbox" checked={Boolean(current)} onChange={event => { const next = event.target.checked ? [...mapping, { source, target: source }] : mapping.filter(item => item.source !== source); stageConfig(selected.id, { mappings: next }); }} /><span title={source}>{source}</span><input disabled={!current} value={current?.target || ''} onChange={event => stageConfig(selected.id, { mappings: mapping.map(item => item.source === source ? { ...item, target: event.target.value } : item) })} aria-label={`${source} 的输出字段名`} /></div>; })}</div>}<p>本步骤只选列和重命名，不自动转换金额或日期类型。</p></>}
        {(modernMap||modernFilter||selected.kind==='export')&&<div className="input-action">
          <button disabled={busy||!configDrafts[selected.id]} onClick={()=>applyConfigDraft(selected.id)}>应用规则</button>
          <button disabled={busy||!configDrafts[selected.id]} onClick={()=>discardConfig(selected.id)}>放弃草稿</button>
          {configDrafts[selected.id]&&<span>此步骤有待确认草稿。</span>}
        </div>}
        {Object.keys(configDrafts).filter(id=>project.workflow.nodes.some(node=>node.id===id)).length>0&&<div>
          <p>待确认步骤草稿</p>{project.workflow.nodes.filter(node=>configDrafts[node.id]).map(node=><button key={node.id} onClick={()=>setSelectedId(node.id)}>{node.label}</button>)}
        </div>}
        {modernFilter && condition && <><p>符合条件的行进入主结果；其他行保留在“未纳入”结果中。</p><label>筛选字段</label><select value={condition.field} onChange={event => stageConfig(selected.id, { condition: { ...condition, field: event.target.value } })}><option value="">选择上游字段</option>{fields.map(field => <option key={field} value={field}>{field}</option>)}</select><label>条件</label><select value={condition.operator} onChange={event => stageConfig(selected.id, { condition: { ...condition, operator: event.target.value } })}>{operators.map(([value, label]) => <option key={value} value={value}>{label}</option>)}</select>{!['is_empty', 'is_not_empty'].includes(condition.operator) && <><label>比较值</label><input value={condition.value} onChange={event => stageConfig(selected.id, { condition: { ...condition, value: event.target.value } })} placeholder="例如 6602" /></>}</>}
        {['decimal','text_clean','text_extract'].includes(selected.kind) && <><p>新增结果列，原字段保留。修改规则后请计算本步。</p><ColumnActionPanel fields={fields} selected={fields} busy={busy} fieldTypes={Object.fromEntries((inputSchema||[]).map(f=>[f.name,f.data_type]))} editing={selected} onConfig={config=>editConfig(selected.id,config)} />{activeContracts[selected.id]?.diagnostics.map((d,i)=><div key={i} className="inline-error" role="alert">{d.message}</div>)}</>}
        {['join','union'].includes(selected.kind)&&selected.version===2&&<JoinAppendPanel key={JSON.stringify([selected.id,selected.config,project.workflow.edges.filter(edge=>edge.target===selected.id)])} node={selected} project={project} contracts={activeContracts} busy={busy} apply={(node,edges)=>editProject(previous=>applyStepCommand(previous,{kind:'configure_inputs',node,edges}))}/>}
        {String(selected.kind)==='calculate'&&<p className="inline-error" role="alert">该节点已移除。请删除此步骤，并用公式计算重新添加差额列。</p>}
        {selected.kind==='derive'&&<FormulaColumnPanel key={selected.id} fields={fields} project={project} nodeId={input?.id||selected.id} result={validRun} busy={busy} editing={selected} onAction={recordTableAction} onConfig={config=>editConfig(selected.id,config)}/>}
        {(['sequence','sort','duplicates','deduplicate'].includes(selected.kind)||(selected.kind==='filter'&&Array.isArray(selected.config.conditions)))&&<RowOperationsPanel key={JSON.stringify([selected.id,selected.config])} fields={fields} result={validRun} nodeId={input?.id} busy={busy} editing={selected} onAction={recordTableAction} onConfig={config=>editConfig(selected.id,config)}/>}
        {selected.kind==='aggregate'&&selected.version===2&&<AggregatePanel key={selected.id} fields={fields} busy={busy} editing={selected} onAction={recordTableAction} onConfig={config=>editConfig(selected.id,config)}/>}
        {selected.kind==='pivot'&&<PivotPanel key={JSON.stringify([selected.id,selected.config])} node={selected} fields={fields} busy={busy} apply={config=>editConfig(selected.id,config)}/>}
        {selected.kind === 'text_extract' && <p>未提取成功的行仍保留在主结果，并在异常视图中标记；两者记录有重叠。</p>}
        {selected.kind === 'decimal' && <p>{selected.version===2?'金额按单元格转换，整行保留。有效金额参与合计，失败格保留原值和原因。空白默认保留为空，按列选择补零。':'旧版规则将金额异常行分流，沿用保存时的处理口径。'}</p>}
        {selected.kind==='decimal'&&selected.version===1&&<div><p>改用按格处理后会保留原异常行，下游行数和合计可能变化，请重新计算复核。</p><button disabled={busy} onClick={()=>editNode(selected.id,node=>({...node,version:2}))}>此步骤改用按格金额规则</button></div>}
        {selected.kind === 'export' && <><p>固定导出规则可供重复运行；普通导出直接使用结果区的导出按钮。</p><button className="secondary-action" disabled={busy} onClick={()=>{void legacyExport();}}>按此节点设置导出</button><label>输出目录</label><div className="input-action"><input readOnly value={String(selectedConfig?.output_root || '')} placeholder="尚未选择输出目录" /><button onClick={() => chooseOutput(selected.id)}>浏览</button></div><label>结果文件名</label><input value={String(selectedConfig?.file_name || '')} onChange={event => stageConfig(selected.id, { file_name: event.target.value })} placeholder="处理结果.csv" /></>}
        {(!toolCatalog.some(tool => tool.kind === selected.kind) || selected.kind === 'map' && !modernMap || selected.kind === 'filter' && !modernFilter && !Array.isArray(selected.config.conditions)) && <><p>这是旧模板中的操作。旧规则保持原样，可在运行前查看参数和绑定；此轮尚未提供新的可视配置表单。</p><pre>{JSON.stringify(selected.config, null, 2)}</pre></>}
        {review && !hasUnappliedReading() && <div className="review-mini"><strong>本次运行</strong><p>{review.rows} 行 · 异常 {review.exceptions} · 未纳入 {review.excluded}</p><button onClick={() => { if (run) void showReview(run, review); }}>查看该步结果</button>{review.excluded > 0 && <button onClick={() => { if (run) void showReview(run, review, 'excluded'); }}>未纳入</button>}{review.exceptions > 0 && <button onClick={() => { if (run) void showReview(run, review, 'exceptions'); }}>异常</button>}</div>}
      </div></aside>}
      {showResults&&<div className="results-divider" role="separator" aria-label="上下调整表格区域高度" aria-orientation="horizontal" aria-controls="node-preview-region"
        tabIndex={0} aria-valuemin={workAreaHeight?Math.round(resultsMin/workAreaHeight*100):0} aria-valuemax={workAreaHeight?Math.round(resultsMax/workAreaHeight*100):100} aria-valuenow={workAreaHeight?Math.round(resultsHeight/workAreaHeight*100):45}
        title="上下拖动调整表格高度；双击恢复默认。也可用上下方向键调整。"
        onPointerDown={startResultsDrag} onPointerMove={moveResultsDrag} onPointerUp={endResultsDrag} onPointerCancel={endResultsDrag}
        onLostPointerCapture={()=>{resultsDrag.current=null;setResizingResults(false);}} onDoubleClick={resetResults}
        onKeyDown={event=>{if(['ArrowUp','ArrowDown','Home','End'].includes(event.key)){event.preventDefault();resizeResults(event.key==='Home'?resultsMin:event.key==='End'?resultsMax:resultsHeight+(event.key==='ArrowUp'?40:-40));}}}><span/></div>}
      {showResults && selected && <NodePreviewPanel state={visiblePreview} query={activeQuery} node={selected}
        loadFilterValues={loadFilterValues}
        toolbarSupplement={canTableAction && tableDownstream.length > 1 && <label>
        表格操作插入位置 <select value={tableInsertEdge?.id ?? ''} onChange={event=>setTableInsertEdgeId(event.target.value)}>
          <option value="">请选择下游连线</option>
          {tableDownstream.map(edge=><option key={edge.id} value={edge.id}>{project.workflow.nodes.find(node=>node.id===edge.target)?.label ?? edge.target}（{edge.target_port ?? '输入'}）</option>)}
        </select>
      </label>}
        pageOffset={resultOffset} onPage={offset=>setResultPage({key:resultPageKey,offset})}
        expanded={resultsExpanded} toggleExpanded={()=>setResultsExpanded(value=>!value)}
        fieldTypes={Object.fromEntries((activeContracts[selected.id]?.outputs[activeQuery.category]||[]).map(f=>[f.name,f.data_type]))}
        links={incomingLinks(project,selected.id).map(edge=>({id:edge.id,label:`${edge.target_port || 'in'} ← ${project.workflow.nodes.find(n=>n.id===edge.source)?.label || edge.source}`}))}
        reviews={validRun?.reviews || []} change={setPreviewChoice} select={id=>{setSelectedId(id);setPreviewChoice({nodeId:id,view:'output',category:'normal'});}} close={()=>setResultsOpen(false)}
        busy={busy} canAct={canTableAction} onAction={recordTableAction} result={validRun} project={project} process={()=>{void runCurrent();}} exportRows={()=>{void exportCurrent();}}
        exportEnabled={!hasUnappliedReading() && (selected.kind!=='source' || Boolean(previewContext.sources[selected.id]?.confirmed) || visiblePreview.data?.kind==='run')}
        exportReady={Boolean(validRun?.reviews.some(r=>r.node_id===(visiblePreview.direction==='input'?visiblePreview.sourceId:selected.id)))}
        loadRawWindow={(row,column,tail)=>{void viewSourceWindow(selected.id,row,column,tail);}}
        rawRow={chooseRawRow} headerRows={draft?.header_rows || []} dataStart={draft?.data_start_row} focus={focusCell?.nodeId===selected.id?focusCell:null} />}

    </div>
    <ProjectGuardDialog open={guardOpen} busy={guardBusy} onChoice={choice=>{void guardChoice(choice);}} />
    <footer className={`status-line ${/失败|未通过|不能|缺少/.test(status) ? 'status-error' : ''}`}>{status}<span>原始数据仅在本机处理</span></footer>
  </div>;
}
