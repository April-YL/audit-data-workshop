import {useEffect,useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import type {RunResult} from '../project';
import {resultRequestKey,type PreviewState,type ResultSummary} from '../preview';
import {isNumericType} from './ColumnActionPanel';
export default function ResultSummaryPanel(p:{state:PreviewState;result:RunResult|null;fieldTypes:Record<string,string>;busy?:boolean}){
  const [chosen,setChosen]=useState<string[]>([]);
  const [response,setResponse]=useState<{key:string;data?:ResultSummary;error?:string}|null>(null);
  const fields=Object.keys(p.fieldTypes).filter(f=>!f.startsWith('__')&&f!=='company'&&isNumericType(p.fieldTypes[f]));
  const selection=chosen.filter(f=>fields.includes(f));
  const ready=p.state.status==='ready'&&p.state.data?.kind==='run'&&p.state.direction==='output'&&!p.busy&&p.result?.run_dir===p.state.batch;
  const key=resultRequestKey(p.state.target,p.state.batch||'',selection);
  const selectionKey=JSON.stringify(selection);
  useEffect(()=>{
    let current=true;
    if(!ready||!selection.length){setResponse(null);return;}
    setResponse(null);
    void invoke<ResultSummary>('summarize_result',{batch:p.result?.run_dir,nodeId:p.state.sourceId||p.state.target.nodeId,category:p.state.category||'normal',fields:selection}).then(data=>{if(current)setResponse({key,data});}).catch(error=>{if(current)setResponse({key,error:String(error)});});
    return ()=>{current=false;};
  },[ready,key,selectionKey,p.result]);
  const shown=response?.key===key?response:null;
  return <section className="result-summary" aria-label="全量金额统计"><strong>全量金额统计</strong>
    <div className="summary-fields">{fields.map(f=><label key={f}><input type="checkbox" checked={selection.includes(f)} onChange={e=>setChosen(e.target.checked?[...selection,f]:selection.filter(v=>v!==f))}/>{f}</label>)}</div>
    {!ready?<p>尚无当前规则的有效全量输出；计算后查看。局部样例不参与合计。</p>:!fields.length?<p>没有已确认的数值列，文本金额请先转换。</p>:!selection.length?<p>选择统计列；统计当前类别全量记录，随筛选计算更新。</p>:shown?.error?<p role="alert">统计失败：{shown.error}</p>:shown?.data?<><p>当前类别全量 {shown.data.rows} 行</p>{shown.data.fields.map(f=><p key={f.name}><strong>{f.name}</strong>：合计 {f.sum===null?'无有效数值':f.sum} · 有效 {f.non_null_count} · 未计入 {f.null_count} 格{f.amount_counts && <> · 空白 {f.amount_counts.blank} · 转换失败 {f.amount_counts.failed} · 补零 {f.amount_counts.filled_zero}</>}</p>)}</>:<p role="status">正在计算全量合计…</p>}
  </section>;
}
