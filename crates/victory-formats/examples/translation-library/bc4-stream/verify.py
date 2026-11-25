import os
import re
import subprocess
import sys
from pathlib import Path
from tempfile import TemporaryDirectory

here = Path(__file__).resolve().parent
rustc, dxc, runner, repo = (Path(s).resolve() for s in sys.argv[1:])
base = "5b2c0d07b08c5eab275d54e62a481276721991bc"
source = subprocess.check_output(["git", "-C", repo, "show", base + ":crates/victory-formats/src/bcpack.rs"], text=True).rstrip()
assert source.endswith("}")
tests = subprocess.check_output([runner, "--list"], text=True)
sized_name = "emit::valu_tests::bc4_sized_gpu"
sized = f"{sized_name}: test" in tests.splitlines()
test_name = sized_name if sized else "emit::valu_tests::vic117_bc1_gpu"
assert f"{test_name}: test" in tests.splitlines(), "expected a BC4 or BC1 fixture runner"

with TemporaryDirectory() as folder:
    tmp = Path(folder)
    (tmp/"bcpack.rs").write_text(source[:-1] + (here/"cases.rs").read_text() + "\n}")
    # builtin() is unused. Every fixture passes its own synthetic tables.
    (tmp/"bcpack-tables.bin").write_bytes(b"")
    subprocess.run([rustc,"--edition=2024","--test","bcpack.rs"],cwd=tmp,check=True)
    subprocess.run([tmp/"bcpack.exe","--exact","tests::synthetic_bc4_streams","--nocapture"],cwd=tmp,check=True)
    stages = [
        ("records",here.parent/"bc4-records.hlsl",[(2,260)]),
        ("stream",here/"decode.hlsl",[(0,2),(0,36)]),
        ("short",here/"decode.hlsl",[(0,36)]),
        ("swapped",here/"decode.hlsl",[(0,2)]),
    ]
    if sized:
        stages.extend([
            ("stream", here/"decode.hlsl", [(2,260)]),
            ("short", here/"decode.hlsl", [(2,260)]),
            ("history", here/"decode.hlsl", [(2,260)]),
        ])
    for stage, shader, batches in stages:
        if stage == "history":
            header = (here.parent/"bc4-reconstruct.hlsli").read_text()
            lookup = "8 * (i - back)"
            assert header.count(lookup) == 1
            (tmp/"history.hlsli").write_text(header.replace(lookup, "8 * (i - (back == 255 ? 1 : back))"))
            code = shader.read_text().replace('"../bc4-reconstruct.hlsli"', '"history.hlsli"')
            shader = tmp/"history.hlsl"
            shader.write_text(code)
        if stage=="swapped":
            code=shader.read_text()
            original="b.x | (b.y << 8)"
            assert code.count(original)==1
            code=code.replace(original,"b.y | (b.x << 8)")
            shader=tmp/"swapped.hlsl"
            shader.write_text(code)
        subprocess.run([dxc,"-T","cs_6_6","-E","main","-O3","-I",here,"-Fo",tmp/"bc1-records.dxil",shader],check=True)
        for batch in batches:
            for cap,n in zip((2,260),batch):
                prefix="input" if stage=="records" else "stream"
                expected="expected" if stage=="records" else "combined"
                if stage=="short" and n:
                    prefix,expected="short-stream","short-combined"
                data=(tmp/f"{prefix}-{n}.bin").read_bytes()
                want=(tmp/f"{expected}-{n}.bin").read_bytes()
                expected_bytes = 8*n if stage == "records" else 56*n + 8
                assert len(want) == expected_bytes
                if not sized:
                    assert len(want) <= 8*cap
                    want = want.ljust(8*cap, b"\0")
                (tmp/f"input-{cap}.bin").write_bytes(data)
                (tmp/f"expected-{cap}.bin").write_bytes(want)
            trial=subprocess.run([runner,"--exact",test_name,"--ignored","--nocapture"] if sized else [runner,"--exact",test_name,"--nocapture"],cwd=tmp,env=dict(os.environ,BC1_TEST_DIR=folder),text=True,capture_output=True)
            log=trial.stdout+trial.stderr
            if stage == "history":
                mismatch = re.search(r"hardware: mismatch in slot 260 at byte (\d+)", log)
                assert trial.returncode and mismatch and 259*8 <= int(mismatch[1]) < 260*8, log
                print("Wrong distance-255 history reference first differs in block 259, byte", mismatch[1])
            elif stage == "swapped":
                marker = "hardware: mismatch in slot 260 at byte" if sized else "assertion `left == right` failed: hardware: 260"
                assert trial.returncode and marker in log, log
                if sized:
                    mismatch = re.search(r"hardware: mismatch in slot 260 at byte (\d+)", log)
                    assert mismatch and int(mismatch[1]) < 2*8, log
                print("Swapping adjacent scratch symbols failed the two-block comparison on hardware.")
            else:
                for device in ("hardware", "warp"):
                    marker = f"{device}: slot 260 matched {expected_bytes} bytes and guard" if sized else f"{device}: 260 blocks matched"
                    assert trial.returncode == 0 and marker in log, log
                print(stage,batch,"matched on hardware and WARP; tail checks passed.")
