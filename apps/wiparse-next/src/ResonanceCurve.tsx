import {LocalizedLabel} from './localization';
import type {ResonancePoint} from './lcCurve';

function formatHz(hz: number) {
  return hz >= 1_000_000 ? `${(hz / 1_000_000).toFixed(2)} MHz` : `${Math.round(hz / 1_000)} kHz`;
}

export function ResonanceCurve({points}: {points: ResonancePoint[]}) {
  const width = 640, height = 188, left = 52, right = 16, top = 16, bottom = 28;
  const frequencies = points.map(point => point[0]);
  const currents = points.map(point => point[1]);
  const minX = Math.min(...frequencies), maxX = Math.max(...frequencies), maxY = Math.max(...currents, 1e-12);
  const xOf = (frequency: number) => left + ((frequency - minX) / (maxX - minX || 1)) * (width - left - right);
  const yOf = (current: number) => top + (1 - current / maxY) * (height - top - bottom);
  const path = points.map((point, index) => `${index ? 'L' : 'M'}${xOf(point[0]).toFixed(1)},${yOf(point[1]).toFixed(1)}`).join(' ');
  const peak = currents.indexOf(Math.max(...currents));
  return <figure className="lc-curve"><figcaption><LocalizedLabel text={"频率 - 谐振电流（1 Vrms）"}/></figcaption><svg viewBox={`0 0 ${width} ${height}`} role="img" aria-label={`谐振电流曲线，峰值约 ${currents[peak].toFixed(3)} 安，位于 ${formatHz(frequencies[peak])}`}><line x1={left} y1={top} x2={left} y2={height - bottom} stroke="#d8dee5"/><line x1={left} y1={height - bottom} x2={width - right} y2={height - bottom} stroke="#d8dee5"/><path d={path} fill="none" stroke="#2563a9" strokeWidth="1.8"/><text x={left - 6} y={top + 4} textAnchor="end" fill="#5b6877" fontSize="11">{maxY.toFixed(3)} A</text><text x={left - 6} y={height - bottom} textAnchor="end" fill="#5b6877" fontSize="11">0 A</text><text x={left} y={height - 8} textAnchor="start" fill="#5b6877" fontSize="11">{formatHz(minX)}</text><text x={xOf(frequencies[peak])} y={height - 8} textAnchor="middle" fill="#5b6877" fontSize="11">{formatHz(frequencies[peak])}</text><text x={width - right} y={height - 8} textAnchor="end" fill="#5b6877" fontSize="11">{formatHz(maxX)}</text></svg></figure>;
}
