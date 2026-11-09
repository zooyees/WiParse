import {localizeLabel} from './localization';
import { Children, useEffect, useRef, useState, type ReactNode } from 'react';
import { DIVIDER_WIDTH, RAIL_MIN, railMaximum, railWidth, restoreRail } from './workspaceLayout';

export function ResizableWorkspace({id,className,children}:{id:string;className:string;children:ReactNode}) {
  const key=`wiparse.workspace.rail.v1:${id}`;
  const root=useRef<HTMLDivElement>(null);
  const drag=useRef<{pointer:number;x:number;width:number}|null>(null);
  const [preferred,setPreferred]=useState(()=>{try{return restoreRail(window.localStorage.getItem(key));}catch{return RAIL_MIN;}});
  const [container,setContainer]=useState(0);
  useEffect(()=>{
    const element=root.current;if(!element)return;
    const update=()=>setContainer(element.clientWidth);update();
    const observer=new ResizeObserver(update);observer.observe(element);return()=>observer.disconnect();
  },[]);
  const width=railWidth(preferred,container),parts=Children.toArray(children);
  const resize=(value:number)=>{const next=railWidth(value,container);setPreferred(next);try{window.localStorage.setItem(key,String(next));}catch{/* Layout remains usable without persistence. */}};
  return <div ref={root} className={`${className} resizable-workspace`} style={{gridTemplateColumns:`${width}px ${DIVIDER_WIDTH}px minmax(240px,1fr)`}}>
    {parts[0]}
    <div className="workspace-divider" role="separator" aria-label={localizeLabel("调整左侧栏宽度")} aria-orientation="vertical" aria-valuemin={RAIL_MIN} aria-valuemax={railMaximum(container)} aria-valuenow={width} tabIndex={0}
      title={localizeLabel("拖动调整宽度；双击恢复200；方向键微调")}
      onDoubleClick={()=>resize(RAIL_MIN)}
      onKeyDown={event=>{const delta=event.shiftKey?20:4;if(event.key==='ArrowLeft'||event.key==='ArrowRight'){event.preventDefault();resize(width+(event.key==='ArrowLeft'?-delta:delta));}else if(event.key==='Home'){event.preventDefault();resize(RAIL_MIN);}else if(event.key==='End'){event.preventDefault();resize(railMaximum(container));}}}
      onPointerDown={event=>{if(event.button!==0)return;event.preventDefault();event.currentTarget.setPointerCapture(event.pointerId);drag.current={pointer:event.pointerId,x:event.clientX,width};}}
      onPointerMove={event=>{const start=drag.current;if(start?.pointer===event.pointerId)resize(start.width+event.clientX-start.x);}}
      onPointerUp={event=>{if(drag.current?.pointer===event.pointerId){drag.current=null;event.currentTarget.releasePointerCapture(event.pointerId);}}}
      onLostPointerCapture={()=>{drag.current=null;}}/>
    {parts[1]}
  </div>;
}
