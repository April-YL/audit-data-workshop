export type Kind = 'source' | 'map' | 'filter' | 'export' | 'structure' | 'decimal' | 'text_clean' | 'text_extract' | 'mark' | 'union' | 'aggregate' | 'pivot' | 'sequence' | 'join' | 'inspect' | 'derive' | 'sort' | 'duplicates' | 'deduplicate';
export type WorkNode = { id: string; kind: Kind; version: number; label: string; position: { x: number; y: number }; config: Record<string, unknown> };
export type WorkEdge = { id: string; source: string; target: string; source_port?: string; target_port?: string };
export type Workflow = { schema_version: number; name: string; nodes: WorkNode[]; edges: WorkEdge[] };
export type TableLayout = { first_col: number; last_col: number; header_rows: number[]; data_start_row: number; data_end_row?: number; fill_merged_data_cols: number[]; field_names?: Record<string, string>; strict_columns?: boolean };
export type StructureField = { physical_col: number; parts: string[]; name: string };
export type ReadingIssue = { code: string; message: string; row: number | null; col: number | null; related_cols: number[] };
export type ReadingBaseline = { version: number; sheet: string; layout: TableLayout; header_cells: string[][]; header_merges: number[][] };
export type ReadingCheck = { compatible: boolean; baseline: ReadingBaseline | null; diagnostics: ReadingIssue[]; report: StructureReport | null; full_checked: boolean };
export type StructureCandidate = { layout: TableLayout; fields: StructureField[]; issues: string[]; diagnostics: ReadingIssue[]; baseline: ReadingBaseline; ready: boolean; score: number; preview: SourceSample };
export type StructureReport = { raw: RawSourceSample; candidates: StructureCandidate[]; active: StructureCandidate | null; notices: string[] };
export type TextOptions = { encoding: 'utf-8' | 'gb18030'; delimiter: ',' | '\t' | ';' | '|'; header_record: number };
export type SourceBinding = { path: string; sheet?: string; fields: Record<string, string>; layout?: TableLayout; text?: TextOptions; reading_baseline?: ReadingBaseline | null };
export type ProjectFile = { schema_version: number; workflow: Workflow; company_name?: string | null; bindings: Record<string, SourceBinding> };
export type SourceSample = { headers: string[]; rows: string[][]; physical_rows: number[]; truncated: boolean; sheet: string };
export type RawSourceSample = { columns: string[]; rows: string[][]; physical_rows: number[]; truncated: boolean; sheet: string };
export type Review = { node_id: string; label: string; rows: number; amount_total: string | null; exceptions: number; excluded: number; table: string };
export type RunResult = { run_dir: string; database: string; reviews: Review[]; input_fingerprints: Record<string, string> };
export type Preview = { columns: string[]; rows: string[][]; total_rows: number };
export type Mapping = { source: string; target: string; empty_as_zero?: boolean;trim_special_space?:boolean;parentheses_negative?:boolean;thousands?:boolean;currency_prefix?:string };
export function incomingLinks(project: ProjectFile, nodeId: string): WorkEdge[] {
  return project.workflow.edges.filter(edge => edge.target === nodeId);
}
export type Condition = { field: string; operator: string; value: string;values?:(string|null)[] };
export type TableAction = {kind:'keep';fields:string[]} | {kind:'rename';field:string;name:string} | {kind:'filter';condition:Condition}
  | {kind:'decimal';fields:Mapping[]}
  | {kind:'text_clean';fields:Mapping[];mode:'trim'|'replace';find:string;replacement:string}
  | {kind:'text_extract';source:string;target:string;pattern:string;group:number}
  | {kind:'derive';expression:string;target:string;placement:'first'|'last'|'before'|'after';anchor?:string}
  | {kind:'reorder';fields:string[]}
  | {kind:'source_fields';fields:Mapping[]}
  | {kind:'update_filter';config:Record<string,unknown>}
  | {kind:'sequence'|'sort'|'duplicates'|'deduplicate'|'aggregate'|'multi_filter';config:Record<string,unknown>};
export const projectSnapshot = (project:ProjectFile):string => JSON.stringify(project);
export const hasUnsavedProject = (project:ProjectFile,saved:string,pendingReading:boolean):boolean => pendingReading || projectSnapshot(project)!==saved;
export const businessColumns = (columns:string[]):string[] => columns.filter(c=>c!=='原始行' && c!=='company' && !c.startsWith('__'));

