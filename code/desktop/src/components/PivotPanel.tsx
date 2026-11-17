import {useEffect,useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import type {Preview,RunResult,WorkNode} from '../project';
import StepEditDialog from './StepEditDialog';

type Metric={op:string;field:string;target:string};
type Props={node:WorkNode;fields:string[];busy:boolean;apply:(config:Record<string,unknown>)=>void};
export type PivotSelection={groupValues:Record<string,string|null>;column:string;totalRow:boolean};

export function PivotMembers({node,result,selection}:{node:WorkNode;result:RunResult;selection:PivotSelection}){
  const [details,setDetails]=useState<Preview|null>(null);
  const [error,setError]=useState('');
  useEffect(()=>{
    let active=true;setDetails(null);setError('');
    void invoke<Preview>('inspect_pivot',{batch:result.run_dir,nodeId:node.id,...selection,limit:200})
      .then(value=>{if(active)setDetails(value);}).catch(error=>{if(active)setError(String(error));});
    return()=>{active=false;};
  },[result.run_dir,node.id,selection]);
  return <section className="group-details"><strong>透视组成明细</strong>
    {error?<p role="alert">{error}</p>:details?<><p>展示 {details.rows.length} 行／全量成员 {details.total_rows} 行</p>
      <div className="result-scroll"><table><thead><tr>{details.columns.map((name,index)=><th key={index}>{name}</th>)}</tr></thead><tbody>{details.rows.map((row,index)=><tr key={index}>{row.map((value,column)=><td key={column}>{value}</td>)}</tr>)}</tbody></table></div>
    </>:<p>正在读取本次运行成员…</p>}
  </section>;
}

export default function PivotPanel(p:Props){
  const [rows,setRows]=useState(p.node.config.row_by as string[]||[]);
  const [columns,setColumns]=useState(p.node.config.column_by as string[]||[]);
  const [metrics,setMetrics]=useState(p.node.config.metrics as Metric[]||[]);
  const reset=()=>{setRows(p.node.config.row_by as string[]||[]);setColumns(p.node.config.column_by as string[]||[]);setMetrics(p.node.config.metrics as Metric[]||[]);};
  return <StepEditDialog title="交叉透视" busy={p.busy} cancel={reset} apply={()=>{
    if(!rows.length||!columns.length||!metrics.length)throw Error('请选择行、列维度和统计项。');
    if(rows.some(name=>columns.includes(name)))throw Error('行列维度不能重复。');
    p.apply({row_by:rows,column_by:columns,metrics});
  }}>
    <p>可先用筛选步骤限定范围。总计按原始成员重新计算，动态列最多200列。</p>
    {(['行维度','列维度'] as const).map((label,index)=><fieldset key={label}><legend>{label}</legend>{p.fields.map(field=><label key={field}>
      <input type="checkbox" checked={(index===0?rows:columns).includes(field)} onChange={event=>{
        const update=index===0?setRows:setColumns;
        update(previous=>event.target.checked?[...previous,field]:previous.filter(name=>name!==field));
      }}/>{field}
    </label>)}</fieldset>)}
    {metrics.map((metric,index)=><fieldset key={index}><legend>统计项 {index+1}</legend>
      <select value={metric.op} onChange={event=>setMetrics(previous=>previous.map((item,i)=>i===index?{...item,op:event.target.value}:item))}>
        <option value="sum">求和</option><option value="rows">记录数</option><option value="count">非空计数</option><option value="distinct">去重计数</option>
      </select>
      {metric.op!=='rows'&&<select value={metric.field} onChange={event=>setMetrics(previous=>previous.map((item,i)=>i===index?{...item,field:event.target.value}:item))}><option value="">请选择统计字段</option>{p.fields.map(field=><option key={field}>{field}</option>)}</select>}
      <input aria-label={`统计名称 ${index+1}`} value={metric.target} onChange={event=>setMetrics(previous=>previous.map((item,i)=>i===index?{...item,target:event.target.value}:item))}/>
      <button type="button" onClick={()=>setMetrics(previous=>previous.filter((_,i)=>i!==index))}>移除统计项</button>
    </fieldset>)}
    <button type="button" onClick={()=>setMetrics(previous=>[...previous,{op:'sum',field:'',target:''}])}>添加统计项</button>
    <p>缺组合保留为空，实际零显示0。汇总来源须通过本次运行成员回查。</p>
  </StepEditDialog>;
}
