/**
 * Marketplace trust: integrity (SHA-256), optional signature hooks, publisher policy.
 */

import crypto from "node:crypto";
import fs from "node:fs";

export const MARKETPLACE_ERROR = Object.freeze({
  HTTPS: "E_HTTPS",
  HASH: "E_HASH",
  SIG: "E_SIG",
  TRUST: "E_TRUST",
  COMPAT: "E_COMPAT",
  LAYOUT: "E_LAYOUT",
  AUTH: "E_AUTH",
  NOT_FOUND: "E_NOT_FOUND",
  NETWORK: "E_NETWORK",
  CONFIG: "E_CONFIG",
});

export const SANDBOX_PERMISSIONS = Object.freeze([
  "gui.api",
  "cli",
  "serial",
  "fs.data_root",
  "fs.plugin_dir",
  "network.outbound",
]);

export const CHANNELS = Object.freeze(["stable", "beta", "internal"]);

export class MarketplaceError extends Error {
  constructor(code, message, details = undefined) {
    super(message);
    this.name = "MarketplaceError";
    this.code = code;
    this.details = details;
  }

  toJSON() {
    return {
      ok: false,
      error: {
        code: this.code,
        message: this.message,
        ...(this.details != null ? { details: this.details } : {}),
      },
    };
  }
}

export function defaultTrustPolicy() {
  return {
    require_signature: false,
    public_keys: [],
    allowed_publishers: [],
    denied_publishers: [],
  };
}

export function normalizeTrustPolicy(raw = {}) {
  const base = defaultTrustPolicy();
  return {
    require_signature: Boolean(raw.require_signature ?? base.require_signature),
    public_keys: Array.isArray(raw.public_keys)
      ? raw.public_keys.map((k) => String(k).trim()).filter(Boolean)
      : [],
    allowed_publishers: Array.isArray(raw.allowed_publishers)
      ? raw.allowed_publishers.map((p) => String(p).trim().toLowerCase()).filter(Boolean)
      : [],
    denied_publishers: Array.isArray(raw.denied_publishers)
      ? raw.denied_publishers.map((p) => String(p).trim().toLowerCase()).filter(Boolean)
      : [],
  };
}

export function sha256Hex(buf) {
  return crypto.createHash("sha256").update(buf).digest("hex");
}

export function sha256File(path) {
  const hash = crypto.createHash("sha256");
  const fd = fs.openSync(path, "r");
  try {
    const buf = Buffer.alloc(256 * 1024);
    let n;
    while ((n = fs.readSync(fd, buf, 0, buf.length, null)) > 0) {
      hash.update(buf.subarray(0, n));
    }
  } finally {
    fs.closeSync(fd);
  }
  return hash.digest("hex");
}

export function assertHttpsUrl(url, { allowHttp = false } = {}) {
  const s = String(url || "").trim();
  if (!s) {
    throw new MarketplaceError(MARKETPLACE_ERROR.CONFIG, "empty URL");
  }
  let u;
  try {
    u = new URL(s);
  } catch {
    throw new MarketplaceError(MARKETPLACE_ERROR.CONFIG, `invalid URL: ${s}`);
  }
  if (u.protocol === "https:") return u;
  if (allowHttp && u.protocol === "http:") return u;
  throw new MarketplaceError(
    MARKETPLACE_ERROR.HTTPS,
    `URL must use HTTPS: ${s}`,
    { url: s }
  );
}

export function allowHttpFromEnv() {
  const v = String(process.env.WIPARSE_MARKETPLACE_ALLOW_HTTP || "")
    .trim()
    .toLowerCase();
  return v === "1" || v === "true" || v === "yes" || v === "on";
}

/**
 * Validate marketplace package metadata object.
 * @returns {{ ok: boolean, errors: string[] }}
 */
