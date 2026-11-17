import { useEffect,useRef, useState } from 'react';
import {invoke} from '@tauri-apps/api/core';
import type { ReactNode } from 'react';
import type { PreviewQuery, PreviewState } from '../preview';
import { businessColumns, type Condition, type Review, type TableAction, type WorkNode, type RunResult, type ProjectFile,type Preview } from '../project';
import ColumnActionPanel from './ColumnActionPanel';
import ResultSummaryPanel from './ResultSummaryPanel';
import FormulaColumnPanel from './FormulaColumnPanel';
import RowOperationsPanel from './RowOperationsPanel';
import ResultGrid from './ResultGrid';
import {PivotMembers,type PivotSelection} from './PivotPanel';
import ExportPanel from './ExportPanel';
import AggregatePanel from './AggregatePanel';

type Props = {
  state: PreviewState; query: PreviewQuery; node: WorkNode;
  links: {id:string;label:string}[]; reviews: Review[];
  change: (query: PreviewQuery) => void; select: (id:string) => void; close: () => void;
  rawRow: (row:number) => void; headerRows: number[]; dataStart?:number;
  focus: {row:number|null;col:number|null}|null;
  busy?:boolean;canAct?:boolean;onAction?:(action:TableAction)=>void|Promise<boolean>;
  result?:RunResult|null;project?:ProjectFile;
  fieldTypes?:Record<string,string>;
  process?:()=>void;exportRows?:()=>void;exportReady?:boolean;exportEnabled?:boolean;
  expanded?:boolean;toggleExpanded?:()=>void;
  pageOffset?: number; onPage?: (offset: number) => void;
  toolbarSupplement?: ReactNode;
  loadFilterValues?: (batch: string, nodeId: string, field: string, search: string, offset: number,filterConfig?:Record<string,unknown>) => Promise<(string | null)[]>;
  loadRawWindow?:(row:number,column:number,tail:boolean)=>void;
};
export default function NodePreviewPanel(p: Props) {
  const s=p.state;
  const data=s.status==='ready'?s.data:undefined;
  const isRaw=data?.kind==='raw';
  const fields=businessColumns(data?.columns || []);
  const [chosen,setChosen]=useState<string[]>(fields);
  const [showOrigin,setShowOrigin]=useState(false);
  const [windowRow,setWindowRow]=useState(1);
  const [windowColumn,setWindowColumn]=useState(1);
  const [originField,setOriginField]=useState('');
  const [originTarget,setOriginTarget]=useState('');
  const [rename,setRename]=useState<{field:string;name:string}|null>(null);
  const [filter,setFilter]=useState<Condition|null>(null);
  const [valueSearch, setValueSearch] = useState('');
  const [valueOffset, setValueOffset] = useState(0);
  const [valueOptions, setValueOptions] = useState<(string | null)[]>([]);
  const [pickedValues, setPickedValues] = useState<(string | null)[]>([]);
  const [valuesStatus, setValuesStatus] = useState('');
  const [localError,setLocalError]=useState('');
  const editableFilter=p.node.kind==='filter'&&(Array.isArray(p.node.config.conditions)||Boolean(p.node.config.condition));
  const currentConditions=Array.isArray(p.node.config.conditions)?p.node.config.conditions as Condition[]:
    p.node.config.condition?[p.node.config.condition as Condition]:[];
  const valueSource=editableFilter?p.project?.workflow.edges.find(edge=>edge.target===p.node.id)?.source:s.sourceId;
  const filterScope=editableFilter?{match:p.node.config.match||'all',conditions:currentConditions}:undefined;
  const filterScopeKey=JSON.stringify(filterScope);
  const [groupValues,setGroupValues]=useState<Record<string,string|null>|null>(null);
  const [pivotSelection,setPivotSelection]=useState<PivotSelection|null>(null);
  const [selectedRecords,setSelectedRecords]=useState<number[]>([]);
  const recordIdentity=JSON.stringify([s.target.session,s.target.revision,s.batch,s.sourceId]);
  const recordIdentityRef=useRef(recordIdentity);recordIdentityRef.current=recordIdentity;
  useEffect(()=>{setSelectedRecords([]);},[p.node.id,s.batch,s.sourceId,s.target.revision,p.query.category,p.query.view,p.query.inputEdgeId]);
  useEffect(()=>{setPivotSelection(null);},[p.node.id,s.batch,s.target.revision,p.query.category]);
  useEffect(()=>{setGroupValues(null);},[p.node.id,s.batch,s.target.revision,p.query.category]);
  useEffect(() => { setValueSearch(''); setValueOffset(0); setPickedValues(filter?.values||[]); }, [filter?.field, s.batch]);
  useEffect(() => {
    if (!filter?.field || !s.batch || !valueSource || !p.loadFilterValues) return;
    let active = true;
    setValueOptions([]);
    setValuesStatus('正在查找全量结果中的值…');
    p.loadFilterValues(s.batch, valueSource, filter.field, valueSearch, valueOffset,filterScope).then(values => {
      if (active) { setValueOptions(values); setValuesStatus(values.length ? '' : '没有符合搜索的值。'); }
    }).catch(error => { if (active) setValuesStatus(String(error)); });
    return () => { active = false; };
  }, [filter?.field, s.batch, valueSource, filterScopeKey,p.loadFilterValues, valueSearch, valueOffset]);
  const columnsKey=JSON.stringify(data?.columns || []);
  useEffect(()=>{setChosen(businessColumns(data?.columns || []));setRename(null);setFilter(null);setLocalError('');},[p.node.id,columnsKey]);
  const apply=async(action:TableAction)=>{setLocalError('');if(action.kind==='rename' && action.name===action.field){setRename(null);return true;}const success=await p.onAction?.(action);if(success===false){setLocalError('操作未完成；请根据状态栏的原因修正，输入已保留。');return false;}setRename(null);setFilter(null);return true;};
  const applyColumnFilter=(condition:Condition|null)=>{
    if(!filter)return;
    if(!editableFilter){
      if(condition)void apply(['include_values','exclude_values'].includes(condition.operator)
        ?{kind:'multi_filter',config:{match:'all',conditions:[condition]}}:{kind:'filter',condition});
      return;
    }
    const conditions=[...currentConditions.filter(item=>item.field!==filter.field),...(condition?[condition]:[])];
    const config={match:p.node.config.match||'all',conditions,...(!conditions.length?{cleared:true}:{}),
      ...(p.node.config.selected_fingerprints?{selected_fingerprints:p.node.config.selected_fingerprints}:{})};
    void apply({kind:'update_filter',config});
  };
  const visibleIndices=(data?.columns || []).map((_,i)=>i).filter(i=>showOrigin || (!data!.columns[i].startsWith('__') && data!.columns[i]!=='company') || data!.columns[i]==='__row');
  const view=(value:PreviewQuery['view'])=>p.change({...p.query,view:value,category:'normal'});
  const offset = p.pageOffset ?? 0;
  const renderHeader = (i: number) => {
    if (!data) return null;
    const c=data.columns[i];const editable=Boolean(p.canAct && fields.includes(c));return <div className="result-grid-header">{editable&&<input type="checkbox" aria-label={`选择列 ${c}`} checked={chosen.includes(c)} onChange={e=>setChosen(e.target.checked?[...chosen,c]:chosen.filter(f=>f!==c))} />}<span title={editable?'双击更名':undefined} onDoubleClick={()=>{if(editable){setFilter(null);setRename({field:c,name:c});}}}>{c==='__row'?'原始行':c || `未命名（第${i}列）`}</span>{editable&&<button className="column-filter" aria-label={`筛选 ${c}`} onClick={()=>{setRename(null);const existing=editableFilter?currentConditions.find(condition=>condition.field===c):undefined;setFilter(existing?{...existing,value:existing.value||''}:{field:c,operator:'equals',value:''});}}>筛选</button>}</div>;
  };
  const selectResultRow = (row: string[]) => {
    if (!data || p.node.kind !== 'aggregate' || p.node.version !== 2 || p.query.category !== 'normal') return;
    const keys = p.node.config.group_by as string[];
    setGroupValues(Object.fromEntries(keys.map((key, index) => [
      key, row[data.columns.indexOf('__group_null_' + index)] === 'true' ? null : row[data.columns.indexOf(key)],
    ])));
  };
  return <section id="node-preview-region" className="results" aria-label="当前步骤数据预览">
    <div className="results-head"><strong>{s.title}</strong><span>{data ? data.totalRows===null ? `仅展示 ${data.rows.length} 行；未统计总体` : `预览 ${data.rows.length} 行／全量 ${data.totalRows} 行` : '未展示数据'}</span><div className="results-head-actions">{p.toggleExpanded&&<button aria-expanded={Boolean(p.expanded)} onClick={p.toggleExpanded}>{p.expanded?'恢复布局':'展开表格'}</button>}<button onClick={p.close}>收起</button></div></div>
    <div className="preview-toolbar">
      {p.toolbarSupplement}
      {p.node.kind==='source'&&p.loadRawWindow&&<div className="input-action" aria-label="原始资料局部查看">
        <label>起始物理行<input type="number" min={1} value={windowRow} onChange={event=>setWindowRow(Number(event.target.value))}/></label>
        <label>起始列<input type="number" min={1} max={16384} value={windowColumn} onChange={event=>setWindowColumn(Number(event.target.value))}/></label>
        <button disabled={p.busy} onClick={()=>p.loadRawWindow?.(windowRow,windowColumn,false)}>查看附近20行</button>
        <button disabled={p.busy} onClick={()=>p.loadRawWindow?.(1,1,false)}>查看首部</button>
        <button disabled={p.busy} onClick={()=>p.loadRawWindow?.(1,windowColumn,true)}>扫描查看尾部</button>
      </div>}
      {p.node.kind==='source' ? <><button className={p.query.view==='raw'?'active':''} onClick={()=>view('raw')}>原始表格</button><button className={['auto','prepared'].includes(p.query.view)?'active':''} onClick={()=>view('prepared')}>整理效果</button></> : <button className={s.direction==='input'?'active':''} onClick={()=>view('input')}>本步输入</button>}
      <button className={s.direction==='output' && !['raw','prepared','auto'].includes(p.query.view)?'active':''} onClick={()=>view('output')}>本步输出</button>
      {p.process && <button disabled={p.busy} onClick={p.process}>计算本步</button>}
      {p.exportRows && <button className="primary" disabled={p.busy || isRaw || p.exportEnabled===false} onClick={p.exportRows}>{p.exportReady?'导出CSV':'运行并导出CSV'}</button>}
      {data?.kind==='run' && <label><input type="checkbox" checked={showOrigin} onChange={e=>setShowOrigin(e.target.checked)} />显示全部来源列</label>}
      {p.links.length>1 && <label>输入来源<select aria-label="预览输入来源" value={p.query.inputEdgeId || ''} onChange={e=>p.change({...p.query,view:'input',inputEdgeId:e.target.value || undefined,category:'normal'})}><option value="">请选择输入端口</option>{p.links.map(l=><option key={l.id} value={l.id}>{l.label}</option>)}</select></label>}
      {s.direction==='output' && p.reviews.some(r=>r.node_id===p.node.id) && <label>结果类别<select aria-label="预览结果类别" value={p.query.category} onChange={e=>p.change({...p.query,view:'output',category:e.target.value as PreviewQuery['category']})}><option value="normal">正常</option><option value="excluded">未纳入</option><option value="exceptions">异常</option>{p.node.kind==="join"&&p.node.version===2&&<><option value="matched">匹配</option><option value="left_only">仅左侧</option><option value="right_only">仅右侧</option></>}</select></label>}
    </div>
    {p.onAction && data && !isRaw && <div className="table-actions">
      {p.canAct ? <><span>勾选列头选列；双击列名更名；点列头“筛选”设置条件。</span><button onClick={()=>setChosen(fields)}>全选</button><button onClick={()=>setChosen([])}>清空选择</button><button disabled={p.busy || !chosen.length || chosen.length===fields.length} onClick={()=>apply({kind:'keep',fields:fields.filter(f=>chosen.includes(f))})}>保留所选 {chosen.length} 列</button></>
        : <span>查看本步输出后可直接操作表格；尚未生成时点击“计算本步”。</span>}
    </div>}
    {p.onAction && p.canAct && data && !isRaw && <ColumnActionPanel fields={fields} selected={fields.filter(f=>chosen.includes(f))} busy={p.busy} fieldTypes={p.fieldTypes} onAction={apply}/>}
    {p.onAction&&p.canAct&&data?.kind==='run'&&<div className="input-action"><label>添加来源业务列<select value={originField} onChange={event=>setOriginField(event.target.value)}><option value="">请选择来源</option>{data.columns.filter(name=>/^__(?:(?:left|right)__)?(?:file|sheet|row|record)$/.test(name)).map(name=><option key={name}>{name}</option>)}</select></label>
      <input aria-label="来源业务列名称" placeholder="例如 原始资料行号" value={originTarget} onChange={event=>setOriginTarget(event.target.value)}/>
      <button disabled={!originField||!originTarget||p.busy} onClick={()=>{void apply({kind:'source_fields',fields:[{source:originField,target:originTarget}]});}}>新增来源列</button>
    </div>}
    {p.onAction&&p.canAct&&data?.kind==='run'&&<div className="input-action">
      <span>已选 {selectedRecords.length} 条记录，跨分块保留。</span>
      {[false,true].map(exclude=><button key={String(exclude)} disabled={!selectedRecords.length||selectedRecords.length>1000||p.busy} onClick={()=>{
        const identity=recordIdentity;
        void invoke<Record<string,unknown>>('selected_record_filter',{batch:s.batch,nodeId:s.sourceId,records:selectedRecords,exclude})
          .then(config=>{if(recordIdentityRef.current===identity)void apply({kind:'multi_filter',config});})
          .catch(error=>{if(recordIdentityRef.current===identity)setLocalError(String(error));});
      }}>{exclude?'排除':'保留'}选中来源记录</button>)}
      <span>单次最多1000条，来源变化后须重新选择。</span>
    </div>}
    {p.onAction && p.canAct && data && !isRaw && <div className="extended-actions">
      {p.project&&<FormulaColumnPanel fields={fields} selected={fields.filter(f=>chosen.includes(f))} project={p.project} nodeId={p.node.id} result={p.result||null} busy={p.busy} onAction={p.onAction}/>}
      <RowOperationsPanel fields={fields} result={p.result} nodeId={s.sourceId} busy={p.busy} onAction={p.onAction}/>
      <AggregatePanel key={JSON.stringify([p.node.id,s.batch,groupValues])} fields={fields} busy={p.busy} onAction={p.onAction} result={p.result} nodeId={p.node.id} groupValues={groupValues}/>
    </div>}
    {p.node.kind==='pivot'?<p>总计行和各行合计按对应原始明细重新计算。点击统计格查看成员。</p>:<ResultSummaryPanel state={s} result={p.result||null} fieldTypes={p.fieldTypes||{}} busy={p.busy}/>}
    {p.node.kind==='pivot'&&p.result&&pivotSelection&&<PivotMembers node={p.node} result={p.result} selection={pivotSelection}/>}
    {p.node.kind==='aggregate'&&p.node.version===2&&s.direction==='output'&&p.query.category==='normal'&&<p>点击汇总行查看组成明细及全量成员数。</p>}
    {rename && <form className="table-inline-form" onSubmit={e=>{e.preventDefault();if(!rename.name.trim()){setLocalError('请填写列名。');return;}apply({kind:'rename',...rename});}}><label>将「{rename.field}」更名为<input aria-label="新列名" value={rename.name} onChange={e=>setRename({...rename,name:e.target.value})} autoFocus /></label><button className="primary" type="submit">应用更名</button><button type="button" onClick={()=>setRename(null)}>取消</button></form>}
    {filter && <form className="table-inline-form" onSubmit={event=>{
      event.preventDefault();
      const chosen=['include_values','exclude_values'].includes(filter.operator);
      if(chosen&&!pickedValues.length){setLocalError('请选择至少一个值。');return;}
      if(!chosen&&!['is_empty','is_not_empty'].includes(filter.operator)&&!filter.value.trim()){setLocalError('请填写筛选值。');return;}
      applyColumnFilter(chosen?{...filter,values:pickedValues}:filter);
    }}><strong>筛选 {filter.field}</strong>
      {editableFilter&&<span>替换此步骤的本列条件，其余列条件保留。</span>}
      <select aria-label="列头筛选条件" value={filter.operator} onChange={event=>setFilter({...filter,operator:event.target.value})}>
        <option value="equals">文本等于</option><option value="not_equals">文本不等于</option><option value="contains">包含</option>
        <option value="is_empty">为空</option><option value="is_not_empty">不为空</option><option value="include_values">包含所选值</option><option value="exclude_values">排除所选值</option>
        <option value="numeric_gt">数值大于</option><option value="numeric_gte">数值大于等于</option><option value="numeric_lt">数值小于</option><option value="numeric_lte">数值小于等于</option><option value="numeric_eq">数值等于</option><option value="numeric_neq">数值不等于</option>
        <option value="date_eq">日期等于</option><option value="date_gt">日期晚于</option><option value="date_lt">日期早于</option>
      </select>
      {!['is_empty','is_not_empty','include_values','exclude_values'].includes(filter.operator)&&<input aria-label="列头筛选值" value={filter.value} onChange={event=>setFilter({...filter,value:event.target.value})} autoFocus/>}
      <button className="primary" type="submit">应用筛选</button>
      {editableFilter&&<button type="button" onClick={()=>applyColumnFilter(null)}>清除此步骤本列条件</button>}
      <button type="button" onClick={()=>setFilter(null)}>取消</button>
    </form>}
    {localError && <div className="inline-error" role="alert">{localError}</div>}
    {filter && s.batch && p.loadFilterValues && <div className="table-inline-form" aria-label="全量列值筛选">
      <label>搜索全量列值 <input value={valueSearch} onChange={event=>{setValueSearch(event.target.value);setValueOffset(0);}} /></label>
      <span role="status">{valuesStatus}</span>
      <div className="filter-value-list">{valueOptions.slice(0,200).map(value=><label key={JSON.stringify(value)}>
        <input type="checkbox" checked={pickedValues.includes(value)} onChange={event=>setPickedValues(previous=>event.target.checked?[...previous,value]:previous.filter(item=>item!==value))} />
        {value === null ? '空值（NULL）' : value === '' ? '空文本' : value}
      </label>)}</div>
      <button disabled={!valueOffset} onClick={()=>setValueOffset(Math.max(0,valueOffset-200))}>上一组值</button>
      <button disabled={valueOptions.length<=200} onClick={()=>setValueOffset(valueOffset+200)}>下一组值</button>
      <span>已选 {pickedValues.length} 个值</span>
      {(['include_values','exclude_values'] as const).map(operator=><button key={operator} disabled={!pickedValues.length||p.busy} onClick={()=>{
        applyColumnFilter({field:filter.field,operator,values:pickedValues,value:''});
      }}>{operator==='include_values'?'保留所选值':'排除所选值'}</button>)}
    </div>}
    <div className="preview-provenance">{s.message}{s.batch && <span title={s.batch}> · 批次：{s.batch.split(/[\\/]/).pop()}</span>}</div>
    {data?.kind==='run'&&s.batch&&s.sourceId&&<ExportPanel key={JSON.stringify([s.batch,s.sourceId,p.query.category])} batch={s.batch} nodeId={s.sourceId} category={s.category||p.query.category} columns={data.columns} records={selectedRecords} busy={p.busy}/>}
    {p.reviews.length>0 && <div className="result-steps">{p.reviews.map(r=><button key={r.node_id} className={r.node_id===p.node.id?'active':''} onClick={()=>p.select(r.node_id)}>{r.label} <small>{r.rows} 行</small></button>)}</div>}
    {data ? data.kind === 'run' ? <ResultGrid indices={visibleIndices} columns={data.columns} rows={data.rows}
      renderHeader={renderHeader} selectRow={selectResultRow} selectCell={(row,column)=>{
        if(p.node.kind!=='pivot'||!column.includes('］_'))return;
        const keys=p.node.config.row_by as string[];
        setPivotSelection({column,totalRow:row[data.columns.indexOf('__pivot_total')]==='true',
          groupValues:Object.fromEntries(keys.map((key,index)=>[key,row[data.columns.indexOf('__group_null_'+index)]==='true'?null:row[data.columns.indexOf(key)]]))});
      }} offset={offset} selectedRecords={selectedRecords} selectRecord={(record,selected)=>setSelectedRecords(previous=>selected?previous.includes(record)?previous:[...previous,record]:previous.filter(value=>value!==record))}
        identity={JSON.stringify([s.target.session,s.target.revision,s.batch,s.sourceId,s.category])} totalRows={data.totalRows||0}
        loadBlock={async(offset,limit)=>{
          const page=await invoke<Preview>('preview_table',{batch:s.batch,nodeId:s.sourceId,category:s.category||p.query.category,offset,limit});
          if(JSON.stringify(page.columns)!==JSON.stringify(data.columns))throw Error('结果列已变化，请重新计算。');
          return page.rows;
        }}/>
      : <div className="result-scroll"><table><thead><tr>{visibleIndices.map(i=><th key={i}>{renderHeader(i)}</th>)}</tr></thead><tbody>{data.rows.map((row,i)=><tr key={i} data-physical-row={isRaw?row[0]:undefined} className={isRaw && p.headerRows.includes(Number(row[0]))?'header-candidate':isRaw && p.dataStart===Number(row[0])?'data-candidate':''} onClick={()=>{if(isRaw)p.rawRow(Number(row[0]));else if(p.node.kind==='aggregate'&&p.node.version===2&&p.query.category==='normal'){const keys=p.node.config.group_by as string[];setGroupValues(Object.fromEntries(keys.map((k,j)=>[k,row[data.columns.indexOf('__group_null_'+j)]==='true'?null:row[data.columns.indexOf(k)]])));}}}>{visibleIndices.map(j=><td key={j} className={isRaw && p.focus && ((p.focus.row===Number(row[0]) && !p.focus.col) || (p.focus.col && data.columns[j]===`第${p.focus.col}列`))?'reading-focus-cell':undefined}>{row[j]}</td>)}</tr>)}</tbody></table>{data.rows.length===0 && <p className="result-empty">当前结果为0行。</p>}</div>
      : <p className="result-empty" role={s.status==='error'?'alert':'status'}>{s.status==='loading'?'正在载入当前步骤数据…':s.message}</p>}
  </section>;
}

