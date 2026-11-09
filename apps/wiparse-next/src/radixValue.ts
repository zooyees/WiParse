const DIGITS = '0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ';
const MIN_MAGNITUDE = 1n << 127n;
const MAX_MAGNITUDE = MIN_MAGNITUDE - 1n;
const U128_MAX = (1n << 128n) - 1n;

/** Signed i128 parse from the old radix page, including sign, underscores and 0b/0o/0x. */
export function parseI128(input: string, sourceBase: number): bigint | null {
  if (!Number.isInteger(sourceBase) || sourceBase < 2 || sourceBase > 36) return null;
  const compact = [...input].filter(character => !/\s/u.test(character) && character !== '_').join('');
  if (!compact) return null;
  let negative = false;
  let unsigned = compact;
  if (unsigned.startsWith('-')) {
    negative = true;
    unsigned = unsigned.slice(1);
  } else if (unsigned.startsWith('+')) unsigned = unsigned.slice(1);
  const prefix = unsigned.slice(0, 2).toLowerCase();
  const digits = (sourceBase === 2 && prefix === '0b') || (sourceBase === 8 && prefix === '0o') || (sourceBase === 16 && prefix === '0x')
    ? unsigned.slice(2)
    : unsigned;
  if (!digits) return null;
  let magnitude = 0n;
  const base = BigInt(sourceBase);
  for (const character of digits) {
    const value = DIGITS.indexOf(character.toUpperCase());
    if (value < 0 || value >= sourceBase) return null;
    magnitude = magnitude * base + BigInt(value);
    if (magnitude > U128_MAX) return null;
  }
  if (negative) {
    if (magnitude === MIN_MAGNITUDE) return -MIN_MAGNITUDE;
    return magnitude <= MAX_MAGNITUDE ? -magnitude : null;
  }
  return magnitude <= MAX_MAGNITUDE ? magnitude : null;
}

export function formatI128(value: bigint, targetBase: number): string | null {
  if (!Number.isInteger(targetBase) || targetBase < 2 || targetBase > 36) return null;
  if (value === 0n) return '0';
  const negative = value < 0n;
  let magnitude = negative ? -value : value;
  let output = '';
  const base = BigInt(targetBase);
  while (magnitude > 0n) {
    output = DIGITS[Number(magnitude % base)] + output;
    magnitude /= base;
  }
  return negative ? `-${output}` : output;
}

export function radixSummary(input: string, sourceBase: number): string | null {
  const value = parseI128(input, sourceBase);
  if (value === null) return null;
  const parts = [2, 8, 10, 16].map(base => formatI128(value, base));
  if (parts.some(part => part === null)) return null;
  const [bin, oct, dec, hex] = parts;
  return `BIN ${bin}  ·  OCT ${oct}  ·  DEC ${dec}  ·  HEX ${hex}`;
}
