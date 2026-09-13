# FTDI FT4222 runtime (optional)

WiParse talks to FT4222H over **LibFT4222 + D2XX**. Scan/identify does **not** need these DLLs; SPI / I2C / GPIO does.

## Can we ship the DLLs?

**Yes, beside the app — with FTDI’s redistributable terms.** Application vendors may ship `LibFT4222.dll` and `ftd2xx.dll` **unmodified** with software that drives FT4222H. We do **not** commit the binaries to git (proprietary FTDI license). Copy them next to `WiParse.exe` or into this folder.

Place:

- `LibFT4222.dll` (or `LibFT4222-64.dll`)
- `ftd2xx.dll` (or `FTD2XX64.dll`)

Search order at runtime: exe directory → `vendor/ftdi` → `WIPARSE_FTDI_DIR` → common FTDI install paths → PATH / System32.

Download (FTDI account / license required):

- LibFT4222: https://ftdichip.com/software-examples/ft4222h-software-examples/
- D2XX: https://ftdichip.com/drivers/d2xx-drivers/

Helper: `scripts/sync-ftdi-dlls.ps1` copies from a typical install into `vendor/ftdi` and `dist/`.

## J-Link / ST-Link / CMSIS-DAP

**Do not bundle `JLinkARM.dll`.** SEGGER’s EULA does not allow redistributing the J-Link software DLL inside a third-party product.

WiParse drives J-Link, ST-Link, and CMSIS-DAP through **probe-rs USB** (no vendor DLL). Install SEGGER J-Link software only if you also use J-Flash / Ozone on the same PC; WiParse does not load that DLL.
