export type CanvasNode = {id:string;position:{x:number;y:number}};
export type CanvasEdge = {source:string;target:string};

const STEP_GAP = 240;

/** Leave a readable horizontal slot on the original branch when inserting. */
export function makeInsertSpace<T extends CanvasNode>(nodes:T[], sourceId:string, targetId:string):{nodes:T[];position:{x:number;y:number}}{
  const source=nodes.find(n=>n.id===sourceId), target=nodes.find(n=>n.id===targetId);
  if(!source||!target)throw new Error('插入位置的连线端点不存在');
  const insertedX=source.position.x+STEP_GAP;
  const shift=Math.max(0,insertedX+STEP_GAP-target.position.x);
  if(target.position.x>source.position.x){
    return {nodes:nodes.map(n=>n.position.x>=target.position.x?{...n,position:{...n.position,x:n.position.x+shift}}:n),
      position:{x:insertedX,y:source.position.y}};
  }
  // A manually reversed edge cannot be made left-to-right by shifting every
  // column: that would also move its source. Keep unrelated nodes in place.
  return {nodes,position:{x:(source.position.x+target.position.x)/2,y:Math.max(source.position.y,target.position.y)+110}};
}

/** The selected step, its direct inputs and its direct outputs form a readable local view. */
export function localNeighborhood(selectedId:string,edges:CanvasEdge[]):string[]{
  const ids=new Set([selectedId]);
  for(const edge of edges){
    if(edge.source===selectedId)ids.add(edge.target);
    if(edge.target===selectedId)ids.add(edge.source);
  }
  return [...ids];
}
