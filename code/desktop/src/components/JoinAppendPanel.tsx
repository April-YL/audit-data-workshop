import { useEffect,useRef,useState } from 'react';
import {invoke} from '@tauri-apps/api/core';
import type { FieldDescriptor, NodeContract } from '../preview';
import type { ProjectFile, WorkEdge, WorkNode } from '../project';
import {applyStepCommand} from '../project';
import StepEditDialog from './StepEditDialog';

type Key = {left:string;right:string};
type Column = {side:'left'|'right';source:string;target:string};
type Target = {name:string;data_type:string};
type Props = {
  node: WorkNode;
  project: ProjectFile;
  contracts: Record<string,NodeContract>;
  busy: boolean;
  apply: (node: WorkNode, edges: WorkEdge[]) => void;
};

export default function JoinAppendPanel(p: Props) {
  const join = p.node.kind === 'join';
  const initialPorts = join ? ['left','right'] : p.node.config.ports as string[];
  const [ports, setPorts] = useState(initialPorts);
  const [sources, setSources] = useState(initialPorts.map(port=>p.project.workflow.edges.find(edge=>edge.target===p.node.id&&edge.target_port===port)?.source||''));
  const [mode, setMode] = useState(String(p.node.config.mode||'left'));
  const [empty, setEmpty] = useState(Boolean(p.node.config.match_empty));
  const [keys, setKeys] = useState<Key[]>(p.node.config.keys as Key[]||[]);
  const [columns, setColumns] = useState<Column[]>(p.node.config.columns as Column[]||[]);
  const [targets, setTargets] = useState<Target[]>(p.node.config.fields as Target[]||[]);
  const [mappings, setMappings] = useState<(string|null)[][]>(p.node.config.mappings as (string|null)[][]||initialPorts.map(()=>[]));
  const [excluded, setExcluded] = useState<string[][]>(p.node.config.excluded as string[][]||initialPorts.map(()=>[]));
  const [generation, setGeneration] = useState(0);
  const [allowExpand,setAllowExpand]=useState(Boolean(p.node.config.allow_expand));
  const [preflight,setPreflight]=useState<{matched:number;left_only:number;right_only:number;expands_left:boolean;expands_right:boolean}|null>(null);
  const [checking,setChecking]=useState(false);
  const [checkError,setCheckError]=useState('');
  const checkKey=JSON.stringify([p.project,sources,mode,keys,columns,empty]);
  const checkRef=useRef(checkKey);checkRef.current=checkKey;
  const previousCheckKey=useRef(checkKey);
  useEffect(()=>{
    setPreflight(null);setCheckError('');
    if(previousCheckKey.current!==checkKey)setAllowExpand(false);
    previousCheckKey.current=checkKey;
  },[checkKey]);
  const candidates = p.project.workflow.nodes.filter(node=>node.id!==p.node.id&&node.kind!=='export');
  const fields = (index:number):FieldDescriptor[] => (p.contracts[sources[index]]?.outputs.normal||[]).filter(field=>!field.name.startsWith('__')&&field.name!=='company');
  const fieldSelect = (index:number,value:string,change:(value:string)=>void) =>
    <select value={value} onChange={event=>change(event.target.value)}><option value="">请选择字段</option>{fields(index).map(field=><option key={field.name}>{field.name}</option>)}</select>;
  const reset = () => {
    setPorts(initialPorts);
    setSources(initialPorts.map(port=>p.project.workflow.edges.find(edge=>edge.target===p.node.id&&edge.target_port===port)?.source||''));
    setMode(String(p.node.config.mode||'left'));setEmpty(Boolean(p.node.config.match_empty));
    setKeys(p.node.config.keys as Key[]||[]);setColumns(p.node.config.columns as Column[]||[]);
    setTargets(p.node.config.fields as Target[]||[]);
    setMappings(p.node.config.mappings as (string|null)[][]||initialPorts.map(()=>[]));
    setExcluded(p.node.config.excluded as string[][]||initialPorts.map(()=>[]));
    setGeneration(value=>value+1);
  };
  const moveInput = (index:number, direction:number) => {
    const target=index+direction;
    const swap = <T,>(values:T[]) => values.map((value,i)=>i===index?values[target]:i===target?values[index]:value);
    setPorts(swap);setSources(swap);setMappings(swap);setExcluded(swap);
  };
  const configured = () => {
    if (sources.some(source=>!source)) throw new Error('请逐个选择输入。');
    if (join && (!keys.length||!columns.length||keys.some(key=>!key.left||!key.right))) throw new Error('请配置匹配键和带回列。');
    if (!join && !targets.length) throw new Error('请添加追加目标列。');
    const config = join ? {mode,keys,columns,match_empty:empty,allow_expand:allowExpand} : {ports,fields:targets,mappings,excluded};
    return {node:{...p.node,config},edges:ports.map((port,index)=>({
      id:'edge_'+crypto.randomUUID().replaceAll('-','').slice(0,12),
      source:sources[index],target:p.node.id,source_port:'main',target_port:port,
    }))};
  };
  const apply = () => {
    if(join&&allowExpand&&!preflight)throw Error('允许展开前请先预检关联规模。');
    const {node,edges}=configured();
    p.apply(node,edges);
  };
  const check=async()=>{
    const key=checkKey;setChecking(true);setCheckError('');
    try{
      const {node,edges}=configured();
      const project=applyStepCommand(p.project,{kind:'configure_inputs',node,edges});
      const result=await invoke<NonNullable<typeof preflight>>('preflight_join',{project,nodeId:p.node.id});
      if(checkRef.current===key)setPreflight(result);
    }catch(error){if(checkRef.current===key)setCheckError(String(error));}finally{setChecking(false);}
  };
  return <StepEditDialog key={generation} title={join?'配置多表关联':'配置多表追加'} busy={p.busy} apply={apply} cancel={reset}>
    <p>应用前的选择保留在草稿中。输入字段来自各上游步骤的当前规则。</p>
    {ports.map((port,index)=><fieldset key={port}><legend>{join?(index===0?'左侧输入':'右侧输入'):`第 ${index+1} 份输入`}</legend>
      <select aria-label={`输入 ${index+1}`} value={sources[index]} onChange={event=>{
        setSources(previous=>previous.map((value,i)=>i===index?event.target.value:value));
        if (join) {setKeys([]);setColumns([]);} else {
          setMappings(previous=>previous.map((value,i)=>i===index?targets.map(()=>null):value));
          setExcluded(previous=>previous.map((value,i)=>i===index?[]:value));
        }
      }}><option value="">请选择上游步骤</option>{candidates.map(node=><option key={node.id} value={node.id}>{node.label} · {p.project.bindings[node.id]?.path.split(/[\\/]/).pop()||node.id.slice(-6)}</option>)}</select>
      {sources[index]&&!fields(index).length&&<p>上游字段尚未就绪，请检查上游规则或读取资料。</p>}
      {!join&&<label>排除列<select multiple aria-label={`第 ${index+1} 份排除列`} value={excluded[index]} onChange={event=>{
        const values=Array.from(event.target.selectedOptions,option=>option.value);
        setExcluded(previous=>previous.map((value,i)=>i===index?values:value));
      }}>{fields(index).map(field=><option key={field.name}>{field.name}</option>)}</select></label>}
      {!join&&<div className="input-action">
        <button type="button" disabled={index===0} onClick={()=>moveInput(index,-1)}>上移输入</button>
        <button type="button" disabled={index===ports.length-1} onClick={()=>moveInput(index,1)}>下移输入</button>
        <button type="button" disabled={ports.length<=2} onClick={()=>{
          setPorts(previous=>previous.filter((_,i)=>i!==index));setSources(previous=>previous.filter((_,i)=>i!==index));
          setMappings(previous=>previous.filter((_,i)=>i!==index));setExcluded(previous=>previous.filter((_,i)=>i!==index));
        }}>移除输入</button>
      </div>}
    </fieldset>)}
    {join ? <>
      <label>连接方式<select value={mode} onChange={event=>setMode(event.target.value)}><option value="inner">内连接</option><option value="left">左连接</option><option value="right">右连接</option><option value="full">全连接</option></select></label>
      <label><input type="checkbox" checked={empty} onChange={event=>setEmpty(event.target.checked)}/>空文本参与匹配</label>
      <p>NULL不匹配。预检会计算各输入，按键计数后预测行数。</p>
      <button type="button" disabled={checking||p.busy} onClick={()=>{void check();}}>预检关联规模</button>
      {checking&&<p role="status">正在核对实际输入，请稍候…</p>}
      {checkError&&<p role="alert">{checkError}</p>}
      {preflight&&<p>预计匹配 {preflight.matched} 行，仅左 {preflight.left_only} 行，仅右 {preflight.right_only} 行。{preflight.expands_left?'左侧原记录及金额会重复出现。':''}{preflight.expands_right?'右侧原记录及金额会重复出现。':''}</p>}
      <label><input type="checkbox" checked={allowExpand} disabled={!preflight||checking} onChange={event=>setAllowExpand(event.target.checked)}/>已核对重复计入影响，允许展开，最多输出100万行</label>
      {keys.map((key,index)=><div className="input-action" key={index}>
        {fieldSelect(0,key.left,value=>setKeys(previous=>previous.map((item,i)=>i===index?{...item,left:value}:item)))}
        {fieldSelect(1,key.right,value=>setKeys(previous=>previous.map((item,i)=>i===index?{...item,right:value}:item)))}
        <button type="button" onClick={()=>setKeys(previous=>previous.filter((_,i)=>i!==index))}>移除键</button>
      </div>)}
      <button type="button" onClick={()=>setKeys(previous=>[...previous,{left:'',right:''}])}>添加匹配键</button>
      {([0,1] as const).map(side=><fieldset key={side}><legend>{side===0?'左侧':'右侧'}带回列</legend>{fields(side).map(field=>{
        const direction=side===0?'left':'right';
        const selected=columns.find(column=>column.side===direction&&column.source===field.name);
        return <label key={field.name}><input type="checkbox" checked={Boolean(selected)} onChange={event=>setColumns(previous=>event.target.checked?[...previous,{side:direction,source:field.name,target:field.name}]:previous.filter(column=>!(column.side===direction&&column.source===field.name)))}/>{field.name}
          {selected&&<input aria-label={`${direction} ${field.name} 输出列名`} value={selected.target} onChange={event=>setColumns(previous=>previous.map(column=>column===selected?{...column,target:event.target.value}:column))}/>}
        </label>;
      })}</fieldset>)}
    </> : <>
      <button type="button" onClick={()=>{setPorts(previous=>[...previous,'input_'+crypto.randomUUID().slice(0,8)]);setSources(previous=>[...previous,'']);setMappings(previous=>[...previous,targets.map(()=>null)]);setExcluded(previous=>[...previous,[]]);}}>添加输入</button>
      {targets.map((target,index)=><fieldset key={index}><legend>目标列 {index+1}</legend>
        <input aria-label={`目标列名 ${index+1}`} value={target.name} onChange={event=>setTargets(previous=>previous.map((value,i)=>i===index?{...value,name:event.target.value}:value))}/>
        <span>{target.data_type||'选择首个映射后确认类型'}</span>
        {ports.map((port,input)=><label key={port}>第 {input+1} 份<select value={mappings[input]?.[index]||''} onChange={event=>{
          const source=event.target.value;
          setMappings(previous=>previous.map((mapping,i)=>i===input?mapping.map((value,j)=>j===index?source||null:value):mapping));
          const field=fields(input).find(field=>field.name===source);
          if (field) setTargets(previous=>previous.map((value,i)=>i===index?{...value,data_type:field.data_type}:value));
        }}><option value="">明确补空</option>{fields(input).map(field=><option key={field.name}>{field.name}</option>)}</select></label>)}
        <button type="button" onClick={()=>{setTargets(previous=>previous.filter((_,i)=>i!==index));setMappings(previous=>previous.map(mapping=>mapping.filter((_,i)=>i!==index)));}}>移除目标列</button>
      </fieldset>)}
      <button type="button" onClick={()=>{setTargets(previous=>[...previous,{name:'',data_type:''}]);setMappings(previous=>previous.map(mapping=>[...mapping,null]));}}>添加目标列</button>
      <p>输入按上方顺序追加。每个字段须映射或明确排除，类型须一致。</p>
    </>}
  </StepEditDialog>;
}
