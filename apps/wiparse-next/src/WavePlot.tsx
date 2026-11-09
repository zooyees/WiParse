import {axisReadoutExtent} from './waveMapping';
import {axisScale,PLOT_INSETS} from './plotAxes';
import {frameScheduler} from './frameScheduler';
import {useEffect,useRef,useState,type PointerEvent as ReactPointerEvent} from 'react';
import {localizeLabel} from './localization';
import {plotExtent,pointerY,nearestCursorIndex,waveBoxZoom} from './waveCursor';

export type View={channelIndex?:number;channel:string;x_unit:string;y_unit:string;start:number;end:number;minimum:number;maximum:number;samples:number;buckets:{x0:number;x1:number;low:number;high:number}[]};
export type Point={index:number;x:number;y:number;x_unit:string;y_unit:string};
type Extent={minimum:number;maximum:number};
type Fraction={x:number;y:number};
type Handle={channel:number;label:string;scale:number;offset:number};
type Props={axisUnit:string;handles:Handle[];onMapping:(channel:number,mapping:{scale:number;offset:number})=>void;view:View;overlays:View[];cursors:Point[];yCursors:number[];axis:'x'|'y';mode:'pan'|'cursor';band:{start:number;end:number}|null;yLimit:Extent|null;onPickX:(x:number)=>void;onPickY:(y:number)=>void;onMoveX:(index:number,x:number)=>void;onMoveY:(index:number,y:number)=>void;onZoomAt:(fraction:number,factor:number)=>void;onZoomY:(fraction:number,factor:number)=>void;onPanY:(minimum:number,maximum:number)=>void;onPan:(start:number,end:number)=>void;onShiftWindow:(fraction:number)=>void;onBox:(range:{start:number;end:number}|null,extent:Extent|null)=>void;onFit:()=>void};
type Drag={kind:'pan'|'box'|'cursor'|'pick';pointer:number;from:Fraction;start:number;end:number;extent:Extent;index:number|null;axis:'x'|'y'};

