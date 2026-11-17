import {useEffect,useState} from 'react';
import type {Mapping,TableAction,WorkNode} from '../project';

type Operation='decimal'|'trim'|'replace'|'text_extract'|'numeric';
type Props={fields:string[];selected:string[];busy?:boolean;fieldTypes?:Record<string,string>;onAction?:(action:TableAction)=>void|Promise<boolean>;editing?:WorkNode;onConfig?:(config:Record<string,unknown>)=>void};
export const isNumericType=(type:string|undefined)=>Boolean(type && /^(DECIMAL|DOUBLE|FLOAT|REAL|TINYINT|SMALLINT|INTEGER|BIGINT|HUGEINT|UINTEGER|UBIGINT)\b/i.test(type));
const titles:Record<Operation,string>={decimal:'转换为金额',trim:'去首尾空格',replace:'普通文本替换',text_extract:'文本提取',numeric:'数值筛选'};
export default function ColumnActionPanel(p:Props){
  const [operation,setOperation]=useState<Operation|null>(null);
  const [mappings,setMappings]=useState<Mapping[]>([]);
  const [source,setSource]=useState(''),[target,setTarget]=useState('');
  const [find,setFind]=useState(''),[replacement,setReplacement]=useState(''),[pattern,setPattern]=useState(''),[group,setGroup]=useState(1);
  const [operator,setOperator]=useState('numeric_gt'),[value,setValue]=useState(''),[error,setError]=useState('');
  const numeric=p.fields.filter(f=>isNumericType(p.fieldTypes?.[f]));
  const begin=(op:Operation,existing=p.editing)=>{
    const cfg=existing?.config;
    const chosen=p.selected.filter(f=>p.fields.includes(f));
    const first=chosen[0] || p.fields[0] || '';
    setOperation(op);setError('');
    const suffix=op==='decimal'?'金额':op==='trim'?'去空格':'替换';
    setMappings((cfg?.fields as Mapping[]|undefined) || chosen.map(source=>({source,target:`${source}_${suffix}`})));
    setSource(String(cfg?.source || (cfg?.condition as {field?:string}|undefined)?.field || (op==='numeric'?numeric.find(f=>chosen.includes(f))||numeric[0]||'':first)));
    setTarget(String(cfg?.target || (op==='text_extract'?`${first}_提取`:'差额')));
    setFind(String(cfg?.find || ''));setReplacement(String(cfg?.replacement || ''));setPattern(String(cfg?.pattern || ''));setGroup(Number(cfg?.group ?? 1));
    setOperator(String((cfg?.condition as {operator?:string}|undefined)?.operator || 'numeric_gt'));setValue(String((cfg?.condition as {value?:string}|undefined)?.value || ''));
  };
  useEffect(()=>{
    if(p.editing){const kind=p.editing.kind;begin(kind==='text_clean'?(p.editing.config.mode==='replace'?'replace':'trim'):kind==='filter'?'numeric':kind as Operation,p.editing);}
    else setOperation(null);
  },[p.editing?.id,JSON.stringify(p.editing?.config),p.fields.join('\0')]);
  const submit=async()=>{
    if(!operation)return;
    if(['decimal','trim','replace'].includes(operation)&&!mappings.length){setError('请先选择要处理的列。');return;}
    if(operation==='numeric'&&!numeric.includes(source)){setError('请先将所选列转换为金额，只有已确认的数值列可用于此操作。');return;}
    let action:TableAction;
    if(operation==='decimal')action={kind:'decimal',fields:mappings};
    else if(operation==='trim'||operation==='replace')action={kind:'text_clean',fields:mappings,mode:operation,find,replacement};
    else if(operation==='text_extract')action={kind:'text_extract',source,target,pattern,group};
    else action={kind:'filter',condition:{field:source,operator,value}};
    if(p.editing && p.onConfig){const {kind,...config}=action;p.onConfig(config);}
    else {try{if(await p.onAction?.(action)===false){setError('操作未完成；输入已保留，请根据状态栏的具体原因修正。');return;}setOperation(null);}catch(e){setError(String(e));}}
  };
  const options=(list:string[])=>list.map(f=><option key={f} value={f}>{f}</option>);
  const form=operation&&<form className="column-action-form" onSubmit={e=>{e.preventDefault();submit();}}>
    <strong>{titles[operation]}</strong>
    {p.editing && ['trim','replace'].includes(operation) && <label>处理方式<select aria-label="文本清理方式" value={operation} onChange={e=>setOperation(e.target.value as 'trim'|'replace')}><option value="trim">去首尾空格</option><option value="replace">普通文本替换</option></select></label>}
    {['decimal','trim','replace'].includes(operation)&&<><p>新增结果列，原列保留。处理 {mappings.length} 列。</p>{p.editing&&<label>添加源列<select aria-label="添加处理列" value="" onChange={e=>{if(e.target.value&&!mappings.some(m=>m.source===e.target.value))setMappings([...mappings,{source:e.target.value,target:`${e.target.value}_${operation==='decimal'?'金额':'清理'}`}]);}}><option value="">选择列</option>{options(p.fields)}</select></label>}<div className="column-targets">{mappings.map((m,i)=><label key={i}>{m.source}<input aria-label={`${m.source} 的新列名`} required value={m.target} onChange={e=>setMappings(mappings.map((old,j)=>j===i?{...old,target:e.target.value}:old))}/>{operation==='decimal'&&(!p.editing||p.editing.version===2)&&<span><input type="checkbox" aria-label={`${m.source} 空白按零`} checked={m.empty_as_zero===true} onChange={e=>setMappings(mappings.map((old,j)=>j===i?{...old,empty_as_zero:e.target.checked}:old))}/> 空白按零</span>}{p.editing&&<button type="button" onClick={()=>setMappings(mappings.filter((_,j)=>j!==i))}>移除</button>}</label>)}</div></>}
    {operation==='decimal'&&(!p.editing||p.editing.version===2)&&mappings.map((mapping,index)=><fieldset key={mapping.source}><legend>{mapping.source} 的金额格式</legend>
      {([['trim_special_space','识别首尾特殊空白'],['thousands','识别逗号千分位'],['parentheses_negative','识别括号负数']] as const).map(([name,label])=><label key={name}><input type="checkbox" checked={mapping[name]===true} onChange={event=>setMappings(previous=>previous.map((item,i)=>i===index?{...item,[name]:event.target.checked}:item))}/>{label}</label>)}
      <label>本列币种前缀<select value={mapping.currency_prefix||''} onChange={event=>setMappings(previous=>previous.map((item,i)=>i===index?{...item,currency_prefix:event.target.value}:item))}>
        <option value="">无前缀处理</option>{['¥','￥','CNY','RMB','USD','$','EUR','€','HKD','HK$'].map(prefix=><option key={prefix}>{prefix}</option>)}
      </select></label><p>仅识别所选前缀，不做汇率换算。千分位分组不合法时保留待确认。</p>
    </fieldset>)}
    {operation==='decimal'&&<p>精度为28位、4位小数。新步骤保留整行，有效金额按列参与合计。空白默认保留，可逐列选择空白按零。旧步骤仍按保存时的规则处理。</p>}
    {operation==='trim'&&<p>仅清除首尾的半角空格、Tab及换行，不清除中间字符或全角空格。</p>}
    {operation==='replace'&&<><label>查找文本<input aria-label="查找文本" required value={find} onChange={e=>setFind(e.target.value)}/></label><label>替换为<input aria-label="替换为" value={replacement} onChange={e=>setReplacement(e.target.value)}/></label><p>按普通文本替换全部匹配内容，区分大小写；替换为空表示删除匹配文本。</p></>}
    {['text_extract','numeric'].includes(operation)&&<label>源列<select aria-label="操作源列" value={source} onChange={e=>setSource(e.target.value)} required><option value="">请选择</option>{options(operation==='text_extract'?p.fields:numeric)}</select></label>}
    {['text_extract'].includes(operation)&&<label>新列名<input aria-label="操作新列名" required value={target} onChange={e=>setTarget(e.target.value)}/></label>}
    {operation==='text_extract'&&<><label>提取规则<input aria-label="提取规则" required value={pattern} onChange={e=>setPattern(e.target.value)} placeholder="例如：([0-9]+)"/></label><label>分组序号<input aria-label="分组序号" type="number" min="0" step="1" required value={group} onChange={e=>setGroup(Number(e.target.value))}/></label><p>0表示完整匹配，1表示第一组；未提取记录可在异常中复核。</p></>}
    {operation==='numeric'&&<><label>条件<select aria-label="数值比较条件" value={operator} onChange={e=>setOperator(e.target.value)}>{[['numeric_gt','大于'],['numeric_gte','大于等于'],['numeric_lt','小于'],['numeric_lte','小于等于'],['numeric_eq','数值等于'],['numeric_neq','数值不等于']].map(([v,l])=><option key={v} value={v}>{l}</option>)}</select></label><label>比较值<input aria-label="数值比较值" required value={value} onChange={e=>setValue(e.target.value)}/></label></>}
    {['numeric'].includes(operation)&&<p>仅使用后端已确认的数值列；文本金额请先转换，不根据预览猜类型。</p>}
    {error&&<p role="alert" className="inline-error">{error}</p>}
    <div className="column-action-buttons"><button type="submit" className="primary" disabled={p.busy}>{p.editing?'应用规则':'应用并计算'}</button>{!p.editing&&<button type="button" onClick={()=>setOperation(null)}>取消</button>}</div>
  </form>;
  if(p.editing)return <div className="column-action-editor">{form}</div>;
  return <><div className="column-actions" aria-label="选列处理">{(Object.keys(titles) as Operation[]).map(op=><button key={op} disabled={p.busy || !p.selected.length} onClick={()=>begin(op,undefined)}>{titles[op]}</button>)}</div>{operation&&<div className="modal-backdrop"><section className="column-action-dialog" role="dialog" aria-modal="true" aria-label={titles[operation]}>{form}</section></div>}</>;
}