export function applyTableAction(project:ProjectFile,nodeId:string,action:TableAction,columns:string[],edgeId?:string):{project:ProjectFile;node:WorkNode} {
  const current=project.workflow.nodes.find(n=>n.id===nodeId);
  if(!current || current.kind==='export')throw new Error('请在有效处理步骤的输出上操作。');
  if(action.kind==='update_filter'){
    if(current.kind!=='filter')throw Error('当前步骤不是筛选。');
    const node={...current,config:action.config};
    return {project:applyStepCommand(project,{kind:'update',node}),node};
  }
  const available=businessColumns(columns);
  if(!available.length || new Set(available.map(c=>c.toLowerCase())).size!==available.length)throw new Error('请先确认有效且不重名的字段。');
  let step:WorkNode;
  const position={x:current.position.x+250,y:current.position.y};
  const validSource=(field:string)=>{if(!available.includes(field))throw new Error('处理字段不在当前输出中。');};
  const validTarget=(name:string)=>{
    if(!name.trim() || name!==name.trim() || name.startsWith('__') || name.toLowerCase()==='company' || name==='原始行' || [...name].length>100 || /[\r\n\0]/.test(name))throw new Error('目标列名无效或使用了保留名称。');
    if(available.some(c=>c.toLowerCase()===name.toLowerCase()))throw new Error('目标列已存在，请使用新的名称；原值会保留。');
  };
  if(action.kind==='source_fields'){
    if(!action.fields.length)throw Error('请选择来源列。');
    const targets=new Set<string>();
    for(const field of action.fields){
      if(!columns.includes(field.source)||!/^__(?:(?:left|right)__)?(?:file|sheet|row|record)$/.test(field.source))throw Error('请选择当前结果中存在的来源列。');
      validTarget(field.target);
      if(targets.has(field.target.toLowerCase()))throw Error('来源输出列重名。');
      targets.add(field.target.toLowerCase());
    }
    step=newNode('map',position);
    step.config={mappings:[...available.map(source=>({source,target:source})),...action.fields]};
  } else if(action.kind==='derive') {
    validTarget(action.target);validTarget(`${action.target}__error`);
    if(!action.expression.trim())throw new Error('请填写公式。');
    if(['before','after'].includes(action.placement) && !available.includes(action.anchor||''))throw new Error('请选择有效位置列。');
    step=newNode('derive',position);step.config={expression:action.expression,target:action.target};
  } else if(['sequence','sort','duplicates','deduplicate','aggregate','multi_filter'].includes(action.kind) && 'config' in action) {
    step=newNode(action.kind==='multi_filter'?'filter':action.kind as Kind,position);step.config=action.config;
    if(action.kind==='aggregate')step.version=2;
  } else if(action.kind==='filter') {
    if(!available.includes(action.condition.field))throw new Error('筛选字段不在当前输出中。');
    if(!['equals','not_equals','contains','is_empty','is_not_empty','numeric_gt','numeric_gte','numeric_lt','numeric_lte','numeric_eq','numeric_neq','date_eq','date_neq','date_gt','date_gte','date_lt','date_lte'].includes(action.condition.operator))throw new Error('不支持的筛选条件。');
    if(!['is_empty','is_not_empty'].includes(action.condition.operator) && !action.condition.value.trim())throw new Error('请填写筛选值。');
    step=newNode('filter',{x:current.position.x+250,y:current.position.y});step.config={condition:action.condition};
  } else if(action.kind==='decimal' || action.kind==='text_clean') {
    if(!action.fields.length)throw new Error('请至少选择一列。');
    const sources=new Set<string>(),targets=new Set<string>();
    for(const field of action.fields){validSource(field.source);validTarget(field.target);if(sources.has(field.source)||targets.has(field.target.toLowerCase()))throw new Error('源字段或目标列不能重复。');sources.add(field.source);targets.add(field.target.toLowerCase());}
    if(action.kind==='text_clean' && (action.mode!=='trim' && action.mode!=='replace'))throw new Error('不支持的文本处理规则。');
    if(action.kind==='text_clean' && action.mode==='replace' && !action.find)throw new Error('查找文本不能为空。');
    step=newNode(action.kind,position);step.config=action.kind==='decimal'?{fields:action.fields}:{fields:action.fields,mode:action.mode,find:action.find,replacement:action.replacement};
  } else if(action.kind==='text_extract') {
    validSource(action.source);validTarget(action.target);
    if(!action.pattern || !Number.isInteger(action.group) || action.group<0)throw new Error('请填写提取规则及有效分组序号。');
    step=newNode('text_extract',position);step.config={source:action.source,target:action.target,pattern:action.pattern,group:action.group};
  } else {
    const fields=action.kind==='keep'||action.kind==='reorder'?action.fields:available;
    if(action.kind==='reorder' && fields.length!==available.length)throw new Error('移动列必须保留全部字段。');
    if(!fields.length || new Set(fields).size!==fields.length || fields.some(c=>!available.includes(c)))throw new Error('请至少选择一个有效字段，且不能重复。');
    if(action.kind==='rename') {
      const name=action.name;
      if(!available.includes(action.field) || !name.trim() || name!==name.trim() || name.startsWith('__') || name==='company' || [...name].length>100 || /[\r\n\0]/.test(name))throw new Error('列名无效或使用了保留名称。');
      if(available.some(c=>c!==action.field && c.toLowerCase()===name.toLowerCase()))throw new Error('列名与已有字段重复。');
    }
    step=newNode('map',{x:current.position.x+250,y:current.position.y});
    step.config={mappings:fields.map(source=>({source,target:action.kind==='rename' && source===action.field?action.name:source}))};
  }
  const outgoing = project.workflow.edges.filter(edge => edge.source === nodeId);
  const main = outgoing.filter(edge => !edge.source_port || edge.source_port === 'main');
  const chosen = edgeId ? main.find(edge => edge.id === edgeId) : main.length === 1 ? main[0] : undefined;
  if (action.kind !== 'aggregate' && (edgeId ? !chosen : main.length > 1)) {
    throw new Error('请选择要插入的下游连线。');
  }
  const incomingEdgeId = 'edge_' + crypto.randomUUID().replaceAll('-', '').slice(0, 12);
  if (action.kind === 'aggregate') step.position = { x: position.x, y: position.y + 220 };
  const next = chosen && action.kind !== 'aggregate'
    ? applyStepCommand(project, { kind: 'insert', edgeId: chosen.id, node: step, incomingEdgeId })
    : applyStepCommand(project, { kind: 'add', node: step, incoming: { id: incomingEdgeId, source: nodeId, target: step.id, source_port: 'main' } });
    if(action.kind==='derive'){
    const order=[...available];
    const index=action.placement==='first'?0:action.placement==='last'?order.length:order.indexOf(action.anchor!)+(action.placement==='after'?1:0);
    order.splice(index,0,action.target);order.push(`${action.target}__error`);
    return applyTableAction(next,step.id,{kind:'reorder',fields:order},[...available,action.target,`${action.target}__error`]);
  }
  return {project:next,node:step};
}

