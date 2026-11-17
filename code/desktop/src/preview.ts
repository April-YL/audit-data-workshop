import { incomingLinks } from './project';
import type { ProjectFile, RawSourceSample, RunResult, SourceSample } from './project';

export type PreviewView = 'auto' | 'raw' | 'prepared' | 'input' | 'output';
export type FieldDescriptor = {name:string;data_type:string;nullable:boolean;origin?:unknown;confirmed:boolean};
export type NodeContract = {outputs:Record<string,FieldDescriptor[]>;diagnostics:{node_id:string;parameter:string;code:string;message:string}[];exception_mode:string};
export type ResultCategory = 'normal' | 'excluded' | 'exceptions' | 'matched' | 'left_only' | 'right_only';
export type ResultSummary = {batch:string;node_id:string;category:string;rows:number;fields:{name:string;data_type:string;non_null_count:number;null_count:number;sum:string|null;amount_counts?:{blank:number;failed:number;filled_zero:number}|null}[]};
export const resultRequestKey=(target:PreviewTarget,batch:string,fields:string[])=>JSON.stringify([target.session,target.revision,target.nodeId,target.view,target.category,target.inputEdgeId,batch,fields]);
export type PreviewQuery = { nodeId: string; view: PreviewView; category: ResultCategory; inputEdgeId?: string };
export type PreviewTarget = PreviewQuery & { session: string; revision: number; request: number; offset?: number };
export type PreviewData = { columns: string[]; rows: string[][]; kind: 'raw' | 'sample' | 'run'; totalRows: number | null };
export type PreviewPlan = {
  status: 'pending' | 'loading' | 'ready' | 'stale' | 'error' | 'unsupported' | 'table';
  title: string; message: string; direction: 'raw' | 'input' | 'output'; sourceId?: string;
  data?: PreviewData; database?: string; table?: string; batch?: string;
  category?: ResultCategory;
};
export type PreviewState = PreviewPlan & { target: PreviewTarget };
export type SourceEvidence = {
  raw?: RawSourceSample; prepared?: SourceSample; preparedFresh: boolean;
  applied?: SourceSample; appliedKey?: string; confirmed: boolean; loading?: boolean; error?: string;
};
export type PreviewContext = { project: ProjectFile; sources: Record<string, SourceEvidence>; run: RunResult | null };

// Positions and labels are presentation, not calculation rules.
export function dataSignature(project: ProjectFile): string {
  return JSON.stringify({ company: project.company_name, bindings: project.bindings,
    nodes: project.workflow.nodes.map(({ id, kind, version, config }) => ({ id, kind, version, config })), edges: project.workflow.edges });
}
export function sourceIdentity(project: ProjectFile, nodeId: string): string {
  const node = project.workflow.nodes.find(n => n.id === nodeId);
  return JSON.stringify({ binding: project.bindings[nodeId], config: node?.config });
}
export function acceptsResponse(current: PreviewTarget, received: PreviewTarget): boolean {
  return current.session === received.session && current.revision === received.revision && current.request === received.request
    && current.nodeId === received.nodeId && current.view === received.view && current.category === received.category && current.inputEdgeId === received.inputEdgeId && current.offset === received.offset;
}
function sampled(sample: SourceSample | RawSourceSample): PreviewData {
  return { columns: ['原始行', ...('headers' in sample ? sample.headers : sample.columns)],
    rows: sample.rows.map((row, i) => [String(sample.physical_rows[i]), ...row]),
    kind: 'headers' in sample ? 'sample' : 'raw', totalRows: null };
}

