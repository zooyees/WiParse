export type CrcParameterView = {width: number; poly: string; init: string; refin: boolean; refout: boolean; xorout: string};
type Fields = {width: number; poly: string; init: string; xorout: string; refin: boolean; refout: boolean};

const PRESETS: Record<string, {width: number; poly: number; init: number; refin: boolean; refout: boolean; xorout: number}> = {
  'CRC-8': {width: 8, poly: 0x07, init: 0, refin: false, refout: false, xorout: 0},
  'CRC-8/MAXIM-DOW': {width: 8, poly: 0x31, init: 0, refin: true, refout: true, xorout: 0},
  'CRC-16/IBM (ARC)': {width: 16, poly: 0x8005, init: 0, refin: true, refout: true, xorout: 0},
  'CRC-16/MODBUS': {width: 16, poly: 0x8005, init: 0xffff, refin: true, refout: true, xorout: 0},
  'CRC-16/CCITT-FALSE': {width: 16, poly: 0x1021, init: 0xffff, refin: false, refout: false, xorout: 0},
  'CRC-16/XMODEM': {width: 16, poly: 0x1021, init: 0, refin: false, refout: false, xorout: 0},
  'CRC-32/ISO-HDLC': {width: 32, poly: 0x04c11db7, init: 0xffffffff, refin: true, refout: true, xorout: 0xffffffff},
  'CRC-32C': {width: 32, poly: 0x1edc6f41, init: 0xffffffff, refin: true, refout: true, xorout: 0xffffffff},
};

function mask(width: number): number | null {
  if (width === 8) return 0xff;
  if (width === 16) return 0xffff;
  if (width === 32) return 0xffffffff;
  return null;
}

function formatHex(value: number, width: number): string {
  return `0x${value.toString(16).toUpperCase().padStart(width / 4, '0')}`;
}

function parseHex(input: string, width: number): number | null {
  const limit = mask(width);
  if (limit === null) return null;
  const trimmed = input.trim();
  const body = trimmed.slice(0, 2).toLowerCase() === '0x' ? trimmed.slice(2) : trimmed;
  const digits = body.replaceAll('_', '');
  if (!/^[0-9a-fA-F]+$/.test(digits)) return null;
  const value = Number.parseInt(digits, 16);
  return Number.isSafeInteger(value) && value >= 0 && value <= limit ? value : null;
}

/** Parameter table from the old CRC page. Custom values stay hidden until they fit the width. */
export function crcParameters(preset: string, custom: Fields): CrcParameterView | null {
  const known = PRESETS[preset];
  if (known) return {width: known.width, poly: formatHex(known.poly, known.width), init: formatHex(known.init, known.width), refin: known.refin, refout: known.refout, xorout: formatHex(known.xorout, known.width)};
  if (preset !== 'Custom') return null;
  const poly = parseHex(custom.poly, custom.width);
  const init = parseHex(custom.init, custom.width);
  const xorout = parseHex(custom.xorout, custom.width);
  if (poly === null || poly === 0 || init === null || xorout === null) return null;
  return {width: custom.width, poly: formatHex(poly, custom.width), init: formatHex(init, custom.width), refin: custom.refin, refout: custom.refout, xorout: formatHex(xorout, custom.width)};
}
