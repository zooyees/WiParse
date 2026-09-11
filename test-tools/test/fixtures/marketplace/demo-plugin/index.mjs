export default async function run(ctx) {
  return { ok: true, plugin: "demo-marketplace-plugin", lifecycle: ctx.lifecycle };
}
export async function preflight(ctx) {
  return { ok: true, lifecycle: "preflight", checks: [{ id: "smoke", ok: true }] };
}
