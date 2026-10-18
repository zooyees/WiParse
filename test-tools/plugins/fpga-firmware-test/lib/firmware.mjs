import fs from 'node:fs';
import path from 'node:path';
import { jailed, hash, fail } from './common.mjs';
export function inspectFirmware(root, file, firmware) {
  if (firmware.download !== false) throw fail('FEATURE.FIRMWARE_DOWNLOAD_NOT_IMPLEMENTED');
  if (!['hex', 'bin'].includes(firmware.format) || path.extname(firmware.path).toLowerCase() !== `.${firmware.format}`) throw fail('FIRMWARE.FORMAT');
  if (!firmware.target || (firmware.format === 'bin' && (!Number.isSafeInteger(firmware.base_address) || firmware.base_address < 0))) throw fail('FIRMWARE.CONFIG');
  const absolute = jailed(root, path.resolve(path.dirname(file), firmware.path));
  const sha256 = hash(fs.readFileSync(absolute));
  if (!/^[a-f0-9]{64}$/i.test(firmware.sha256) || sha256 !== firmware.sha256.toLowerCase()) throw fail('FIRMWARE.HASH');
  return { ...firmware, absolute, sha256, identity_verified: false };
}
export class FirmwareProgrammer {
  version = 'wiparse.firmware-programmer/1';
  prepare() { throw fail('FEATURE.FIRMWARE_DOWNLOAD_NOT_IMPLEMENTED'); }
  program() { throw fail('FEATURE.FIRMWARE_DOWNLOAD_NOT_IMPLEMENTED'); }
  verify() { throw fail('FEATURE.FIRMWARE_DOWNLOAD_NOT_IMPLEMENTED'); }
  reset() { throw fail('FEATURE.FIRMWARE_DOWNLOAD_NOT_IMPLEMENTED'); }
  read_identity() { return { verified: false }; }
}
