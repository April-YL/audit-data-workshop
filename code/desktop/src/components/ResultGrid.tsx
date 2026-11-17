import { useMemo,useRef,useState, type ReactNode } from 'react';
import { AgGridReact } from 'ag-grid-react';
import { AllCommunityModule, ModuleRegistry, themeQuartz, type ColDef, type ValueGetterParams,type IDatasource,type GridApi } from 'ag-grid-community';

ModuleRegistry.registerModules([AllCommunityModule]);

const theme = themeQuartz.withParams({ fontSize: 12, rowHeight: 30, headerHeight: 38 });

type Props = {
  indices: number[];
  columns: string[];
  rows: string[][];
  renderHeader: (index: number) => ReactNode;
  selectRow: (row: string[]) => void;
  selectCell?: (row:string[], column:string) => void;
  offset:number;
  selectedRecords:number[];
  selectRecord:(record:number,selected:boolean)=>void;
  identity:string;
  totalRows:number;
  loadBlock:(offset:number,limit:number)=>Promise<string[][]>;
};
type GridRow={values:string[];record:number};

export default function ResultGrid(p:Props) {
  const {indices,columns,renderHeader,selectRow,selectCell,selectedRecords,selectRecord}=p;
  const current=useRef(p);current.current=p;
  const api=useRef<GridApi<GridRow>|null>(null);
  const [message,setMessage]=useState('');
  const datasource=useMemo<IDatasource>(()=>{
    let active=true;
    return {
      getRows:params=>{
        const latest=current.current;
        const request=params.startRow===latest.offset
          ? Promise.resolve(latest.rows)
          : latest.loadBlock(params.startRow,params.endRow-params.startRow);
        void request.then(rows=>{
          if(!active)return;
          params.successCallback(rows.map((values,index)=>({values,record:params.startRow+index+1})),latest.totalRows);
          setMessage(`已读取第 ${params.startRow+1}–${params.startRow+rows.length} 行，全量 ${latest.totalRows} 行`);
        }).catch(error=>{if(active){params.failCallback();setMessage(String(error));}});
      },
      destroy:()=>{active=false;},
    };
  },[p.identity]);
  const definitions = useMemo<ColDef<GridRow>[]>(() => [{
    colId:'record-selection',headerName:'选记录',pinned:'left',width:85,
    sortable:false,resizable:false,
    cellRenderer:(params:{data?:GridRow})=>{
      if(!params.data)return null;
      const record=params.data.record;
      return <input type="checkbox" aria-label={`选择记录 ${record}`} checked={selectedRecords.includes(record)}
        onClick={event=>event.stopPropagation()} onChange={event=>selectRecord(record,event.target.checked)}/>;
    },
  },...indices.map(index => ({
    colId: String(index),
    headerName: columns[index],
    headerComponent: () => renderHeader(index),
    valueGetter: (params:ValueGetterParams<GridRow>) => params.data?.values[index],
    pinned: columns[index] === '__row' ? 'left' as const : undefined,
    width: columns[index] === '__row' ? 104 : 190,
    sortable: false,
    resizable: true,
  }))], [indices, columns, renderHeader,selectedRecords,selectRecord]);
  return <div className="formal-result-grid" aria-label="全量结果分块表格">
    <div className="input-action"><button onClick={()=>api.current?.ensureIndexVisible(0,'top')} disabled={!p.totalRows}>查看首行</button>
      <button onClick={()=>api.current?.ensureIndexVisible(p.totalRows-1,'bottom')} disabled={!p.totalRows}>查看末行</button>
      <button onClick={()=>api.current?.refreshInfiniteCache()}>重读分块</button><span role="status">{message}</span></div>
    <AgGridReact<GridRow> key={p.identity} theme={theme} columnDefs={definitions} rowModelType="infinite" datasource={datasource}
      cacheBlockSize={200} maxBlocksInCache={10} maxConcurrentDatasourceRequests={1}
      getRowId={params=>String(params.data.record)} onGridReady={event=>{api.current=event.api;}}
      overlayNoRowsTemplate="当前结果为0行。"
      onCellClicked={event=>{if(event.data)selectCell?.(event.data.values,columns[Number(event.column.getColId())]);}}
      onRowClicked={event => { if (event.data) selectRow(event.data.values); }} />
  </div>;
}
