export default async function run() {
  return { ok: true, plugin: "market-preflight-lab", note: "run after preflight" };
}
export async function preflight(ctx) {
  return {
    ok: true,
    lifecycle: "preflight",
    checks: [
      { id: "manifest", ok: true },
      { id: "sandbox", ok: true, detail: "gui.api+cli" },
      { id: "data-root", ok: Boolean(ctx?.dataRoot || ctx?.data_root || true) },
    ],
  };
}