export function validatePackageMeta(raw) {
  const errors = [];
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) {
    return { ok: false, errors: ["meta must be an object"] };
  }
  if (!raw.id || typeof raw.id !== "string") errors.push("id: required string");
  else if (!/^[a-zA-Z0-9][a-zA-Z0-9._-]*$/.test(raw.id)) {
    errors.push("id: invalid pattern");
  }
  if (!raw.version || typeof raw.version !== "string") {
    errors.push("version: required string");
  }
  if (!raw.sha256 || typeof raw.sha256 !== "string") {
    errors.push("sha256: required string");
  } else if (!/^[a-fA-F0-9]{64}$/.test(raw.sha256)) {
    errors.push("sha256: must be 64 hex chars");
  }
  if (raw.size == null || typeof raw.size !== "number" || !Number.isFinite(raw.size) || raw.size < 0) {
    errors.push("size: required non-negative number");
  }
  if (raw.channel != null && !CHANNELS.includes(raw.channel)) {
    errors.push(`channel: must be one of ${CHANNELS.join("|")}`);
  }
  if (raw.sandbox?.permissions != null) {
    if (!Array.isArray(raw.sandbox.permissions)) {
      errors.push("sandbox.permissions: must be array");
    } else {
      for (const p of raw.sandbox.permissions) {
        if (!SANDBOX_PERMISSIONS.includes(p)) {
          errors.push(`sandbox.permissions: unknown '${p}'`);
        }
      }
    }
  }
  return { ok: errors.length === 0, errors };
}

export function checkPublisherTrust(publisher, policy) {
  const p = String(publisher || "").trim().toLowerCase();
  const denied = policy.denied_publishers || [];
  if (p && denied.includes(p)) {
    throw new MarketplaceError(
      MARKETPLACE_ERROR.TRUST,
      `publisher denied: ${publisher}`,
      { publisher }
    );
  }
  const allowed = policy.allowed_publishers || [];
  if (allowed.length && (!p || !allowed.includes(p))) {
    throw new MarketplaceError(
      MARKETPLACE_ERROR.TRUST,
      `publisher not in allowlist: ${publisher || "(missing)"}`,
      { publisher, allowed }
    );
  }
}

/**
 * Verify bytes/file against package meta + trust policy.
 * Signature verification: Ed25519 over UTF-8 sha256 hex when keys configured / required.
 */
export function verifyArtifactIntegrity({ meta, bytes = null, filePath = null, trust = null }) {
  const policy = normalizeTrustPolicy(trust || {});
  const check = validatePackageMeta(meta);
  if (!check.ok) {
    throw new MarketplaceError(MARKETPLACE_ERROR.LAYOUT, check.errors.join("; "));
  }
  checkPublisherTrust(meta.publisher, policy);

  let actual;
  if (filePath) {
    const st = fs.statSync(filePath);
    if (st.size !== meta.size) {
      throw new MarketplaceError(MARKETPLACE_ERROR.HASH, `size mismatch: expected ${meta.size}, got ${st.size}`);
    }
    actual = sha256File(filePath);
  } else if (bytes != null) {
    const buf = Buffer.isBuffer(bytes) ? bytes : Buffer.from(bytes);
    if (buf.length !== meta.size) {
      throw new MarketplaceError(MARKETPLACE_ERROR.HASH, `size mismatch: expected ${meta.size}, got ${buf.length}`);
    }
    actual = sha256Hex(buf);
  } else {
    throw new MarketplaceError(MARKETPLACE_ERROR.CONFIG, "verify requires bytes or filePath");
  }

  const expected = String(meta.sha256).toLowerCase();
  if (actual !== expected) {
    throw new MarketplaceError(MARKETPLACE_ERROR.HASH, `hash mismatch: expected ${expected}, got ${actual}`, {
      expected,
      actual,
    });
  }

  const hasSig = meta.signature != null && String(meta.signature).trim() !== "";
  if (policy.require_signature && !hasSig) {
    throw new MarketplaceError(MARKETPLACE_ERROR.SIG, "signature required by trust policy");
  }
  if (hasSig && policy.public_keys.length) {
    verifyDetachedSignature(meta.sha256.toLowerCase(), meta.signature, policy.public_keys);
  } else if (hasSig && policy.require_signature && !policy.public_keys.length) {
    throw new MarketplaceError(MARKETPLACE_ERROR.SIG, "signature present but no public keys configured");
  }

  return { ok: true, sha256: actual };
}

/**
 * Ed25519 verify: signature is base64 of signature over message = sha256 hex utf8.
 * Keys may be base64 SPKI/PKCS8 raw 32-byte or hex 64-char seed/public.
 */