export const blankProject = (): ProjectFile => ({
  schema_version: 1,
  workflow: { schema_version: 1, name: '未命名处理项目', nodes: [], edges: [] },
  bindings: {},
});

export const toolCatalog: { kind: Kind; label: string; group: string; hint: string }[] = [
  { kind: 'source', label: '文件输入', group: '输入', hint: '选择本地 CSV、TXT／TSV 或 XLSX' },
  { kind: 'map', label: '字段选择／映射', group: '整理', hint: '选择并重命名字段' },
  { kind: 'decimal', label: '转换为金额', group: '整理', hint: '新增金额列，保留原值与异常' },
  { kind: 'text_clean', label: '文本清理', group: '整理', hint: '去首尾空格或普通文本替换' },
  { kind: 'text_extract', label: '文本提取', group: '整理', hint: '按明确规则生成新列' },
  { kind: 'filter', label: '条件筛选', group: '行处理', hint: '保留符合条件的行' },
  { kind: 'derive', label: '公式计算', group: '整理', hint: '引用字段生成新列' },
  { kind: 'sequence', label: '结果编号', group: '行处理', hint: '按当前确定顺序生成编号' },
  { kind: 'sort', label: '排序', group: '行处理', hint: '多字段稳定排序' },
  { kind: 'duplicates', label: '重复识别', group: '行处理', hint: '保留全部记录并标记重复' },
  { kind: 'deduplicate', label: '按规则去重', group: '行处理', hint: '按明确顺序保留，其他行进入未纳入' },
  { kind: 'aggregate', label: '分类汇总', group: '统计', hint: '创建汇总旁支，保留明细' },
  { kind: 'pivot', label: '交叉透视', group: '统计', hint: '行列维度、多项统计与总计' },
  { kind: 'join', label: '多表关联', group: '多表', hint: '按匹配键带回列，复核左右来源' },
  { kind: 'union', label: '多表追加', group: '多表', hint: '逐表映射后按顺序合并' },
  { kind: 'export', label: '导出 CSV', group: '输出', hint: '指定输出目录' },
];

