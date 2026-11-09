import {LocalizedLabel,localizeLabel} from './localization';
import {bandpassGeometry, horizontalResistor, verticalResistor, type Point} from './bandpassGeometry';

const line = '#253547';
const accent = '#2563a9';
const muted = '#5b6877';

function points(path: Point[]): string {
  return path.map(point => `${point.x.toFixed(2)},${point.y.toFixed(2)}`).join(' ');
}

export function BandpassSchematic() {
  const width = 640;
  const height = 170;
  const drawing = {x: 6, y: 6, width: width - 12, height: height - 12};
  const node = bandpassGeometry(drawing);
  const c2Top = node.vout.y + 22;
  const c2Bottom = c2Top + 8;
  return <figure className="bandpass-schematic"><figcaption><LocalizedLabel text={"电路结构"}/></figcaption><svg viewBox={`0 0 ${width} ${height}`} role="img" aria-label={localizeLabel("RC 带通电路：高通级 C1 和 R1，理想缓冲，低通级 R2 和 C2")}>
    <text x={(node.vin.x + node.highPass.x) / 2} y={drawing.y + 12} textAnchor="middle" fill={muted} fontSize="10"><LocalizedLabel text={"高通级"}/></text>
    <text x={(node.r2Left.x + node.vout.x) / 2} y={drawing.y + 12} textAnchor="middle" fill={muted} fontSize="10"><LocalizedLabel text={"低通级"}/></text>
    <line x1={node.vin.x} y1={node.vin.y} x2={node.c1Left.x} y2={node.c1Left.y} stroke={line} strokeWidth="1.5"/>
    <line x1={node.c1Left.x} y1={node.c1Left.y - 10} x2={node.c1Left.x} y2={node.c1Left.y + 10} stroke={accent} strokeWidth="1.5"/>
    <line x1={node.c1Right.x} y1={node.c1Right.y - 10} x2={node.c1Right.x} y2={node.c1Right.y + 10} stroke={accent} strokeWidth="1.5"/>
    <text x={(node.c1Left.x + node.c1Right.x) / 2} y={node.c1Left.y - 14} textAnchor="middle" fill={line} fontSize="10">C1</text>
    <line x1={node.c1Right.x} y1={node.c1Right.y} x2={node.highPass.x} y2={node.highPass.y} stroke={line} strokeWidth="1.5"/>
    <circle cx={node.highPass.x} cy={node.highPass.y} r="2.5" fill={accent}/>
    <polyline points={points(verticalResistor(node.highPass.x, node.highPass.y + 12, node.groundY - 16))} fill="none" stroke={line} strokeWidth="1.5"/>
    <line x1={node.highPass.x} y1={node.highPass.y} x2={node.highPass.x} y2={node.highPass.y + 12} stroke={line} strokeWidth="1.5"/>
    <text x={node.highPass.x + 8} y={(node.highPass.y + node.groundY) / 2} fill={line} fontSize="10">R1</text>
    <line x1={node.highPass.x} y1={node.highPass.y} x2={node.bufferIn.x} y2={node.bufferIn.y} stroke={line} strokeWidth="1.5"/>
    <polyline points={points([{x: node.bufferIn.x, y: node.bufferIn.y - 16}, {x: node.bufferIn.x, y: node.bufferIn.y + 16}, node.bufferOut])} fill="none" stroke={accent} strokeWidth="1.5"/>
    <text x={(node.bufferIn.x + node.bufferOut.x) / 2} y={node.bufferIn.y + 29} textAnchor="middle" fill={accent} fontSize="9"><LocalizedLabel text={"理想缓冲"}/></text>
    <line x1={node.bufferOut.x} y1={node.bufferOut.y} x2={node.r2Left.x} y2={node.r2Left.y} stroke={line} strokeWidth="1.5"/>
    <polyline points={points(horizontalResistor(node.r2Left, node.r2Right))} fill="none" stroke={line} strokeWidth="1.5"/>
    <text x={(node.r2Left.x + node.r2Right.x) / 2} y={node.r2Left.y - 14} textAnchor="middle" fill={line} fontSize="10">R2</text>
    <line x1={node.r2Right.x} y1={node.r2Right.y} x2={node.vout.x} y2={node.vout.y} stroke={line} strokeWidth="1.5"/>
    <circle cx={node.vout.x} cy={node.vout.y} r="2.5" fill={accent}/>
    <line x1={node.vout.x} y1={node.vout.y} x2={node.vout.x} y2={c2Top} stroke={line} strokeWidth="1.5"/>
    <line x1={node.vout.x - 10} y1={c2Top} x2={node.vout.x + 10} y2={c2Top} stroke={accent} strokeWidth="1.5"/>
    <line x1={node.vout.x - 10} y1={c2Bottom} x2={node.vout.x + 10} y2={c2Bottom} stroke={accent} strokeWidth="1.5"/>
    <text x={node.vout.x + 12} y={(c2Bottom + node.groundY) / 2} fill={line} fontSize="10">C2</text>
    <line x1={node.vout.x} y1={c2Bottom} x2={node.vout.x} y2={node.groundY - 6} stroke={line} strokeWidth="1.5"/>
    <Ground x={node.highPass.x} y={node.groundY}/>
    <Ground x={node.vout.x} y={node.groundY}/>
    <text x={node.vin.x} y={node.vin.y + 22} textAnchor="middle" fill={line} fontSize="10">Vin</text>
    <text x={node.vout.x} y={node.vout.y + 22} textAnchor="middle" fill={line} fontSize="10">Vout</text>
    <text x={(node.highPass.x + node.vout.x) / 2} y={node.groundY + 14} textAnchor="middle" fill={muted} fontSize="9">GND</text>
  </svg></figure>;
}

function Ground({x, y}: {x: number; y: number}) {
  return <g stroke={line} strokeWidth="1.5">
    <line x1={x} y1={y - 6} x2={x} y2={y - 1}/>
    <line x1={x - 8} y1={y} x2={x + 8} y2={y}/>
    <line x1={x - 5} y1={y + 3} x2={x + 5} y2={y + 3}/>
    <line x1={x - 2} y1={y + 6} x2={x + 2} y2={y + 6}/>
  </g>;
}
