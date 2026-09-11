export default async function run(ctx) {
  const n = Math.max(1, Number(ctx.params?.count ?? 3) || 3);
  const steps = [];
  for (let i = 1; i <= n; i++) steps.push(i);
  return { ok: true, plugin: "market-counter", count: n, steps };
}
export async function preflight() {
  return {
    ok: true,
    lifecycle: "preflight",
    checks: [
      { id: "node", ok: true },
      { id: "counter", ok: true, detail: "ready" },
    ],
  };
}
