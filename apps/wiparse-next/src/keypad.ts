export const keypadRows = [
  ['7', '8', '9', '/', '*', '^', '⌫'],
  ['4', '5', '6', '-', '+', '%', 'C'],
  ['1', '2', '3', '(', ')', '.', 'H'],
  ['0', '0x', 'e', 'pi', 'sin(', 'cos(', 'tan('],
  ['sqrt(', 'ln(', 'log(', 'log2(', 'asin(', 'acos(', 'atan('],
  ['abs(', 'exp(', 'floor(', 'ceil('],
];

/** Same edits as the old scientific keypad. Backspace removes one character. */
export function appendKeypadToken(expression: string, token: string): string {
  switch (token) {
    case '⌫':
      return [...expression].slice(0, -1).join('');
    case 'C':
      return '';
    case '0x':
      return `${expression}0x`;
    case 'H':
      return `${expression}H`;
    case 'pi':
      return `${expression}pi`;
    case 'e':
      return `${expression}e`;
    default:
      return expression + token;
  }
}

/** Enter calculates. Shift+Enter keeps a new line. Other keys stay with the field. */
export function scientificEnter(key: string, shift: boolean, ready: boolean): 'calculate' | 'newline' | 'block' | 'pass' {
  if (key !== 'Enter') return 'pass';
  if (shift) return 'newline';
  return ready ? 'calculate' : 'block';
}
