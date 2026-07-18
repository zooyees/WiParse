import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

export interface CliEnvelope {
  ok: boolean;
  cmd: string;
  ts?: string;
  data?: unknown;
  error?: { code: string; message: string };
}

const moduleDir = path.dirname(fileURLToPath(import.meta.url));

function candidateCliPaths(): string[] {
  const repoRoot = path.resolve(moduleDir, "..", "..", "..");
  return [
    process.env.WIPARSE_CLI_PATH,
    path.join(repoRoot, "dist", "WiParse-CLI.exe"),
    path.join(repoRoot, "target", "release", "wiparse.exe"),
    path.join(repoRoot, "target", "package", "release", "wiparse.exe"),
    "wiparse",
    "WiParse-CLI.exe",
  ].filter((value): value is string => Boolean(value));
}

export function resolveCliPath(): string {
  for (const candidate of candidateCliPaths()) {
    if (candidate.includes(path.sep) || candidate.includes("/")) {
      if (fs.existsSync(candidate)) {
        return candidate;
      }
    } else {
      return candidate;
    }
  }
  return "wiparse";
}

export interface RunCliOptions {
  config?: string;
  cwd?: string;
  timeoutMs?: number;
}

export async function runCli(
  args: string[],
  options: RunCliOptions = {},
): Promise<CliEnvelope> {
  const cli = resolveCliPath();
  const fullArgs = ["--json"];
  if (options.config) {
    fullArgs.push("--config", options.config);
  }
  fullArgs.push(...args);

  const cwd = options.cwd ?? process.env.WIPARSE_CWD ?? process.cwd();
  const timeoutMs = options.timeoutMs ?? 120_000;

  return new Promise((resolve, reject) => {
    const proc = spawn(cli, fullArgs, {
      cwd,
      windowsHide: true,
      env: process.env,
    });

    let stdout = "";
    let stderr = "";
    const timer = setTimeout(() => {
      proc.kill();
      reject(new Error(`WiParse CLI timed out after ${timeoutMs}ms`));
    }, timeoutMs);

    proc.stdout.on("data", (chunk: Buffer) => {
      stdout += chunk.toString("utf8");
    });
    proc.stderr.on("data", (chunk: Buffer) => {
      stderr += chunk.toString("utf8");
    });

    proc.on("error", (error) => {
      clearTimeout(timer);
      reject(new Error(`Failed to start WiParse CLI (${cli}): ${error.message}`));
    });

    proc.on("close", (code) => {
      clearTimeout(timer);
      const text = (stdout || stderr).trim();
      if (!text) {
        reject(
          new Error(
            `WiParse CLI produced no output (exit ${code ?? "?"}). CLI path: ${cli}`,
          ),
        );
        return;
      }
      try {
        const envelope = JSON.parse(text) as CliEnvelope;
        if (!envelope.ok) {
          reject(
            new Error(
              envelope.error?.message ??
                `WiParse command failed: ${envelope.cmd}`,
            ),
          );
          return;
        }
        resolve(envelope);
      } catch {
        reject(new Error(stderr.trim() || stdout.trim() || `exit ${code}`));
      }
    });
  });
}

export function formatEnvelope(envelope: CliEnvelope): string {
  return JSON.stringify(envelope, null, 2);
}