export function newNode(kind: Kind, position: { x: number; y: number }): WorkNode {
  const id = `${kind}_${crypto.randomUUID().replaceAll('-', '').slice(0, 12)}`;
  const config: Record<string, unknown> = kind === 'source' ? { source_key: id }
    : kind === 'map' ? { mappings: [] }
    : kind === 'decimal' ? {fields:[]}
    : kind === 'text_clean' ? {fields:[],mode:'trim',find:'',replacement:''}
    : kind === 'text_extract' ? {source:'',target:'',pattern:'',group:1}
    : kind === 'derive' ? {expression:'',target:''}
    : kind === 'sequence' ? {target:'结果编号',start:1}
    : kind === 'sort' ? {order:[]}
    : kind === 'duplicates' ? {keys:[],target:'是否重复',count_target:'重复条数',rank_target:'组内序号'}
    : kind === 'deduplicate' ? {keys:[],keep:'first'}
    : kind === 'aggregate' ? {group_by:[],metrics:[]}
    : kind === 'pivot' ? {row_by:[],column_by:[],metrics:[]}
    : kind === 'join' ? {mode:'left',keys:[],columns:[],match_empty:false}
    : kind === 'union' ? {ports:['input_1','input_2'],fields:[],mappings:[[],[]],excluded:[[],[]]}
    : kind === 'filter' ? { condition: { field: '', operator: 'equals', value: '' } }
    : kind === 'export' ? { output_root: '', file_name: '处理结果.csv' } : {};
  return { id, kind, version: ['aggregate','decimal','join','union'].includes(kind)?2:1, label: toolCatalog.find(tool => tool.kind === kind)?.label ?? kind, position, config };
}

export function incomingNode(project: ProjectFile, nodeId: string): WorkNode | undefined {
  const edge = project.workflow.edges.find(item => item.target === nodeId);
  return project.workflow.nodes.find(node => node.id === edge?.source);
}

export function visibleFields(project: ProjectFile, nodeId: string, samples: Record<string, SourceSample>, seen = new Set<string>()): string[] {
  if (seen.has(nodeId)) return [];
  seen.add(nodeId);
  const node = project.workflow.nodes.find(item => item.id === nodeId);
  if (!node) return [];
  if (node.kind === 'source') return samples[node.id]?.headers ?? [];
  if (node.kind === 'map') {
    const mappings = node.config.mappings as Mapping[] | undefined;
    if (mappings) return mappings.map(mapping => mapping.target).filter(Boolean);
  }
  const parent = incomingNode(project, nodeId);
  return parent ? visibleFields(project, parent.id, samples, seen) : [];
}

export type StepCommand =
  | { kind: 'add'; node: WorkNode; incoming?: WorkEdge }
  | { kind: 'update'; node: WorkNode }
  | { kind: 'configure_inputs'; node: WorkNode; edges: WorkEdge[] }
  | { kind: 'remove'; nodeId: string }
  | { kind: 'connect'; edge: WorkEdge }
  | { kind: 'disconnect'; edgeId: string }
  | { kind: 'insert'; edgeId: string; node: WorkNode; incomingEdgeId: string };

