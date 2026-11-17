type Props={open:boolean;busy:boolean;onChoice:(choice:'save'|'discard'|'cancel')=>void};
export default function ProjectGuardDialog(p:Props) {
  if(!p.open)return null;
  return <div className="modal-backdrop"><section className="project-guard" role="dialog" aria-modal="true" aria-labelledby="guard-title">
    <h2 id="guard-title">当前项目有未保存的更改</h2><p>继续前可以保存，也可以放弃更改；取消会保留当前工作。读取方案尚未应用时，请先应用或放弃该方案再保存。</p>
    <div><button disabled={p.busy} onClick={()=>p.onChoice('cancel')}>取消，继续编辑</button><button disabled={p.busy} onClick={()=>p.onChoice('discard')}>放弃更改</button><button className="primary" disabled={p.busy} onClick={()=>p.onChoice('save')}>{p.busy?'正在保存…':'保存并继续'}</button></div>
  </section></div>;
}