export function verifyDetachedSignature(sha256HexStr, signatureB64, publicKeys) {
  const message = Buffer.from(String(sha256HexStr).toLowerCase(), "utf8");
  let sig;
  try {
    sig = Buffer.from(String(signatureB64), "base64");
  } catch {
    throw new MarketplaceError(MARKETPLACE_ERROR.SIG, "invalid signature encoding");
  }
  if (sig.length !== 64) {
    throw new MarketplaceError(MARKETPLACE_ERROR.SIG, "signature must be 64 bytes (Ed25519)");
  }
  for (const keyRaw of publicKeys) {
    try {
      const keyObj = importPublicKey(keyRaw);
      const ok = crypto.verify(null, message, keyObj, sig);
      if (ok) return true;
    } catch {
      // try next key
    }
  }
  throw new MarketplaceError(MARKETPLACE_ERROR.SIG, "signature verification failed");
}

function importPublicKey(raw) {
  const s = String(raw).trim();
  // PEM
  if (s.includes("BEGIN PUBLIC KEY")) {
    return crypto.createPublicKey(s);
  }
  // hex 32-byte raw public key
  if (/^[a-fA-F0-9]{64}$/.test(s)) {
    const rawKey = Buffer.from(s, "hex");
    return crypto.createPublicKey({
      key: Buffer.concat([
        // SPKI prefix for Ed25519
        Buffer.from("302a300506032b6570032100", "hex"),
        rawKey,
      ]),
      format: "der",
      type: "spki",
    });
  }
  // base64 DER SPKI or raw 32
  const buf = Buffer.from(s, "base64");
  if (buf.length === 32) {
    return crypto.createPublicKey({
      key: Buffer.concat([Buffer.from("302a300506032b6570032100", "hex"), buf]),
      format: "der",
      type: "spki",
    });
  }
  return crypto.createPublicKey({ key: buf, format: "der", type: "spki" });
}

/** Sign sha256 hex with Ed25519 private key PEM or raw hex/base64 seed (for tests/server). */
export function signSha256Hex(sha256HexStr, privateKeyPemOrRaw) {
  const message = Buffer.from(String(sha256HexStr).toLowerCase(), "utf8");
  let key;
  const s = String(privateKeyPemOrRaw).trim();
  if (s.includes("BEGIN PRIVATE KEY") || s.includes("BEGIN OPENSSH")) {
    key = crypto.createPrivateKey(s);
  } else if (/^[a-fA-F0-9]{64}$/.test(s)) {
    const seed = Buffer.from(s, "hex");
    // Node requires PKCS8; generate from seed via createPrivateKey is non-trivial —
    // prefer PEM in production. For hex seed use crypto.generateKeyPair sync substitute:
    throw new MarketplaceError(
      MARKETPLACE_ERROR.SIG,
      "raw hex private keys unsupported; pass PKCS8 PEM"
    );
  } else {
    key = crypto.createPrivateKey({
      key: Buffer.from(s, "base64"),
      format: "der",
      type: "pkcs8",
    });
  }
  const sig = crypto.sign(null, message, key);
  return sig.toString("base64");
}

export function generateEd25519KeyPair() {
  const { publicKey, privateKey } = crypto.generateKeyPairSync("ed25519");
  return {
    publicKeyPem: publicKey.export({ type: "spki", format: "pem" }),
    privateKeyPem: privateKey.export({ type: "pkcs8", format: "pem" }),
    publicKeyRawHex: publicKey
      .export({ type: "spki", format: "der" })
      .subarray(-32)
      .toString("hex"),
  };
}

export default {
  MARKETPLACE_ERROR,
  SANDBOX_PERMISSIONS,
  CHANNELS,
  MarketplaceError,
  defaultTrustPolicy,
  normalizeTrustPolicy,
  sha256Hex,
  sha256File,
  assertHttpsUrl,
  allowHttpFromEnv,
  validatePackageMeta,
  checkPublisherTrust,
  verifyArtifactIntegrity,
  verifyDetachedSignature,
  signSha256Hex,
  generateEd25519KeyPair,
};
