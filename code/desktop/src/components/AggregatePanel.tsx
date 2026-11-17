import {useEffect,useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import type {Preview,RunResult,TableAction,WorkNode} from '../project';
type Metric={op:string;field:string;target:string};
type Props={fields:string[];busy?:boolean;onAction:(a:TableAction)=>void|Promise<boolean>;editing?:WorkNode;onConfig?:(c:Record<string,unknown>)=>void;result?:RunResult|null;nodeId?:string;groupValues?:Record<string,string|null>|null};
export default function AggregatePanel(p:Props){
  const [open,setOpen]=useState(Boolean(p.editing));const [keys,setKeys]=useState<string[]>((p.editing?.config.group_by as string[])||[]);
  const [metrics,setMetrics]=useState<Metric[]>((p.editing?.config.metrics as Metric[])||[]);
  const [error,setError]=useState('');const [details,setDetails]=useState<Preview|null>(null);const [detailError,setDetailError]=useState('');
  const groupKey=JSON.stringify(p.groupValues);
  useEffect(()=>{let current=true;setDetails(null);setDetailError('');if(p.result&&p.nodeId&&p.groupValues){void invoke<Preview>('inspect_group',{batch:p.result?.run_dir,nodeId:p.nodeId,groupValues:p.groupValues,limit:100}).then(v=>{if(current)setDetails(v);}).catch(e=>{if(current)setDetailError(String(e));});}return()=>{current=false;};},[p.result,p.nodeId,groupKey]);
  async function submit(){setError('');try{if(!keys.length||!metrics.length)throw Error('请选择分组字段及统计项。');if(metrics.some(m=>!m.target.trim()||(m.op!=='rows'&&!m.field)))throw Error('请填写统计字段和输出名称。');const config={group_by:keys,metrics};if(p.editing)p.onConfig?.(config);else {if(await p.onAction({kind:'aggregate',config})===false)throw Error('操作未完成；输入已保留，请查看状态栏原因。');setOpen(false);}}catch(e){setError(String(e));}}
  const options=p.fields.map(f=><option key={f}>{f}</option>);
  const form=<form className="operation-form" onSubmit={e=>{e.preventDefault();void submit();}}><strong>分类汇总</strong><p>创建汇总旁支，保留原明细及其处理链。</p>
    <fieldset><legend>分组字段</legend>{p.fields.map(f=><label key={f}><input type="checkbox" checked={keys.includes(f)} onChange={e=>setKeys(e.target.checked?[...keys,f]:keys.filter(k=>k!==f))}/>{f}</label>)}</fieldset>
    {metrics.map((m,i)=><div className="operation-row" key={i}><select aria-label={`统计方式${i+1}`} value={m.op} onChange={e=>setMetrics(metrics.map((v,j)=>j===i?{...v,op:e.target.value}:v))}><option value="sum">求和</option><option value="rows">记录数</option><option value="count">非空值计数</option><option value="distinct">去重计数（不计NULL）</option></select>{m.op!=='rows'&&<select aria-label={`统计字段${i+1}`} value={m.field} onChange={e=>setMetrics(metrics.map((v,j)=>j===i?{...v,field:e.target.value}:v))} required><option value="">请选择</option>{options}</select>}<input aria-label={`统计新列名${i+1}`} value={m.target} placeholder="输出列名" required onChange={e=>setMetrics(metrics.map((v,j)=>j===i?{...v,target:e.target.value}:v))}/><button type="button" onClick={()=>setMetrics(metrics.filter((_,j)=>j!==i))}>移除</button></div>)}
    <button type="button" onClick={()=>setMetrics([...metrics,{op:'sum',field:'',target:''}])}>增加统计项</button><p>空分组独立保留；求和同时生成“统计列名__nulls”空值条数。全空组金额为空。</p>
    {error&&<p className="inline-error" role="alert">{error}</p>}<button type="submit" className="primary" disabled={p.busy}>{p.editing?'应用规则':'创建汇总并计算'}</button>{!p.editing&&<button type="button" onClick={()=>setOpen(false)}>取消</button>}
  </form>;
  if(p.editing)return form;
  return <><button disabled={p.busy} onClick={()=>setOpen(true)}>分类汇总</button>{open&&<div className="modal-backdrop"><section className="column-action-dialog" role="dialog" aria-label="分类汇总">{form}</section></div>}
    {p.groupValues&&<section className="group-details"><strong>组成明细</strong>{detailError?<p role="alert">{detailError}</p>:details?<><p>局部 {details.rows.length} 行／全量成员 {details.total_rows} 行</p><div className="result-scroll"><table><thead><tr>{details.columns.map((c,i)=><th key={i}>{c}</th>)}</tr></thead><tbody>{details.rows.map((r,i)=><tr key={i}>{r.map((v,j)=><td key={j}>{v}</td>)}</tr>)}</tbody></table></div></>:<p>正在读取本批次组成明细…</p>}</section>}
  </>;
}