// Produces a display plan only; all transformations and result-table reads stay in Rust.
export function resolvePreview(c: PreviewContext, q: PreviewQuery): PreviewPlan {
  const node = c.project.workflow.nodes.find(n => n.id === q.nodeId);
  const direction = q.view === 'raw' ? 'raw' : q.view === 'input' ? 'input' : 'output';
  const title = `${node?.label || '步骤'} · ${direction === 'raw' ? '原始表格' : direction === 'input' ? '本步输入' : '本步输出'}`;
  const base: PreviewPlan = { status: 'pending', title, direction, message: '本步输出尚未生成，请运行后查看。' };
  if (!node) return {...base, message:'请选择有效步骤查看数据。'};
  const source = c.sources[node.id];
  if (q.view === 'raw') {
    if (node.kind !== 'source') return {...base,status:'unsupported',message:'原始表格请在对应文件输入步骤中查看。'};
    if (source?.loading) return {...base,status:'loading',message:'正在读取原始表格…'};
    return source?.raw ? {...base,status:'ready',data:sampled(source.raw),sourceId:node.id,message:'原文件局部网格；未统计总体。'}
      : {...base,status:source?.error?'error':'pending',message:source?.error || '原始表格尚未读取，请读取／刷新表格。'};
  }
  const runTable = (id: string, input: boolean, category: ResultCategory): PreviewPlan => {
    const item=c.run?.reviews.find(r=>r.node_id===id);
    const label=input ? `${node.label} · 本步输入` : `${node.label} · 本步输出`;
    if (!item || !c.run) return {...base,title:label,direction:input?'input':'output'};
    if ((category==='excluded' && item.excluded===0) || (category==='exceptions' && item.exceptions===0))
      return {...base,title:label,status:'unsupported',message:`该步骤本批次没有${category==='excluded'?'未纳入':'异常'}结果可查看。`};
    return {...base,status:'table',title:`${label} · ${({normal:'全部结果',excluded:'未纳入',exceptions:'异常',matched:'匹配',left_only:'仅左侧',right_only:'仅右侧'})[category]}`,direction:input?'input':'output',sourceId:id,
      database:c.run.database,table:category==='normal'?item.table:`${item.table}__${category}`,batch:c.run.run_dir,category,
      message:input?`来自上游「${c.project.workflow.nodes.find(n=>n.id===id)?.label || id}」的本批次结果。`:'当前规则对应的实际运行结果。'};
  };
  if (node.kind==='source' && (q.view==='auto' || q.view==='prepared')) {
    if (source?.loading) return {...base,status:'loading',message:'正在检查读取方式…'};
    if (source?.prepared && source.preparedFresh) return {...base,status:'ready',sourceId:node.id,data:sampled(source.prepared),
      title:`${node.label} · 整理效果（${source.confirmed?'已应用':'待确认／未应用'}）`,message:'局部整理样例；未统计总体。'};
    if (source?.applied && source.confirmed && source.appliedKey===sourceIdentity(c.project,node.id))
      return {...base,status:'ready',sourceId:node.id,data:sampled(source.applied),message:'局部字段样例；未统计总体。'};
    if (c.run && !source?.error) return runTable(node.id,false,'normal');
    return {...base,status:source?.error?'error':'stale',message:source?.error || '读取效果尚未更新，请读取／刷新表格；也可查看原始表格。'};
  }
  if (q.view==='input' || (q.view==='auto' && !c.run)) {
    const links=incomingLinks(c.project,node.id);
    const edge=q.inputEdgeId ? links.find(e=>e.id===q.inputEdgeId) : links.length===1?links[0]:undefined;
    const inputBase={...base,title:`${node.label} · 本步输入`,direction:'input' as const};
    if (!edge) return {...inputBase,message:links.length?'请选择实际输入端口／连线。':'该步骤没有连接上游输入。'};
    const category=edge.source_port && edge.source_port!=='main' ? edge.source_port : 'normal';
    if (!['normal','excluded','exceptions'].includes(category)) return {...inputBase,status:'unsupported',message:'该来源端口暂不支持预览。'};
    if(c.run) return runTable(edge.source,true,category as ResultCategory);
    const parent=c.project.workflow.nodes.find(n=>n.id===edge.source);
    const evidence=c.sources[edge.source];
    if (parent?.kind==='source' && category==='normal') {
      if (evidence?.loading) return {...inputBase,status:'loading',message:'正在取得已确认的上游字段样例…'};
      if(evidence?.confirmed && evidence.applied && evidence.appliedKey===sourceIdentity(c.project,parent.id))
        return {...inputBase,status:'ready',sourceId:parent.id,data:sampled(evidence.applied),message:`来自上游「${parent.label}」已确认输出的局部样例；未统计总体。`};
      return {...inputBase,status:evidence?.error?'error':'pending',message:evidence?.error || '上游读取尚未确认或尚未刷新；请先在文件输入步骤检查并应用。'};
    }
    return {...inputBase,sourceId:edge.source,message:'上游处理结果尚未生成，请运行后查看；不会使用更早步骤的原表代替。'};
  }
  return c.run ? runTable(node.id,false,q.category) : base;
}
