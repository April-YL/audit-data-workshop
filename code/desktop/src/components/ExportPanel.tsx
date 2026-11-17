import {useEffect,useRef,useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import {save} from '@tauri-apps/plugin-dialog';

type Plan={token:string;rows:number;columns:string[];order:string};
type Condition={field:string;operator:string;value:string};
type Props={batch:string;nodeId:string;category:string;columns:string[];records:number[];busy?:boolean};

export default function ExportPanel(p:Props){
  const [open,setOpen]=useState(false);
  const [columns,setColumns]=useState(p.columns);
  const [selection,setSelection]=useState(false);
  const [conditions,setConditions]=useState<Condition[]>([]);
  const [match,setMatch]=useState('all');
  const [plan,setPlan]=useState<Plan|null>(null);
  const [message,setMessage]=useState('');
  const [busy,setBusy]=useState(false);
  const scope={columns,condition:conditions.length?{match,conditions}:null,records:selection?p.records:null};
  const key=JSON.stringify([p.batch,p.nodeId,p.category,scope]);
  const keyRef=useRef(key);keyRef.current=key;
  useEffect(()=>{setPlan(null);setMessage('');},[key]);
  async function preflight(){
    const request=key;setBusy(true);setMessage('');
    try{const result=await invoke<Plan>('preflight_export',{batch:p.batch,nodeId:p.nodeId,category:p.category,scope});
      if(keyRef.current===request)setPlan(result);
    }catch(error){if(keyRef.current===request)setMessage(String(error));}finally{setBusy(false);}
  }
  async function write(){
    if(!plan)return;
    const request=key;setBusy(true);
    try{
      const path=await save({defaultPath:'处理结果.csv',filters:[{name:'CSV',extensions:['csv']}]});
      if(typeof path!=='string'||request!==keyRef.current)return;
      const result=await invoke<{rows:number;path:string}>('export_scope',{batch:p.batch,nodeId:p.nodeId,category:p.category,scope,token:plan.token,path});
      if(request===keyRef.current)setMessage(`已导出 ${result.rows} 行至 ${result.path}`);
    }catch(error){setMessage(String(error));}finally{setBusy(false);}
  }
  return <section className="export-scope"><button disabled={p.busy} onClick={()=>setOpen(value=>!value)}>选择范围导出</button>
    {open&&<div className="operation-form"><strong>本次结果的导出范围</strong>
      <fieldset><legend>导出列</legend>{p.columns.map(name=><label key={name}><input type="checkbox" checked={columns.includes(name)} onChange={event=>setColumns(previous=>event.target.checked?[...previous,name]:previous.filter(value=>value!==name))}/>{name}</label>)}</fieldset>
      <label><input type="checkbox" checked={selection} onChange={event=>setSelection(event.target.checked)}/>仅选中记录，当前已选 {p.records.length} 行</label>
      <select aria-label="导出条件组合" value={match} onChange={event=>setMatch(event.target.value)}><option value="all">同时满足</option><option value="any">满足任一</option></select>
      {conditions.map((condition,index)=><div className="operation-row" key={index}>
        <select value={condition.field} onChange={event=>setConditions(previous=>previous.map((item,i)=>i===index?{...item,field:event.target.value}:item))}><option value="">选择字段</option>{p.columns.map(name=><option key={name}>{name}</option>)}</select>
        <select value={condition.operator} onChange={event=>setConditions(previous=>previous.map((item,i)=>i===index?{...item,operator:event.target.value}:item))}>
          <option value="equals">等于</option><option value="not_equals">不等于</option><option value="contains">包含</option><option value="is_empty">为空</option><option value="is_not_empty">不为空</option>
          <option value="numeric_gt">数值大于</option><option value="numeric_lt">数值小于</option><option value="date_eq">日期等于</option><option value="date_gt">日期晚于</option><option value="date_lt">日期早于</option>
        </select>
        <input value={condition.value} aria-label="导出比较值" onChange={event=>setConditions(previous=>previous.map((item,i)=>i===index?{...item,value:event.target.value}:item))}/>
        <button onClick={()=>setConditions(previous=>previous.filter((_,i)=>i!==index))}>移除条件</button>
      </div>)}
      <button onClick={()=>setConditions(previous=>[...previous,{field:'',operator:'equals',value:''}])}>增加导出条件</button>
      <button disabled={busy||p.busy} onClick={()=>{void preflight();}}>预检导出范围</button>
      {plan&&<><p>将导出 {plan.rows} 行、{plan.columns.length} 列。{plan.order}。</p><button disabled={busy||p.busy} onClick={()=>{void write();}}>确认范围并保存CSV</button></>}
      {message&&<p role="status">{message}</p>}
    </div>}
  </section>;
}