export function WavePlot({axisUnit,handles,onMapping,view,overlays,cursors,yCursors,axis,mode,band,yLimit,onPickX,onPickY,onMoveX,onMoveY,onZoomAt,onZoomY,onPanY,onPan,onShiftWindow,onBox,onFit}:Props){
  const canvas=useRef<HTMLCanvasElement>(null),drag=useRef<Drag|null>(null);
  const [box,setBox]=useState<{from:Fraction;to:Fraction}|null>(null);
  const hit=(x:number,y:number,clamp=false)=>{const rect=canvas.current?.getBoundingClientRect();if(!rect)return null;const width=rect.width-PLOT_INSETS.left-PLOT_INSETS.right,height=rect.height-PLOT_INSETS.top-PLOT_INSETS.bottom;if(width<=0||height<=0)return null;let a=(x-rect.left-PLOT_INSETS.left)/width,b=(y-rect.top-PLOT_INSETS.top)/height;if(!clamp&&(a<0||a>1||b<0||b>1))return null;a=Math.max(0,Math.min(1,a));b=Math.max(0,Math.min(1,b));return {x:a,y:b,width,height};};
  useEffect(()=>{const element=canvas.current;if(!element)return;const wheel=(event:WheelEvent)=>{if(drag.current)return;const point=hit(event.clientX,event.clientY);if(!point)return;const pixels=event.deltaY*(event.deltaMode===1?16:event.deltaMode===2?point.height:1);if(!pixels)return;if(event.ctrlKey){event.preventDefault();const factor=Math.exp(Math.max(-2,Math.min(2,pixels*.002)));if(event.shiftKey)onZoomY(point.y,factor);else onZoomAt(point.x,factor);}else if(mode==='pan'){event.preventDefault();onShiftWindow(Math.max(-1,Math.min(1,pixels*.001)));}};element.addEventListener('wheel',wheel,{passive:false});return()=>element.removeEventListener('wheel',wheel);},[onZoomAt,onZoomY,onShiftWindow,mode]);
  const down=(event:ReactPointerEvent<HTMLCanvasElement>)=>{if(drag.current||![0,1,2].includes(event.button))return;const point=hit(event.clientX,event.clientY),extent=yLimit??plotExtent(view,overlays);if(!point||!extent)return;event.preventDefault();const kind=event.button===2?'box':event.button===1||mode==='pan'?'pan':'pick';const positions=axis==='x'?cursors.map(cursor=>(cursor.x-view.start)/(view.end-view.start)*point.width):yCursors.map(value=>(extent.maximum-value)/(extent.maximum-extent.minimum)*point.height);const index=kind==='pick'?nearestCursorIndex(positions,axis==='x'?point.x*point.width:point.y*point.height):null;drag.current={kind:index!==null?'cursor':kind,pointer:event.pointerId,from:point,start:view.start,end:view.end,extent,index,axis};if(kind==='box')setBox({from:point,to:point});event.currentTarget.setPointerCapture(event.pointerId);};
  const move=(event:ReactPointerEvent<HTMLCanvasElement>)=>{const origin=drag.current;if(!origin||origin.pointer!==event.pointerId)return;const point=hit(event.clientX,event.clientY,true);if(!point)return;event.preventDefault();if(origin.kind==='pan'){const offset=(point.x-origin.from.x)*(origin.end-origin.start);onPan(origin.start-offset,origin.end-offset);const dy=(point.y-origin.from.y)*(origin.extent.maximum-origin.extent.minimum);onPanY(origin.extent.minimum+dy,origin.extent.maximum+dy);}else if(origin.kind==='box')setBox({from:origin.from,to:point});else if(origin.kind==='cursor'&&origin.index!==null){if(origin.axis==='x')onMoveX(origin.index,origin.start+point.x*(origin.end-origin.start));else onMoveY(origin.index,origin.extent.maximum-point.y*(origin.extent.maximum-origin.extent.minimum));}};
  const release=(event:ReactPointerEvent<HTMLCanvasElement>)=>{const origin=drag.current;if(!origin||origin.pointer!==event.pointerId)return;const point=hit(event.clientX,event.clientY,true);drag.current=null;setBox(null);if(event.currentTarget.hasPointerCapture(event.pointerId))event.currentTarget.releasePointerCapture(event.pointerId);if(!point)return;const dx=Math.abs(point.x-origin.from.x)*point.width,dy=Math.abs(point.y-origin.from.y)*point.height;if(origin.kind==='box'){const zoom=waveBoxZoom(origin.start,origin.end,origin.extent,origin.from,point,point.width,point.height);if(zoom)onBox(zoom.range,zoom.extent);}else if(origin.kind==='pick'&&dx<4&&dy<4){if(origin.axis==='x')onPickX(origin.start+point.x*(origin.end-origin.start));else{const value=pointerY(point.y,0,1,origin.extent.minimum,origin.extent.maximum);if(value!==null)onPickY(value);}}};
  useEffect(()=>{
    const element=canvas.current;if(!element)return;
    const draw=()=>{
      const rect=element.getBoundingClientRect(),width=rect.width,height=rect.height;
      if(width<1||height<1)return;
      const ratio=Math.min(window.devicePixelRatio||1,2);
      const pixelWidth=Math.round(width*ratio),pixelHeight=Math.round(height*ratio);if(element.width!==pixelWidth)element.width=pixelWidth;if(element.height!==pixelHeight)element.height=pixelHeight;
      const context=element.getContext('2d');if(!context)return;
      const style=getComputedStyle(document.documentElement),dark=document.documentElement.dataset.theme==='dark';
      const muted=style.getPropertyValue('--muted').trim(),line=style.getPropertyValue('--line').trim();
      context.setTransform(ratio,0,0,ratio,0,0);context.clearRect(0,0,width,height);
      const {left,right,top,bottom}=PLOT_INSETS,w=Math.max(1,width-left-right),h=Math.max(1,height-top-bottom);
      const extent=yLimit??plotExtent(view,overlays);
      if(!extent){context.fillStyle='#a54b3f';context.font='12px Microsoft YaHei UI, sans-serif';context.fillText('叠加幅值范围过大，无法显示',16,24);return;}
      const ymin=extent.minimum,ymax=extent.maximum;
      const span=view.end-view.start,dx=span||1;
      const px=(x:number)=>span===0?left+w/2:left+(x-view.start)/dx*w;
      const py=(y:number)=>top+(ymax-y)/(ymax-ymin)*h;
      context.font='12px Microsoft YaHei UI, sans-serif';context.fillStyle=muted;context.strokeStyle=line;context.lineWidth=1;
      const rawAxis=axisReadoutExtent({minimum:ymin,maximum:ymax},handles[0]??{scale:1,offset:0});const xAxis=axisScale(view.start,view.end),yAxis=axisScale(rawAxis.minimum,rawAxis.maximum);
      context.textAlign='right';context.textBaseline='middle';
      for(let i=0;i<=4;i++){const y=top+h*i/4;context.beginPath();context.moveTo(left,y);context.lineTo(left+w,y);context.stroke();context.fillText(yAxis.labels[4-i],left-8,y,left-12);}
      context.textBaseline='top';
      for(let i=0;i<=4;i++){const x=left+w*i/4;context.beginPath();context.moveTo(x,top);context.lineTo(x,top+h);context.stroke();context.textAlign=i===0?'left':i===4?'right':'center';context.fillText(xAxis.labels[i],x,top+h+7,Math.max(20,w/4-4));}
      context.font='11px Microsoft YaHei UI, sans-serif';context.textAlign='left';context.fillText(`${view.channel} · ${rawAxis.original?axisUnit:localizeLabel('显示坐标')}${yAxis.annotation?' · '+yAxis.annotation:''}`,left,5,w);
      context.textAlign='right';context.fillText(xAxis.annotation,left+w,height-15,w);
      context.save();context.beginPath();context.rect(left,top,w,h);context.clip();
      if(band&&band.end>view.start&&band.start<view.end){const a=Math.max(view.start,band.start),b=Math.min(view.end,band.end);context.fillStyle='rgba(37,108,176,.14)';context.fillRect(px(a),top,Math.max(1,px(b)-px(a)),h);}
      context.lineWidth=1.25;
      [view,...overlays].forEach((series,seriesIndex)=>{const color=(dark?['#77baff','#f3b273','#6bd9b0','#c5a0ee']:['#256cb0','#bd6439','#398263','#895ba5'])[seriesIndex%4];context.strokeStyle=color;
        if(series.buckets.every(bucket=>bucket.x0===bucket.x1&&bucket.low===bucket.high)){
          context.beginPath();series.buckets.forEach((bucket,index)=>{const x=px(bucket.x0),y=py(bucket.low);if(index===0)context.moveTo(x,y);else context.lineTo(x,y);});context.stroke();if(series.buckets.length===1){context.beginPath();context.arc(px(series.buckets[0].x0),py(series.buckets[0].low),2,0,Math.PI*2);context.fillStyle=color;context.fill();}
        }else{
          // Connect the two extrema envelopes, retaining narrow peaks without
          // inventing a mean value or leaving flat intervals as isolated dots.
          for(const edge of ['low','high'] as const){context.beginPath();series.buckets.forEach((bucket,index)=>{const y=py(bucket[edge]);if(index===0)context.moveTo(px(bucket.x0),y);else context.lineTo(px(bucket.x0),y);context.lineTo(px(bucket.x1),y);});context.stroke();}
          context.beginPath();for(const bucket of series.buckets){if(bucket.low===bucket.high)continue;const x=px(bucket.x0+(bucket.x1-bucket.x0)/2);context.moveTo(x,py(bucket.low));context.lineTo(x,py(bucket.high));}context.stroke();
        }
      });
      context.setLineDash([4,3]);context.strokeStyle='#c17a19';for(const point of cursors){if(point.x<view.start||point.x>view.end)continue;const x=px(point.x);context.beginPath();context.moveTo(x,top);context.lineTo(x,top+h);context.stroke();}context.strokeStyle='#7a4ea3';for(const value of yCursors){if(value<ymin||value>ymax)continue;const y=py(value);context.beginPath();context.moveTo(left,y);context.lineTo(left+w,y);context.stroke();}context.setLineDash([]);
      if(box){const x=Math.min(box.from.x,box.to.x),y=Math.min(box.from.y,box.to.y);context.fillStyle='rgba(79,149,218,.18)';context.fillRect(left+x*w,top+y*h,Math.max(1,Math.abs(box.to.x-box.from.x)*w),Math.max(1,Math.abs(box.to.y-box.from.y)*h));context.strokeStyle='#4f95da';context.setLineDash([4,3]);context.strokeRect(left+x*w,top+y*h,Math.abs(box.to.x-box.from.x)*w,Math.abs(box.to.y-box.from.y)*h);context.setLineDash([]);}
      context.restore();
    };
    const frames=frameScheduler(draw,callback=>requestAnimationFrame(callback),id=>cancelAnimationFrame(id));
    const observer=new ResizeObserver(frames.schedule);observer.observe(element);
    const themeObserver=new MutationObserver(frames.schedule);themeObserver.observe(document.documentElement,{attributes:true,attributeFilter:['data-theme']});
    window.addEventListener('resize',frames.schedule);frames.schedule();return()=>{frames.dispose();observer.disconnect();themeObserver.disconnect();window.removeEventListener('resize',frames.schedule);};
  },[view,overlays,cursors,yCursors,band,yLimit,box,axisUnit,handles]);
  const extent=yLimit??plotExtent(view,overlays);
  const handleDrag=useRef<{pointer:number;from:number;handle:Handle;scaleMode:boolean}|null>(null);
  return <div className="wave-plot-surface"><canvas ref={canvas} className={`wave-canvas wave-mode-${mode}`} title={localizeLabel('Ctrl+滚轮缩放X；Ctrl+Shift+滚轮缩放Y；左键二维平移；右键框选；双击适应')} onContextMenu={event=>event.preventDefault()} onDoubleClick={onFit} onPointerDown={down} onPointerMove={move} onPointerUp={release} onPointerCancel={()=>{drag.current=null;setBox(null);}} onLostPointerCapture={()=>{drag.current=null;setBox(null);}} aria-label={`${view.channel} 波形，横轴 ${view.x_unit}，纵轴 ${view.y_unit}`}/>{extent&&handles.map((handle,index)=>{const fraction=Math.max(0,Math.min(1,(extent.maximum-handle.offset)/(extent.maximum-extent.minimum)));return <button key={handle.channel} type="button" className="wave-ground-handle" style={{top:`calc(${PLOT_INSETS.top}px + (100% - ${PLOT_INSETS.top+PLOT_INSETS.bottom}px) * ${fraction})`,left:PLOT_INSETS.left+3+(index%4)*13}} title={`${handle.label} 地线：拖动平移，Ctrl 拖动调幅，双击复位`} aria-label={`${handle.label} 地线显示手柄`} onDoubleClick={()=>onMapping(handle.channel,{scale:1,offset:0})} onPointerDown={event=>{event.preventDefault();event.stopPropagation();handleDrag.current={pointer:event.pointerId,from:event.clientY,handle:{...handle},scaleMode:event.ctrlKey};event.currentTarget.setPointerCapture(event.pointerId);}} onPointerMove={event=>{const drag=handleDrag.current;if(!drag||drag.pointer!==event.pointerId)return;const height=(canvas.current?.getBoundingClientRect().height??0)-PLOT_INSETS.top-PLOT_INSETS.bottom;if(height<=0)return;const delta=event.clientY-drag.from;onMapping(drag.handle.channel,drag.scaleMode?{scale:Math.max(1e-12,Math.min(1e12,drag.handle.scale*Math.exp(-delta/100))),offset:drag.handle.offset}:{scale:drag.handle.scale,offset:drag.handle.offset-delta/height*(extent.maximum-extent.minimum)});}} onPointerUp={event=>{handleDrag.current=null;if(event.currentTarget.hasPointerCapture(event.pointerId))event.currentTarget.releasePointerCapture(event.pointerId);}} onPointerCancel={()=>{handleDrag.current=null;}} onLostPointerCapture={()=>{handleDrag.current=null;}}>▸<span>{handle.label}</span></button>;})}</div>;
}
