import csv
import json
import sys
from datetime import datetime
from pathlib import Path

import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
import numpy as np

repo = Path(__file__).resolve().parents[1]
stamp = sys.argv[1] if len(sys.argv) > 1 else datetime.now().strftime('%Y%m%d_%H%M%S')
assert len(stamp) == 15 and stamp[8] == '_' and stamp.replace('_', '').isdigit()
version = __import__('re').search(r'^version\s*=\s*"([^"]+)"', (repo/'Cargo.toml').read_text(), __import__('re').M)[1]
demo_date = f'{stamp[:4]}-{stamp[4:6]}-{stamp[6:8]}'
run = repo / 'projects/default/tests/ScopeSerialDemo/runs' / stamp
artifacts = run / 'artifacts'
for folder in ['reports', 'waves', 'shots', 'docs']:
    (artifacts / folder).mkdir(parents=True, exist_ok=False)
records = []
dt = 5e-7
t = np.arange(8000) * dt
for cycle, (trigger, frequency, fault) in enumerate([
    ('PING_START', 125000, False),
    ('POWER_TRANSFER', 150000, False),
    ('VIN_UV_FAULT', 150000, True),
], 1):
    vin = np.where((t >= 0.002) & fault, 4.8, 9.0) + 0.03*np.sin(2*np.pi*1000*t)
    pwm = 3.3 * ((t * frequency) % 1 < 0.45)
    if fault:
        pwm[t >= 0.00208] = 0
    stem = f'ScopeSerialDemo_{trigger}_{stamp}'
    isf = artifacts / 'waves' / f'{stem}.isf'
    with isf.open('wb') as stream:
        for channel, values in [('CH1 VIN', vin), ('CH2 PWM', pwm)]:
            raw = np.rint(values / 0.1).astype(np.int8).tobytes()
            header = f':WFMPRE:BYT_NR 1;BIT_NR 8;ENCDG BIN;BN_FMT RI;BYT_OR MSB;NR_PT {len(raw)};XINCR {dt};XZERO 0;XUNIT "s";YMULT 0.1;YOFF 0;YZERO 0;YUNIT "V";WFID "{channel}";:CURVE #{len(str(len(raw)))}{len(raw)}'
            stream.write(header.encode('ascii') + raw + b'\n')
    csv_path = isf.with_suffix('.csv')
    with csv_path.open('w', newline='', encoding='utf-8') as stream:
        writer = csv.writer(stream)
        writer.writerow(['time_s', 'VIN_V', 'PWM_V'])
        writer.writerows(zip(t, vin, pwm))
    shot = artifacts / 'shots' / f'{stem}.png'
    plt.style.use('dark_background')
    fig, axes = plt.subplots(2, 1, figsize=(12, 4.8), sharex=True)
    window = (t >= 0.00196) & (t <= 0.00216) if fault else t <= 0.00012
    for ax, values, label, color in zip(axes, [vin, pwm], ['CH1 VIN (V)', 'CH2 PWM (V)'], ['#59de9b', '#63bcff']):
        ax.plot(t[window]*1000, values[window], color=color, linewidth=1.25)
        ax.set_ylabel(label)
        ax.grid(True, alpha=0.18)
    axes[0].set_ylim(4, 10)
    axes[1].set_ylim(-0.3, 3.8)
    axes[1].set_xlabel('Time from capture start (ms)')
    fig.suptitle(f'SIMULATION ONLY | Cycle {cycle} | {trigger} | PWM preset {frequency/1000:g} kHz', fontsize=12)
    fig.tight_layout()
    fig.savefig(shot, dpi=110)
    plt.close(fig)
    lines = [f'[SIM] [12:00:{cycle:02d}.000] RX_DETECT=1 VIN=9.00V',
             f'[SIM] [12:00:{cycle:02d}.020] EVENT={trigger} PWM_HZ={frequency}',
             f'[SIM] [12:00:{cycle:02d}.021] CAPTURE_REQUEST cycle={cycle}',
             f'[SIM] [12:00:{cycle:02d}.100] ' + ('VIN=4.80V FAULT=UV PWM=OFF' if fault else 'VIN=9.00V CAPTURE_OK=1')]
    records.append(dict(cycle=cycle, trigger=trigger, stamp=stamp, timeText=f'{demo_date} 12:00:{cycle:02d} (模拟)',
                        line=lines[1], context=lines, isfPath=str(isf), screenshot=str(shot), csvPath=str(csv_path),
                        pdfPath=None, guiVersion=f'{version} (模拟)', scopeId='SIM-SCOPE-001',
                        scopeModel='SIMULATED OSCILLOSCOPE', serialMonitoring=True,
                        preset_frequency_hz=frequency, simulated_fault=fault))
metadata = dict(run_dir=str(run), stamp=stamp, records=records, simulation_only=True)
(run / 'demo-input.json').write_text(json.dumps(metadata, ensure_ascii=False, indent=2), encoding='utf-8')
print(run)
