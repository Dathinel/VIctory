import os
import struct
import subprocess
import sys
from pathlib import Path
from tempfile import TemporaryDirectory

from fixture import synthetic_models

here = Path(__file__).resolve().parent
dxc, runner, bucket = (Path(p).resolve() for p in sys.argv[1:])
tables, words = synthetic_models()
with TemporaryDirectory() as folder:
    tmp = Path(folder)
    src = tmp / "shader"
    src.mkdir()
    inc = tmp / "bcpack-bucket"
    inc.mkdir()
    (inc / "bucket.hlsli").write_bytes(bucket.read_bytes())
    (src / "check.hlsl").write_bytes((here / "check.hlsl").read_bytes())
    for capacity, chunks in [(2, 4), (260, 512)]:
        (tmp / f"input-{capacity}.bin").write_bytes(struct.pack("<I", chunks) + tables + words[:chunks * 128 * 16])
        (tmp / f"expected-{capacity}.bin").write_bytes(struct.pack("<I", 128) * chunks + bytes(8 * capacity - 4 * chunks))
    for mutant in ["", "zero-symbol frequency", "nonzero-symbol quotient"]:
        code = (here / "decode-symbol.hlsli").read_text()
        if mutant == "nonzero-symbol quotient":
            assert code.count("offset / f") == 1
            code = code.replace("offset / f", "(offset + 1) / f")
        if mutant == "zero-symbol frequency":
            assert code.count("hi * (f - 1)") == 1
            code = code.replace("hi * (f - 1)", "hi * f")
        (src / "decode-symbol.hlsli").write_text(code)
        subprocess.run([dxc, "-T", "cs_6_6", "-E", "main", "-O3", "-Fo", tmp / "bc1-records.dxil", src / "check.hlsl"], check=True)
        result = subprocess.run([runner, "--exact", "emit::valu_tests::vic117_bc1_gpu", "--nocapture"], cwd=tmp, env=dict(os.environ, BC1_TEST_DIR=folder), capture_output=True, text=True)
        log = result.stdout + result.stderr
        if mutant:
            assert result.returncode and "assertion `left == right` failed: hardware: 2" in log, log
            print(f"Incorrect {mutant} rejected on hardware.")
        else:
            assert result.returncode == 0 and all(f"{d}: 260 blocks matched" in log for d in ("hardware", "warp")), log
            print(log, "131072 symbol/state pairs matched per backend.")
