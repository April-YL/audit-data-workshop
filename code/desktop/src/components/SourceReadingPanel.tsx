import { useState } from 'react';
import type { ReadingIssue, StructureReport, TableLayout } from '../project';

type Props = {
  report: StructureReport; draft?: TableLayout; checked: boolean; applied: boolean;
  pending: boolean; differences: ReadingIssue[];
  previewView: 'auto' | 'raw' | 'prepared' | 'input' | 'output';
  update: (change: Partial<TableLayout>) => void;
  select: (index: number) => void; locate: (issue: ReadingIssue) => void;
  view: (value: 'raw' | 'prepared') => void;
  pick: (value: 'header_start' | 'header_end' | 'data_start') => void;
  check: () => void; apply: () => void; discard: () => void;
};

export default function SourceReadingPanel(p: Props) {
  const [editingCol, setEditingCol] = useState<number | null>(null);
  const c = p.report.active;
  const problems = [...(c?.diagnostics || []), ...p.differences];
  return <section className="structure-panel reading-panel">
    <strong>{p.pending ? '新资料待确认' : p.applied ? '正在使用的读取方案' : '请核实读取方式'}</strong>
    {p.pending && <><p className="inline-error">新资料尚未替换项目输入，确认后才使用。</p><button className="secondary-action" onClick={p.discard}>放弃替换，返回原资料</button></>}
    <p>工作表：{p.report.raw.sheet}</p>
    {p.draft && <p className="reading-summary">第 {p.draft.header_rows.join('、')} 行作表头，第 {p.draft.data_start_row} 行起读取明细；读取第 {p.draft.first_col}—{p.draft.last_col} 列{p.draft.data_end_row ? `，截至第 ${p.draft.data_end_row} 行` : '，直到表格末尾'}。</p>}
    <div className="preview-switch"><button aria-pressed={p.previewView==='raw'} onClick={() => p.view('raw')}>查看原始表格</button><button aria-pressed={p.previewView==='auto' || p.previewView==='prepared'} onClick={() => p.view('prepared')}>查看整理效果</button></div>
    <p className="structure-notice">局部检查仅依据前 {p.report.raw.rows.length} 行；全量运行仍检查后续范围外列。不会自动剔除合计或填补普通空白。</p>
    {problems.length > 0 ? <div className="reading-issues"><strong>{problems.length} 处需要核实</strong>{problems.map((issue, i) => <div key={`${issue.code}-${issue.col}-${i}`} className="reading-issue">
      <button onClick={() => { p.locate(issue); if (issue.col) setEditingCol(issue.col); }}>{issue.row ? `第${issue.row}行 ` : ''}{issue.col ? `第${issue.col}列：` : ''}{issue.message}</button>
      {issue.code === 'outside_columns' && issue.col && p.draft && <button onClick={()=>p.update({first_col:Math.min(p.draft!.first_col,issue.col!),last_col:Math.max(p.draft!.last_col,issue.col!)})}>将第{issue.col}列纳入读取，再检查</button>}
      {issue.col && editingCol === issue.col && ['unnamed','duplicate','invalid_name'].includes(issue.code) && p.draft && <label>此列输出名称<input aria-label={`修正第${issue.col}列名称`} value={p.draft.field_names?.[String(issue.col)] ?? c?.fields.find(f => f.physical_col === issue.col)?.name ?? ''} onChange={e => p.update({ field_names: { ...p.draft?.field_names, [String(issue.col)]: e.target.value } })} /></label>}
    </div>)}</div> : <p>局部结构{p.differences.length ? '待核实' : '未发现阻断问题'}。</p>}
    {p.pending && <p className="inline-error">当前展示的是待替换资料。使用方案后才替换项目输入；也可放弃并返回原资料。</p>}
    <details><summary>调整读取方式</summary>
      {p.report.candidates.length > 0 && <div className="candidate-list">{p.report.candidates.map((item, i) => <button key={i} onClick={() => p.select(i)}>建议：第 {item.layout.header_rows.join('、')} 行表头，第 {item.layout.data_start_row} 行起明细</button>)}</div>}
      {p.draft && <>
        <div className="row-pickers"><button onClick={() => p.pick('header_start')}>点选表头首行</button><button onClick={() => p.pick('header_end')}>点选表头末行</button><button onClick={() => p.pick('data_start')}>点选明细首行</button></div>
        <div className="layout-numbers"><label>起始列<input type="number" min="1" value={p.draft.first_col} onChange={e => p.update({first_col:Number(e.target.value),field_names:{}})} /></label><label>结束列<input type="number" min="1" value={p.draft.last_col} onChange={e => p.update({last_col:Number(e.target.value),field_names:{}})} /></label><label>明细结束行（空为到底）<input type="number" value={p.draft.data_end_row ?? ''} onChange={e => p.update({data_end_row:e.target.value ? Number(e.target.value) : undefined})} /></label></div>
        <label className="range-consent"><input type="checkbox" checked={p.draft.strict_columns === false} onChange={e => p.update({strict_columns:!e.target.checked})} />已核实并明确排除范围外列</label>
        <details><summary>查看或修改全部字段名称</summary>{c?.fields.map(f => <label key={f.physical_col}>第{f.physical_col}列 · {f.parts.join(' / ') || '无表头'}<input value={p.draft?.field_names?.[String(f.physical_col)] ?? f.name} onChange={e => p.update({field_names:{...p.draft?.field_names,[String(f.physical_col)]:e.target.value}})} /></label>)}</details>
      </>}
    </details>
    {!p.checked && <p role="status">读取设置已改变，显示的整理效果尚未更新。</p>}
    <button className="secondary-action" onClick={p.check}>{p.differences.length ? '核实差异后重新检查' : '更新整理效果'}</button>
    <button className="primary apply-structure" disabled={!p.checked || !c?.ready || p.differences.length > 0} onClick={p.apply}>使用此读取方案</button>
    {!p.pending && !p.applied && <button className="secondary-action" onClick={p.discard}>放弃本次修改，恢复已确认读取方式</button>}
    {p.report.notices.filter(n => !n.startsWith('仅依据') && !n.startsWith('未自动')).map((n,i) => <p className="structure-notice" key={i}>{n}</p>)}
  </section>;
}
