export function settlingPercent(tauMultiple: number): {complete: number; remaining: number} {
  const remaining = Math.exp(-tauMultiple) * 100;
  return {complete: 100 - remaining, remaining};
}

function fixed(value: number): string {
  return value.toFixed(3);
}

export function settlingNote(): string {
  const one = settlingPercent(1);
  const three = settlingPercent(3);
  const five = settlingPercent(5);
  return `1τ: 完成${fixed(one.complete)}%/误差${fixed(one.remaining)}% · 3τ: ${fixed(three.complete)}%/${fixed(three.remaining)}% · 5τ: ${fixed(five.complete)}%/${fixed(five.remaining)}%`;
}
