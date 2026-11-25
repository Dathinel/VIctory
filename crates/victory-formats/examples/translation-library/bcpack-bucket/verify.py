import os
import struct
import subprocess
import sys
from pathlib import Path
from tempfile import TemporaryDirectory

here = Path(__file__).resolve().parent
dxc, runner = (Path(arg).resolve() for arg in sys.argv[1:])
rows = [[0, 257], [0, 1, 2, 4, 8, 16, 32, 64, 128]]
def reference(row, slot):
    return sum(start <= slot for start in row[1:])

tables = bytearray()
for row in rows:
    tables.extend(struct.pack("<10I", *(row + [65536] * (10 - len(row)))))
    tables.extend(reference(row, window << 8) for window in range(256))
words = [reference(rows[0], s) | reference(rows[1], s) << 8 for s in range(65536)]
with TemporaryDirectory() as folder:
    tmp = Path(folder)
    for capacity, chunks in [(2, 4), (260, 512)]:
        n = chunks * 128
        (tmp / f"input-{capacity}.bin").write_bytes(struct.pack("<I", chunks) + tables + struct.pack(f"<{n}I", *words[:n]))
        (tmp / f"expected-{capacity}.bin").write_bytes(struct.pack("<I", 128) * chunks + bytes(8 * capacity - 4 * chunks))
    for mutate in [False, True]:
        source = (here / "bucket.hlsli").read_text()
        if mutate:
            assert source.count("<= slot") == 1
            source = source.replace("<= slot", "< slot")
        (tmp / "bucket.hlsli").write_text(source)
        (tmp / "check.hlsl").write_bytes((here / "check.hlsl").read_bytes())
        subprocess.run([dxc, "-T", "cs_6_6", "-E", "main", "-O3", "-Fo", "bc1-records.dxil", "check.hlsl"], cwd=tmp, check=True)
        trial = subprocess.run([runner, "--exact", "emit::valu_tests::vic117_bc1_gpu", "--nocapture"], cwd=tmp, env=dict(os.environ, BC1_TEST_DIR=folder), capture_output=True, text=True)
        log = trial.stdout + trial.stderr
        if mutate:
            assert trial.returncode and "assertion `left == right` failed: hardware: 2" in log, log
            print("Strict-boundary mutant failed the first hardware comparison.")
        else:
            assert trial.returncode == 0 and all(f"{d}: 260 blocks matched" in log for d in ("hardware", "warp")), log
            print(log)
            print("131072 slot/model comparisons per backend against a linear reference.")
