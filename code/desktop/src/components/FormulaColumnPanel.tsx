import {useEffect,useRef,useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import type {Preview,ProjectFile,RunResult,TableAction,WorkNode} from '../project';
type Props={fields:string[];selected?:string[];project:ProjectFile;nodeId:string;result:RunResult|null;busy?:boolean;onAction:(a:TableAction)=>void|Promise<boolean>;editing?:WorkNode;onConfig?:(config:Record<string,unknown>)=>void};
export default function FormulaColumnPanel(p:Props){
  const [open,setOpen]=useState(Boolean(p.editing));const [moving,setMoving]=useState(false);
  const [target,setTarget]=useState(String(p.editing?.config.target||''));const [expression,setExpression]=useState(String(p.editing?.config.expression||''));
  const [placement,setPlacement]=useState<'first'|'last'|'before'|'after'>('last');const [anchor,setAnchor]=useState('');
  const [rowField,setRowField]=useState(''),[rowSort,setRowSort]=useState(''),[rowGroup,setRowGroup]=useState(''),[distance,setDistance]=useState('1'),[direction,setDirection]=useState('PREV');
  const [moveField,setMoveField]=useState('');const [error,setError]=useState('');const [trial,setTrial]=useState<Preview|null>(null);const [loading,setLoading]=useState(false);const [trialKey,setTrialKey]=useState('');
  const trialSerial=useRef(0);
  const trialIdentity=JSON.stringify([expression,p.nodeId,p.result?.run_dir]);
  const trialIdentityRef=useRef(trialIdentity);trialIdentityRef.current=trialIdentity;
  useEffect(()=>{trialSerial.current++;setTrial(null);setError('');setLoading(false);return()=>{trialSerial.current++;};},[trialIdentity]);
  useEffect(()=>{if(p.editing){setExpression(String(p.editing.config.expression||''));setTarget(String(p.editing.config.target||''));}},[p.editing?.id,JSON.stringify(p.editing?.config)]);
  async function preview(){const serial=++trialSerial.current;const identity=trialIdentity;setError('');setLoading(true);try{if(!p.result)throw Error('请先计算当前步骤，再局部试算。');const v=await invoke<Preview>('preview_formula',{batch:p.result?.run_dir,nodeId:p.nodeId,expression,limit:10});if(serial===trialSerial.current&&identity===trialIdentityRef.current){setTrialKey(identity);setTrial(v);}}catch(e){if(serial===trialSerial.current&&identity===trialIdentityRef.current)setError(String(e));}finally{if(serial===trialSerial.current&&identity===trialIdentityRef.current)setLoading(false);}}
  async function submit(){const identity=trialIdentity;setError('');setLoading(true);try{
    if(moving){const fields=p.fields.filter(f=>f!==moveField);if(!moveField||(['before','after'].includes(placement)&&(!anchor||anchor===moveField)))throw Error('请选择移动列及不同的位置列。');let index=placement==='first'?0:placement==='last'?fields.length:fields.indexOf(anchor)+(placement==='after'?1:0);if(index<0)throw Error('位置列不存在。');fields.splice(index,0,moveField);const success=await p.onAction({kind:'reorder',fields});if(success===false)throw Error('操作未完成；输入已保留，请查看状态栏原因。');setOpen(false);setMoving(false);return;}
    if(!target.trim())throw Error('请填写新列名。');
    await invoke('validate_formula',{project:p.project,nodeId:p.nodeId,expression,batch:p.result?.run_dir||null});if(identity!==trialIdentityRef.current)throw Error('公式或输入已变化，请重新确认。');
    if(p.editing){p.onConfig?.({expression,target});}else{const success=await p.onAction({kind:'derive',expression,target,placement,anchor});if(success===false)throw Error('操作未完成；输入已保留，请查看状态栏原因。');setOpen(false);}
  }catch(e){setError(String(e));}finally{setLoading(false);}}
  const refs=(p.selected||[]).map(f=>`[${f}]`);
  function insertOffset(){
    const n=Number(distance);
    if(!rowField||!rowSort||!Number.isInteger(n)||n<1||n>10000){setError('请选择取值列、排序列，并填写 1 至 10000 的行数。');return;}
    setExpression(current=>current+`${direction}([${rowField}],${n},[${rowSort}]${rowGroup?`,[${rowGroup}]`:''})`);
  }
  const form=<form className="operation-form" onSubmit={e=>{e.preventDefault();void submit();}}>
    <strong>{moving?'移动列':'公式计算'}</strong>
    {moving?<label>移动字段<select aria-label="移动字段" value={moveField} onChange={e=>setMoveField(e.target.value)} required><option value="">请选择</option>{p.fields.map(f=><option key={f}>{f}</option>)}</select></label>:<>
      <label>新列名<input aria-label="公式新列名" value={target} onChange={e=>setTarget(e.target.value)} required/></label>
      <label>公式<textarea aria-label="列公式" value={expression} onChange={e=>setExpression(e.target.value)} placeholder={'[公司] & "|" & [凭证编号]'} required/></label>
      <label>插入字段<select aria-label="插入公式字段" value="" onChange={e=>setExpression(expression+`[${e.target.value}]`)}><option value="">选择字段插入公式</option>{p.fields.map(f=><option key={f}>{f}</option>)}</select></label>
      <p>字段用[字段名]，文字用双引号。支持 &、四则运算、比较、IF、ISBLANK、LEFT、RIGHT、MID、LEN、FIND、TRIM、SUBSTITUTE、TEXTBEFORE、TEXTAFTER、ABS、ROUND、AND、OR、VALUE、TOTEXT、DATE、DATEVALUE、TEXT、YEAR、MONTH、DAY、DATEDIFF。位置从1起，缺标记／越界列入错误；金额超过四位小数不舍入。</p>
      {refs.length>=2&&<div><button type="button" onClick={()=>setExpression(refs.join(' + '))}>所选列相加</button><button type="button" onClick={()=>setExpression(refs.join(' - '))}>所选列相减</button><button type="button" onClick={()=>setExpression(refs.join(' & "-" & '))}>所选列拼接</button></div>}
      <details><summary>插入跨行取值</summary>
        <label>取值列<select value={rowField} onChange={e=>setRowField(e.target.value)}><option value="">请选择</option>{p.fields.map(f=><option key={f}>{f}</option>)}</select></label>
        <label>方向<select value={direction} onChange={e=>setDirection(e.target.value)}><option value="PREV">往前</option><option value="NEXT">往后</option></select></label>
        <label>相隔行数<input type="number" min="1" max="10000" value={distance} onChange={e=>setDistance(e.target.value)}/></label>
        <label>排序列<select value={rowSort} onChange={e=>setRowSort(e.target.value)}><option value="">请选择</option>{p.fields.map(f=><option key={f}>{f}</option>)}</select></label>
        <label>分组列<select value={rowGroup} onChange={e=>setRowGroup(e.target.value)}><option value="">不分组</option>{p.fields.map(f=><option key={f}>{f}</option>)}</select></label>
        <p>按排序列升序，空值排最后；同值按上游稳定顺序。组内找不到目标行时留空并标记。</p>
        <button type="button" onClick={insertOffset}>插入跨行引用</button>
      </details>
      <label>函数模板<select aria-label="函数分类与模板" value="" onChange={event=>setExpression(expression+event.target.value)}>
        <option value="">选择函数插入并填写参数</option>
        <optgroup label="数学"><option value="ABS([金额])">绝对值</option><option value="ROUND([金额],2)">明确舍入到两位</option></optgroup>
        <optgroup label="文本"><option value='LEFT([文本],2)'>左侧提取</option><option value='TEXTAFTER([文本],"/")'>分隔符后提取</option><option value='SUBSTITUTE([文本],"原文","新文")'>替换</option></optgroup>
        <optgroup label="逻辑"><option value='IF(ISBLANK([字段]),"空","非空")'>空值判断</option><option value="AND([金额]>0,[金额]<100)">同时满足</option><option value="OR([金额]>0,[金额]<0)">任一满足</option></optgroup>
        <optgroup label="日期"><option value='DATEVALUE([日期文本],"yyyy-mm-dd")'>指定格式转日期</option><option value='TEXT([日期],"yyyymmdd")'>日期格式化</option><option value="YEAR([日期])">提取年</option><option value="MONTH([日期])">提取月</option><option value="DAY([日期])">提取日</option><option value="DATEDIFF([开始日期],[结束日期])">相差天数</option></optgroup>
        <optgroup label="类型"><option value="VALUE([数字文本])">严格转数值</option><option value="TOTEXT([字段])">转文本</option></optgroup>
      </select></label>
      <p>DATEVALUE必须明确日期格式，TEXT用于日期格式化。DATEDIFF为结束日期减开始日期的天数。ROUND的小数位数限0至4。VALUE保留四位精度，前导零编码应保留文本。</p>
      <button type="button" disabled={loading||!p.result} onClick={()=>void preview()}>试算前10行</button>
      {trial&&trialKey===trialIdentity&&<div className="trial-result"><strong>局部试算：{trial.rows.length} 行，不代表全量通过</strong><table><thead><tr>{trial.columns.slice(-2).map(c=><th key={c}>{c}</th>)}</tr></thead><tbody>{trial.rows.map((r,i)=><tr key={i}>{r.slice(-2).map((v,j)=><td key={j}>{v}</td>)}</tr>)}</tbody></table></div>}
    </>}
    {!p.editing&&<><label>位置<select aria-label="列插入位置" value={placement} onChange={e=>setPlacement(e.target.value as typeof placement)}><option value="first">最前</option><option value="last">最后</option><option value="before">指定列之前</option><option value="after">指定列之后</option></select></label>{['before','after'].includes(placement)&&<label>位置列<select aria-label="位置列" value={anchor} onChange={e=>setAnchor(e.target.value)} required><option value="">请选择</option>{p.fields.map(f=><option key={f}>{f}</option>)}</select></label>}</>}
    {error&&<p className="inline-error" role="alert">{error}</p>}
    <button type="submit" className="primary" disabled={p.busy||loading}>{p.editing?'应用规则':'应用并计算'}</button>{!p.editing&&<button type="button" onClick={()=>setOpen(false)}>取消</button>}
  </form>;
  if(p.editing)return form;
  return <><button disabled={p.busy} onClick={()=>{setMoving(false);if(!expression&&refs.length===1)setExpression(refs[0]);if(!rowField&&p.selected?.length)setRowField(p.selected[0]);setOpen(true);}}>公式计算</button><button disabled={p.busy||p.fields.length<2} onClick={()=>{setMoving(true);setOpen(true);}}>移动列</button>{open&&<div className="modal-backdrop"><section className="column-action-dialog" role="dialog" aria-label={moving?'移动列':'公式计算'}>{form}</section></div>}</>;
}
