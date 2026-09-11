export default async function run(ctx) {
  return {
    ok: true,
    plugin: "market-echo",
    echo: { params: ctx.params || {}, lifecycle: ctx.lifecycle },
  };
}
export async function preflight() {
  return { ok: true, lifecycle: "preflight", checks: [{ id: "echo-ready", ok: true }] };
}
