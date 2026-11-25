import os
import struct
import subprocess
import sys
from pathlib import Path
from tempfile import TemporaryDirectory

here = Path(__file__).resolve().parent
rustc, dxc, runner, repo, bucket, decode = (Path(a).resolve() for a in sys.argv[1:])
base = "5b2c0d07b08c5eab275d54e62a481276721991bc"
source = subprocess.check_output(["git", "-C", repo, "show", base + ":crates/victory-formats/src/bcpack.rs"], text=True)
low = next(line for line in source.splitlines() if line.startswith("const LOW:"))
lane = source[source.index("#[derive(", source.index("// Lanes.")):source.index("/// The bucket holding")]
lookup = source[source.index("fn bucket_of("):].split("\n}", 1)[0] + "\n}"
with TemporaryDirectory() as folder:
    tmp = Path(folder)
    (tmp/"oracle.rs").write_text(low+"\n"+lane+lookup+"\n"+(here/"trace.rs").read_text())
    subprocess.run([rustc, "--edition=2024", "-O", "oracle.rs"], cwd=tmp, check=True)
    trace = subprocess.check_output([tmp/"oracle.exe"])
    rows = 332*128
    stride = 40+8*44
    assert len(trace) == rows+260*stride
    for count in (2,260):
        (tmp/f"input-{count}.bin").write_bytes(struct.pack("<I",count)+trace[:rows+count*stride])
        (tmp/f"expected-{count}.bin").write_bytes(struct.pack("<II",8,0)*count)
    for name,header in [("bcpack-bucket",bucket),("bcpack-rans",decode)]:
        (tmp/name).mkdir()
        (tmp/name/header.name).write_bytes(header.read_bytes())
    src=tmp/"lane"
    src.mkdir()
    (src/"check.hlsl").write_bytes((here/"check.hlsl").read_bytes())
    for mutant in ("","carry","reset","latch"):
        shader=(here/"lane-step.hlsli").read_text()
        if mutant == "latch":
            shader=shader.replace("lane.x = decoded.y;", "lane.x = decoded.y; lane.latch = 256;")
        elif mutant:
            assert shader.count("if (lane.reset != 0) lane.x = 0;")==1
            replacement="lane.x = lane.reset != 0 ? 0 : x;" if mutant=="carry" else "lane.x = lane.x;"
            shader=shader.replace("if (lane.reset != 0) lane.x = 0;",replacement)
        (src/"lane-step.hlsli").write_text(shader)
        subprocess.run([dxc,"-T","cs_6_6","-E","main","-O3","-Fo",tmp/"bc1-records.dxil",src/"check.hlsl"],check=True)
        trial=subprocess.run([runner,"--exact","emit::valu_tests::vic117_bc1_gpu","--nocapture"],cwd=tmp,env=dict(os.environ,BC1_TEST_DIR=folder),capture_output=True,text=True)
        log=trial.stdout+trial.stderr
        if mutant:
            failed_batch = 2
            assert trial.returncode and f"assertion `left == right` failed: hardware: {failed_batch}" in log.splitlines(),log
            print(mutant,"mutant rejected on hardware")
        else:
            assert trial.returncode==0 and all(f"{d}: 260 blocks matched" in log for d in ("hardware","warp")),log
            print(log)
            print("260 traces x 8 sequential transitions x 10 state fields per backend")


