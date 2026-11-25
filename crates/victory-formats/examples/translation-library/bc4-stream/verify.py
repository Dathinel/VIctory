import os
import subprocess
import sys
from pathlib import Path
from tempfile import TemporaryDirectory

here = Path(__file__).resolve().parent
rustc, dxc, runner, repo = (Path(s).resolve() for s in sys.argv[1:])
base = "5b2c0d07b08c5eab275d54e62a481276721991bc"
source = subprocess.check_output(["git", "-C", repo, "show", base + ":crates/victory-formats/src/bcpack.rs"], text=True).rstrip()
assert source.endswith("}")
with TemporaryDirectory() as folder:
    tmp = Path(folder)
    (tmp/"bcpack.rs").write_text(source[:-1] + (here/"cases.rs").read_text() + "\n}")
    # builtin() is unused. Every fixture passes its own synthetic tables.
    (tmp/"bcpack-tables.bin").write_bytes(b"")
    subprocess.run([rustc,"--edition=2024","--test","bcpack.rs"],cwd=tmp,check=True)
    subprocess.run([tmp/"bcpack.exe","--exact","tests::synthetic_bc4_streams","--nocapture"],cwd=tmp,check=True)
    for stage, shader, batches in [
        ("records",here.parent/"bc4-records.hlsl",[(2,260)]),
        ("stream",here/"decode.hlsl",[(0,2),(0,36)]),
        ("short",here/"decode.hlsl",[(0,36)]),
        ("swapped",here/"decode.hlsl",[(0,2)]),
    ]:
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
                assert len(want)<=8*cap
                (tmp/f"input-{cap}.bin").write_bytes(data)
                (tmp/f"expected-{cap}.bin").write_bytes(want.ljust(8*cap,b"\0"))
            trial=subprocess.run([runner,"--exact","emit::valu_tests::vic117_bc1_gpu","--nocapture"],cwd=tmp,env=dict(os.environ,BC1_TEST_DIR=folder),text=True,capture_output=True)
            log=trial.stdout+trial.stderr
            if stage=="swapped":
                assert trial.returncode and "assertion `left == right` failed: hardware: 260" in log.splitlines(),log
                print("Swapping adjacent scratch symbols failed the two-block comparison on hardware.")
            else:
                assert trial.returncode==0 and all(f"{device}: 260 blocks matched" in log for device in ("hardware","warp")),log
                print(stage,batch,"matched on hardware and WARP; tail checks passed.")
