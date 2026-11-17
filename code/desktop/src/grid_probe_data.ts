// Synthetic data only. Formal queries and amounts remain in Rust.
export type ProbeRow = { id: string; [field: string]: string | null };
export const PROBE_COUNT = 2401;
export const PROBE_FIELDS = Array.from({length:24},(_,i)=>i===0?'记录号':i===1?'科目编码':i===2?'摘要':`字段${i+1}`);
export function probePage(start:number,end:number):ProbeRow[]{
  if(!Number.isInteger(start)||!Number.isInteger(end)||start<0||end<start||end-start>500)throw Error('读取范围无效');
  return Array.from({length:Math.max(0,Math.min(end,PROBE_COUNT)-start)},(_,j)=>{
    const n=start+j;const row:ProbeRow={id:`probe-${n}`};
    PROBE_FIELDS.forEach((f,i)=>{row[f]=i===0?String(n+1):i===1?`00${String(n%100).padStart(3,'0')}`:i===2?`虚构摘要 ${n+1} ${'长文本核对；'.repeat(n%4+1)}`:n%17===0?null:n%19===0?'':`${n+1}/${i+1}`;});return row;
  });
}
export function probeDatasource(onRead:(start:number,end:number)=>void=()=>{}){
  let active=true;
  return {getRows(p:{startRow:number;endRow:number;successCallback:(rows:ProbeRow[],last:number)=>void;failCallback:()=>void}){
    onRead(p.startRow,p.endRow);
    queueMicrotask(()=>{if(!active)return;try{p.successCallback(probePage(p.startRow,p.endRow),PROBE_COUNT);}catch{p.failCallback();}});
  },destroy(){active=false;}};
}
