import {useEffect,useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import type {Condition,TableAction,WorkNode,RunResult} from '../project';
type Mode='sequence'|'sort'|'duplicates'|'deduplicate'|'multi_filter';
const titles:Record<Mode,string>={sequence:'结果编号',sort:'排序',duplicates:'重复识别',deduplicate:'按规则去重',multi_filter:'多条件筛选'};
type Props={fields:string[];busy?:boolean;onAction:(a:TableAction)=>void|Promise<boolean>;editing?:WorkNode;onConfig?:(c:Record<string,unknown>)=>void;result?:RunResult|null;nodeId?:string};

function ValuesPicker({condition,conditions,match,result,nodeId,change}:{condition:Condition;conditions:Condition[];match:string;result?:RunResult|null;nodeId?:string;change:(values:(string|null)[],operator:string)=>void}){
  const [open,setOpen]=useState(false);const [search,setSearch]=useState('');const [offset,setOffset]=useState(0);
  const [values,setValues]=useState<(string|null)[]>([]);const [message,setMessage]=useState('');
  const scope=JSON.stringify({match,conditions:conditions.filter(item=>item.field!==condition.field)});
  useEffect(()=>{setOffset(0);},[scope,condition.field]);
  useEffect(()=>{
    if(!open||!result||!nodeId||!condition.field)return;
    let active=true;setValues([]);setMessage('正在查询输入和其他列条件范围…');
    void invoke<(string|null)[]>('distinct_result_values',{batch:result.run_dir,nodeId,field:condition.field,search,offset,filterConfig:JSON.parse(scope)})
      .then(values=>{if(active){setValues(values);setMessage(values.length?'':'没有符合条件的值。');}})
      .catch(error=>{if(active)setMessage(String(error));});
    return()=>{active=false;};
  },[open,result?.run_dir,nodeId,condition.field,scope,search,offset]);
  return <div><button type="button" disabled={!result||!condition.field} onClick={()=>setOpen(value=>!value)}>全量值列表</button>
    {open&&<div className="filter-value-list"><label>搜索值<input value={search} onChange={event=>{setSearch(event.target.value);setOffset(0);}}/></label>
      <p role="status">{message}</p>{values.slice(0,200).map(value=><label key={JSON.stringify(value)}><input type="checkbox" checked={condition.values?.includes(value)||false} onChange={event=>{
        const previous=condition.values||[];
        change(event.target.checked?[...previous,value]:previous.filter(item=>item!==value),['include_values','exclude_values'].includes(condition.operator)?condition.operator:'include_values');
      }}/>{value===null?'空值（NULL）':value===''?'空文本':value}</label>)}
      <button type="button" disabled={!offset} onClick={()=>setOffset(Math.max(0,offset-200))}>上一组</button>
      <button type="button" disabled={values.length<=200} onClick={()=>setOffset(offset+200)}>下一组</button>
      <p>已选 {condition.values?.length||0} 个值，跨组保留。</p>
    </div>}
  </div>;
}
export default function RowOperationsPanel(p:Props){
  const initial=p.editing?(p.editing.kind==='filter'?'multi_filter':p.editing.kind as Mode):null;
  const [mode,setMode]=useState<Mode|null>(initial);const [keys,setKeys]=useState<string[]>((p.editing?.config.keys as string[])||[]);
  const [order,setOrder]=useState<{field:string;direction:string}[]>((p.editing?.config.order as {field:string;direction:string}[])||[]);
  const [keep,setKeep]=useState(String(p.editing?.config.keep||'first'));const [match,setMatch]=useState(String(p.editing?.config.match||'all'));
  const [conditions,setConditions]=useState<Condition[]>((p.editing?.config.conditions as Condition[])||[]);
  const [names,setNames]=useState({target:String(p.editing?.config.target||'是否重复'),count_target:String(p.editing?.config.count_target||'重复条数'),rank_target:String(p.editing?.config.rank_target||'组内序号')});
  const [sequenceTarget,setSequenceTarget]=useState(String(p.editing?.config.target||'结果编号'));
  const [sequenceStart,setSequenceStart]=useState(Number(p.editing?.config.start??1));
  const [error,setError]=useState('');
  useEffect(()=>{setError('');},[mode]);
  async function submit(){if(!mode)return;setError('');try{
    let config:Record<string,unknown>;
    if(mode==='sequence'){if(!sequenceTarget.trim()||!Number.isSafeInteger(sequenceStart))throw Error('请填写编号列名和有效整数起点。');config={target:sequenceTarget,start:sequenceStart};}
    else if(mode==='sort'){if(!order.length||order.some(o=>!p.fields.includes(o.field)))throw Error('请选择排序字段。');config={order};}
    else if(mode==='multi_filter'){if((!conditions.length&&!p.editing)||conditions.some(c=>!p.fields.includes(c.field)))throw Error('请填写条件。');config={match,conditions,...(!conditions.length?{cleared:true}:{})};}
    else{if(!keys.length)throw Error('请选择判断字段。');config=mode==='duplicates'?{keys,...names}:{keys,keep};}
    if(p.editing)p.onConfig?.(config);else {if(await p.onAction({kind:mode,config})===false)throw Error('操作未完成；输入已保留，请查看状态栏原因。');setMode(null);}
  }catch(e){setError(String(e));}}
  const recordRule=(p.editing?.config.conditions as {operator:string;records?:unknown[]}[]|undefined)?.find(condition=>['include_records','exclude_records'].includes(condition.operator));
  if(recordRule)return <section><strong>{recordRule.operator==='include_records'?'保留':'排除'}选中来源记录</strong>
    <p>已登记 {recordRule.records?.length||0} 条来源记录。原始文件变化后需要重新选择，避免记录号指向其他业务。</p>
    <button disabled={p.busy} onClick={()=>p.onConfig?.({match:'all',conditions:[],cleared:true})}>清除此步骤记录条件</button>
  </section>;
  const options=p.fields.map(f=><option key={f}>{f}</option>);
  const form=mode&&<form className="operation-form" onSubmit={e=>{e.preventDefault();void submit();}}><strong>{titles[mode]}</strong>
    {mode==='sequence'?<><label>编号列名<input value={sequenceTarget} onChange={event=>setSequenceTarget(event.target.value)}/></label><label>编号起点<input type="number" step={1} value={sequenceStart} onChange={event=>setSequenceStart(Number(event.target.value))}/></label><p>按当前确定顺序编号，保留原始行号。换料复算后编号重新生成。</p></>:mode==='sort'?<><p>按下列字段依次排序，空值最后；相同值保持此前顺序。</p>{order.map((o,i)=><div className="operation-row" key={i}><select aria-label={`排序字段${i+1}`} value={o.field} onChange={e=>setOrder(order.map((v,j)=>j===i?{...v,field:e.target.value,values:[]}:v))} required><option value="">请选择</option>{options}</select><select aria-label={`排序方向${i+1}`} value={o.direction} onChange={e=>setOrder(order.map((v,j)=>j===i?{...v,direction:e.target.value}:v))}><option value="asc">升序</option><option value="desc">降序</option></select><button type="button" onClick={()=>setOrder(order.filter((_,j)=>j!==i))}>移除</button></div>)}<button type="button" onClick={()=>setOrder([...order,{field:'',direction:'asc'}])}>增加排序字段</button></>:
    mode==='multi_filter'?<><label>条件组合<select value={match} onChange={e=>setMatch(e.target.value)}><option value="all">全部满足</option><option value="any">任一满足</option></select></label>{conditions.map((c,i)=><div className="operation-row" key={i}><select aria-label={`条件字段${i+1}`} value={c.field} onChange={e=>setConditions(conditions.map((v,j)=>j===i?{...v,field:e.target.value,values:[]}:v))} required><option value="">请选择</option>{options}</select><select aria-label={`条件操作${i+1}`} value={c.operator} onChange={e=>setConditions(conditions.map((v,j)=>j===i?{...v,operator:e.target.value}:v))}>{[['equals','文本等于'],['not_equals','文本不等于'],['contains','包含'],['include_values','包含所选值'],['exclude_values','排除所选值'],['is_empty','为空'],['is_not_empty','不为空'],['numeric_gt','数值大于'],['numeric_gte','数值大于等于'],['numeric_lt','数值小于'],['numeric_lte','数值小于等于'],['numeric_eq','数值等于'],['numeric_neq','数值不等于'],['date_eq','日期等于'],['date_neq','日期不等于'],['date_gt','日期晚于'],['date_gte','日期不早于'],['date_lt','日期早于'],['date_lte','日期不晚于']].map(([v,l])=><option key={v} value={v}>{l}</option>)}</select>{!['is_empty','is_not_empty','include_values','exclude_values'].includes(c.operator)&&<input aria-label={`条件值${i+1}`} value={c.value} onChange={e=>setConditions(conditions.map((v,j)=>j===i?{...v,value:e.target.value}:v))}/>}<ValuesPicker condition={c} conditions={conditions} match={match} result={p.result} nodeId={p.nodeId} change={(values,operator)=>setConditions(previous=>previous.map((item,index)=>index===i?{...item,values,operator}:item))}/><button type="button" onClick={()=>setConditions(conditions.filter((_,j)=>j!==i))}>移除</button></div>)}<button type="button" onClick={()=>setConditions([...conditions,{field:'',operator:'equals',value:''}])}>增加条件</button>{p.editing&&<button type="button" onClick={()=>p.onConfig?.({match,conditions:[],cleared:true})}>清除此步骤全部条件</button>}<p>值列表使用本步输入及其他列条件，忽略本列条件。清除只影响当前步骤。</p><p>未满足及条件为空的记录进入未纳入，原行保留。</p></>:
    <><p>选择精确判断字段；不自动清理空格或改变大小写。NULL同组，空字符串另组。</p>{p.fields.map(f=><label key={f}><input type="checkbox" checked={keys.includes(f)} onChange={e=>setKeys(e.target.checked?[...keys,f]:keys.filter(k=>k!==f))}/>{f}</label>)}{mode==='deduplicate'?<><label>保留<select value={keep} onChange={e=>setKeep(e.target.value)}><option value="first">当前顺序首条</option><option value="last">当前顺序末条</option></select></label><p>其他记录进入未纳入，可单独查看和导出。</p></>:Object.entries(names).map(([key,v])=><label key={key}>{key==='target'?'重复标记列':key==='count_target'?'组内条数列':'组内序号列'}<input value={v} onChange={e=>setNames({...names,[key]:e.target.value})} required/></label>)}</>}
    {error&&<p className="inline-error" role="alert">{error}</p>}<button className="primary" disabled={p.busy} type="submit">{p.editing?'应用规则':'应用并计算'}</button>{!p.editing&&<button type="button" onClick={()=>setMode(null)}>取消</button>}
  </form>;
  if(p.editing)return form;
  return <>{(Object.keys(titles) as Mode[]).map(m=><button key={m} disabled={p.busy} onClick={()=>setMode(m)}>{titles[m]}</button>)}{mode&&<div className="modal-backdrop"><section className="column-action-dialog" role="dialog" aria-label={titles[mode]}>{form}</section></div>}</>;
}