export function applyStepCommand(project: ProjectFile, command: StepCommand): ProjectFile {
  const { workflow } = project;
  if (command.kind === 'configure_inputs') {
    if (!['join','union'].includes(command.node.kind) || command.node.version !== 2) throw new Error('仅新版多表步骤可配置输入。');
    const ports = command.node.kind === 'join' ? ['left','right'] : command.node.config.ports as string[];
    if (!Array.isArray(ports) || ports.length<2 || new Set(ports).size!==ports.length || command.edges.length!==ports.length) throw new Error('请逐个配置不同的输入端口。');
    let next = applyStepCommand(project, {kind:'update',node:command.node});
    next = {...next,workflow:{...next.workflow,edges:next.workflow.edges.filter(edge=>edge.target!==command.node.id)}};
    for (const edge of command.edges) {
      if (edge.target !== command.node.id) throw new Error('输入连线目标与步骤不一致。');
      next = applyStepCommand(next,{kind:'connect',edge});
    }
    return next;
  }
  if (command.kind === 'remove') {
    if (!workflow.nodes.some(node => node.id === command.nodeId)) return project;
    const bindings = { ...project.bindings };
    delete bindings[command.nodeId];
    return { ...project, bindings, workflow: {
      ...workflow,
      nodes: workflow.nodes.filter(node => node.id !== command.nodeId),
      edges: workflow.edges.filter(edge => edge.source !== command.nodeId && edge.target !== command.nodeId),
    } };
  }
  if (command.kind === 'connect') {
    const { edge } = command;
    const source = workflow.nodes.find(node => node.id === edge.source);
    const target = workflow.nodes.find(node => node.id === edge.target);
    if (!source || !target || source.kind === 'export' || target.kind === 'source') throw new Error('这两个端口不能连接。');
    const multi = target.kind === 'join' || target.kind === 'union';
    if (multi) {
      if (target.version !== 2) throw new Error('旧版多表步骤沿用保存的输入。');
      const ports = target.kind === 'join' ? ['left','right'] : target.config.ports as string[];
      if (!edge.target_port || !ports.includes(edge.target_port)) throw new Error('请选择有效的多表输入端口。');
    }
    if (edge.source_port && edge.source_port !== 'main') throw new Error('请选择正常结果输出端口。');
    if (workflow.edges.some(item => item.id === edge.id || item.target === edge.target && (!multi || item.target_port === edge.target_port))) throw new Error('该输入端口已有连线，请先断开原连线。');
    const pending = [edge.target];
    const visited = new Set<string>();
    while (pending.length) {
      const id = pending.pop()!;
      if (id === edge.source) throw new Error('连线会形成循环，未添加。');
      if (visited.has(id)) continue;
      visited.add(id);
      for (const item of workflow.edges) if (item.source === id) pending.push(item.target);
    }
    return { ...project, workflow: { ...workflow, edges: [...workflow.edges, edge] } };
  }
  if (command.kind === 'update') {
    const previous = workflow.nodes.find(node => node.id === command.node.id);
    if (!previous || previous.kind !== command.node.kind) throw new Error('要修改的步骤不存在或类型已变化。');
    return { ...project, workflow: { ...workflow, nodes: workflow.nodes.map(node => node.id === previous.id ? command.node : node) } };
  }
  if (command.kind === 'disconnect') {
    if (!workflow.edges.some(edge => edge.id === command.edgeId)) return project;
    return { ...project, workflow: { ...workflow, edges: workflow.edges.filter(edge => edge.id !== command.edgeId) } };
  }
  if (workflow.nodes.some(node => node.id === command.node.id)) throw new Error('步骤编号重复。');
  let nodes = [...workflow.nodes, command.node];
  if (command.kind === 'add') {
    const incoming = command.incoming;
    if (!incoming) return { ...project, workflow: { ...workflow, nodes } };
    if (incoming.target !== command.node.id || !workflow.nodes.some(node => node.id === incoming.source) || workflow.edges.some(edge => edge.id === incoming.id)) {
      throw new Error('新步骤的输入连线无效。');
    }
    return { ...project, workflow: { ...workflow, nodes, edges: [...workflow.edges, incoming] } };
  }

  const edge = workflow.edges.find(item => item.id === command.edgeId);
  if (!edge || !workflow.nodes.some(node => node.id === edge.source) || !workflow.nodes.some(node => node.id === edge.target)) {
    throw new Error('指定连线或端点已变化，请重新选择。');
  }
  if (['source', 'export', 'join', 'union'].includes(command.node.kind)) throw new Error('指定步骤不支持插入单条连线。');
  if (workflow.edges.some(item => item.id === command.incomingEdgeId)) throw new Error('连线编号重复。');
  const target = workflow.nodes.find(node => node.id === edge.target);
  if (target?.kind === 'join' && target.version === 1) {
    const key = target.config.left_id === edge.source ? 'left_id' : target.config.right_id === edge.source ? 'right_id' : null;
    if (!key) throw new Error('关联步骤的输入参数与连线不一致。');
    nodes = nodes.map(node => node.id === target.id ? { ...node, config: { ...node.config, [key]: command.node.id } } : node);
  }
  const incoming: WorkEdge = { id: command.incomingEdgeId, source: edge.source, target: command.node.id, source_port: edge.source_port };
  const edges = workflow.edges.map(item => item.id === edge.id ? { ...item, source: command.node.id, source_port: 'main' } : item);
  return { ...project, workflow: { ...workflow, nodes, edges: [...edges, incoming] } };
}
