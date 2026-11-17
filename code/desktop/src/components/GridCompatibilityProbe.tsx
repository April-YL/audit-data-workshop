import {useMemo,useRef,useState} from 'react';
import {AgGridReact} from 'ag-grid-react';
import {AllCommunityModule,ModuleRegistry,themeQuartz,type ColDef,type GridApi} from 'ag-grid-community';
import {PROBE_FIELDS,probeDatasource,type ProbeRow} from '../grid_probe_data';
import './grid_probe.css';
ModuleRegistry.registerModules([AllCommunityModule]);
export default function GridCompatibilityProbe(){
  const api=useRef<GridApi<ProbeRow>|null>(null);
  const [selected,setSelected]=useState<string[]>([]);
  const [menu,setMenu]=useState<{field:string;x:number;y:number}|null>(null);
  const [last,setLast]=useState('尚未读取');
  const [generation,setGeneration]=useState(0);
  const [scale,setScale]=useState(1);
  const columns=useMemo<ColDef<ProbeRow>[]>(()=>PROBE_FIELDS.map((field,i)=>({field,headerName:field,width:i===2?330:150,pinned:i===0?'left':undefined,sortable:false,filter:false,resizable:true,
    headerComponent:()=> <button className="probe-header" aria-pressed={selected.includes(field)} onClick={e=>{setSelected(v=>v.includes(field)?v.filter(f=>f!==field):[...v,field]);setMenu(null);e.stopPropagation();}} onContextMenu={e=>{e.preventDefault();setMenu({field,x:Math.min(e.clientX,window.innerWidth-220),y:Math.min(e.clientY,window.innerHeight-160)});}}>{selected.includes(field)?'✓ ':''}{field}</button>
  })),[selected]);
  const datasource=useMemo(()=>probeDatasource((start,end)=>setLast(`本次读取 ${start+1}—${Math.min(end,2401)} 行`)),[generation,scale]);
  return <main className="grid-probe"><header><h1>表格组件验证 · 虚构资料</h1><p>2401行／24列。点击列头多选；右键列头打开菜单。正式筛选、金额和导出不在本页计算。</p>
    <div className="probe-tools"><label>显示比例 <select value={scale} onChange={e=>setScale(Number(e.target.value))}><option value={1}>100%</option><option value={1.25}>125%</option><option value={1.5}>150%</option></select></label>
    <button onClick={()=>{setGeneration(v=>v+1);setMenu(null);}}>重新加载</button><button onClick={()=>api.current?.ensureIndexVisible(2400,'bottom')}>查看末行</button><button onClick={()=>api.current?.ensureIndexVisible(0,'top')}>返回首行</button></div>
    <p role="status">{last}；所选列：{selected.join('、')||'无'}</p></header>
    <div className="probe-grid" style={{fontSize:14*scale}}><AgGridReact<ProbeRow> key={`${generation}-${scale}`} rowHeight={32*scale} headerHeight={38*scale} theme={themeQuartz.withParams({fontSize:14*scale,rowHeight:32*scale,headerHeight:38*scale})} columnDefs={columns} rowModelType="infinite" datasource={datasource} cacheBlockSize={200} maxBlocksInCache={10} maxConcurrentDatasourceRequests={1} getRowId={p=>p.data.id} onGridReady={e=>{api.current=e.api;}} /></div>
    {menu&&<div className="probe-menu" role="menu" style={{left:menu.x,top:menu.y}}><strong>{menu.field}</strong><button role="menuitem" onClick={()=>{api.current?.applyColumnState({state:[{colId:menu.field,pinned:'left'}]});setMenu(null);}}>固定到左侧</button><button role="menuitem" onClick={()=>{api.current?.applyColumnState({state:[{colId:menu.field,pinned:null}]});setMenu(null);}}>取消固定</button><button role="menuitem" onClick={()=>setMenu(null)}>关闭菜单</button></div>}
  </main>;
}

