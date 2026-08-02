#!/usr/bin/env python3
"""Generate a Tektronix WFM#003 file with 4 FastFrame segments (CH1..CH4)."""

from __future__ import annotations

import math
import struct
from pathlib import Path

N = 2000
BPP = 2
NCH = 4
PRE = 0
POST = 0
DT = 1e-6
T0 = -N * DT / 2
VSCALE = 1.0 / 32767.0
VOFF = 0.0


def ch_y(ch: int, i: int) -> float:
    t = T0 + i * DT
    if ch == 0:
        return 0.8 * math.sin(2 * math.pi * 1000 * t)
    if ch == 1:
        return 0.6 if (i // 50) % 2 == 0 else -0.6
    if ch == 2:
        p = (i % 200) / 200.0
        return (4 * p - 1) if p < 0.5 else (3 - 4 * p)
    return -0.7 + 1.4 * ((i % 160) / 160.0)


def pack_cstr(s: str, n: int) -> bytes:
    b = s.encode("ascii", "ignore")[:n]
    return b + b"\x00" * (n - len(b))


def main() -> None:
    frame_total = (PRE + N + POST) * BPP
    curve = bytearray()
    curve_infos: list[tuple[int, int, int, int, int]] = []
    for ch in range(NCH):
        base = ch * frame_total
        curve.extend(b"\x00" * (PRE * BPP))
        for i in range(N):
            code = int(round((ch_y(ch, i) - VOFF) / VSCALE))
            code = max(-32768, min(32767, code))
            curve.extend(struct.pack("<h", code))
        curve.extend(b"\x00" * (POST * BPP))
        dstart = base + PRE * BPP
        post = base + (PRE + N) * BPP
        poststop = base + frame_total
        curve_infos.append((base, dstart, post, poststop, poststop))

    hdr = bytearray()

    def hput(fmt: str, *vals: object) -> None:
        hdr.extend(struct.pack("<" + fmt, *vals))

    # WaveformHeader
    hput("i", 1)  # FASTFRAME
    hput("L", 1)
    hput("Q", 0)
    hput("Q", 0)
    hput("i", 5)
    hput("i", 1)
    hput("L", 1)
    hput("L", 1)
    hput("L", 1)
    hput("i", 2)  # VECTOR
    hput("Q", 0)
    hput("L", 1)
    hput("L", 1)
    hput("L", 1)
    hput("L", NCH)
    hput("L", NCH - 1)

    # summary + pixmap
    hput("H", 0)
    hput("i", 0)
    hput("Q", 0)

    # Explicit dim 1
    hput("d", VSCALE)
    hput("d", VOFF)
    hput("L", 0)
    hdr.extend(pack_cstr("V", 20))
    hput("d", -1.0)
    hput("d", 1.0)
    hput("d", VSCALE)
    hput("d", 0.0)
    hput("i", 0)  # INT16
    hput("i", 0)  # SAMPLE
    hput("i", 0)
    hput("i", 32767)
    hput("i", -32768)
    hput("i", 32767)
    hput("i", -32768)

    # UserView v3 #1
    hput("d", 1.0)
    hdr.extend(pack_cstr("V", 20))
    hput("d", 0.0)
    hput("d", 1.0)
    hput("d", 50.0)
    hput("d", 0.0)

    # Explicit dim 2 (none)
    hput("d", 0.0)
    hput("d", 0.0)
    hput("L", 0)
    hdr.extend(pack_cstr("", 20))
    hput("d", 0.0)
    hput("d", 0.0)
    hput("d", 0.0)
    hput("d", 0.0)
    hput("i", 9)
    hput("i", 6)
    hput("i", 0)
    hput("i", 0)
    hput("i", 0)
    hput("i", 0)
    hput("i", 0)

    # UserView v3 #2
    hput("d", 0.0)
    hdr.extend(pack_cstr("", 20))
    hput("d", 0.0)
    hput("d", 1.0)
    hput("d", 50.0)
    hput("d", 0.0)

    # Implicit dim 1 (time)
    hput("d", DT)
    hput("d", T0)
    hput("L", N)
    hdr.extend(pack_cstr("s", 20))
    hput("d", T0)
    hput("d", T0 + N * DT)
    hput("d", DT)
    hput("d", 0.0)
    hput("L", 0)

    # UserView v3 implicit #1
    hput("d", 1.0)
    hdr.extend(pack_cstr("s", 20))
    hput("d", 0.0)
    hput("d", 1.0)
    hput("d", 50.0)
    hput("d", 0.0)

    # Implicit dim 2
    hput("d", 0.0)
    hput("d", 0.0)
    hput("L", 0)
    hdr.extend(pack_cstr("", 20))
    hput("d", 0.0)
    hput("d", 0.0)
    hput("d", 0.0)
    hput("d", 0.0)
    hput("L", 0)

    # UserView v3 implicit #2
    hput("d", 0.0)
    hdr.extend(pack_cstr("", 20))
    hput("d", 0.0)
    hput("d", 1.0)
    hput("d", 50.0)
    hput("d", 0.0)

    # TimeBase ×2
    hput("Lii", 1, 1, 0)
    hput("Lii", 0, 3, 3)

    # UpdateSpec primary
    hput("L", 0)
    hput("d", 0.0)
    hput("d", 0.0)
    hput("l", 0)

    # CurveInfo primary
    pre, dstart, post, poststop, eoc = curve_infos[0]
    hput("L", 1)
    hput("i", 0)
    hput("h", 0)
    hput("L", pre)
    hput("L", dstart)
    hput("L", post)
    hput("L", poststop)
    hput("L", eoc)

    # FastFrame extras (N-1)
    for _ in range(1, NCH):
        hput("L", 0)
        hput("d", 0.0)
        hput("d", 0.0)
        hput("l", 0)
    for ch in range(1, NCH):
        pre, dstart, post, poststop, eoc = curve_infos[ch]
        hput("L", 1)
        hput("i", 0)
        hput("h", 0)
        hput("L", pre)
        hput("L", dstart)
        hput("L", post)
        hput("L", poststop)
        hput("L", eoc)

    static_len = 1 + 4 + 1 + 4 + 4 + 4 + 8 + 4 + 32 + 4 + 2
    prefix = 2 + 8 + static_len
    curve_offset = prefix + len(hdr)
    bytes_till_eof = (static_len - 1) + len(hdr) + len(curve) + 8
    digits = len(str(bytes_till_eof))

    static = bytearray()
    static.append(digits)
    static.extend(struct.pack("<L", bytes_till_eof))
    static.append(BPP)
    static.extend(struct.pack("<l", curve_offset))
    static.extend(struct.pack("<l", 1))
    static.extend(struct.pack("<f", 0.0))
    static.extend(struct.pack("<d", 1.0))
    static.extend(struct.pack("<f", 0.0))
    static.extend(pack_cstr("CH1|CH2|CH3|CH4", 32))
    static.extend(struct.pack("<L", NCH - 1))
    static.extend(struct.pack("<H", len(hdr) & 0xFFFF))
    assert len(static) == static_len, len(static)

    out = bytearray()
    out.extend(b"\x0f\x0f")
    out.extend(b":WFM#003")
    out.extend(static)
    out.extend(hdr)
    assert len(out) == curve_offset
    out.extend(curve)
    out.extend(struct.pack("<Q", sum(out) & 0xFFFFFFFFFFFFFFFF))

    root = Path(__file__).resolve().parents[1]
    path = root / "sample_waveforms" / "Tektronix_WFM" / "tek_4ch_sine_square_tri_saw.wfm"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(out)
    print(f"wrote {path} ({len(out)} bytes)")
    print(f"channels={NCH} points/ch={N} curve_offset={curve_offset}")


if __name__ == "__main__":
    main()
