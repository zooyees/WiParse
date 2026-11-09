export type Point = {x: number; y: number};
export type BandpassGeometry = {
  vin: Point;
  c1Left: Point;
  c1Right: Point;
  highPass: Point;
  bufferIn: Point;
  bufferOut: Point;
  r2Left: Point;
  r2Right: Point;
  vout: Point;
  groundY: number;
};

/** Node positions from the old bandpass schematic, as fractions of the drawing rectangle. */
export function bandpassGeometry(rect: {x: number; y: number; width: number; height: number}): BandpassGeometry {
  const x = (fraction: number) => rect.x + rect.width * fraction;
  const signalY = rect.y + rect.height * 0.38;
  return {
    vin: {x: x(0.04), y: signalY},
    c1Left: {x: x(0.16), y: signalY},
    c1Right: {x: x(0.21), y: signalY},
    highPass: {x: x(0.29), y: signalY},
    bufferIn: {x: x(0.37), y: signalY},
    bufferOut: {x: x(0.52), y: signalY},
    r2Left: {x: x(0.57), y: signalY},
    r2Right: {x: x(0.73), y: signalY},
    vout: {x: x(0.84), y: signalY},
    groundY: rect.y + rect.height * 0.86,
  };
}

export function horizontalResistor(start: Point, end: Point): Point[] {
  const step = (end.x - start.x) / 8;
  return Array.from({length: 9}, (_, index) => ({
    x: start.x + step * index,
    y: index === 0 || index === 8 ? start.y : index % 2 === 0 ? start.y - 5 : start.y + 5,
  }));
}

export function verticalResistor(x: number, top: number, bottom: number): Point[] {
  const step = (bottom - top) / 8;
  return Array.from({length: 9}, (_, index) => ({
    x: index === 0 || index === 8 ? x : index % 2 === 0 ? x - 5 : x + 5,
    y: top + step * index,
  }));
}
