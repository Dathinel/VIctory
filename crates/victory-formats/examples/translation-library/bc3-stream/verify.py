import os
import re
import subprocess
import sys
from pathlib import Path
from tempfile import TemporaryDirectory

here = Path(__file__).resolve().parent
rustc, dxc, runner, repo = (Path(s).resolve() for s in sys.argv[1:])
base = "5b2c0d07b08c5eab275d54e62a481276721991bc"
source = subprocess.check_output(["git","-C",repo,"show",base+":crates/victory-formats/src/bcpack.rs"],text=True).rstrip()
assert source.endswith("}")
with TemporaryDirectory() as folder:
    tmp=Path(folder)
    (tmp/"bcpack.rs").write_text(source[:-1]+(here/"cases.rs").read_text()+"\n}")
    (tmp/"bcpack-tables.bin").write_bytes(b"")  # builtin() is unused.
    subprocess.run([rustc,"--edition=2024","--test","bcpack.rs"],cwd=tmp,check=True)
    subprocess.run([tmp/"bcpack.exe","--exact","tests::bc3_stream_roundtrip","--nocapture"],cwd=tmp,check=True)
    for mutation in ("","predictor","quad"):
        code=(here/"decode.hlsl").read_text()
        if mutation=="predictor":
            assert code.count("predictor(src,dst,i,back.x)")==1
            code=code.replace("predictor(src,dst,i,back.x)","predictor(src,dst,i,back.y)")
        if mutation=="quad":
            assert code.count("(y/2)*2 + x/2")==1
            code=code.replace("(y/2)*2 + x/2","(x/2)*2 + y/2")
        shader=tmp/"main.hlsl"
        shader.write_text(code)
        subprocess.run([dxc,"-T","cs_6_6","-E","main","-O3","-I",here,"-Fo",tmp/"bc1-records.dxil",shader],check=True)
        cases=[("full",1)] if mutation else [("full",1),("full",2),("full",260),("exact",260),("minus-one",260),("short",260)]
        for mode,n in cases:
            for slot,count in [(2,0),(260,n)]:
                for suffix,stem in [("input","input"),("expected","expected")]:
                    data=(tmp/f"{mode}-{count}.{suffix}").read_bytes()
                    if suffix=="expected": assert len(data)==112*count+8
                    (tmp/f"{stem}-{slot}.bin").write_bytes(data)
            trial=subprocess.run([runner,"--exact","emit::valu_tests::bc4_sized_gpu","--ignored","--nocapture"],cwd=tmp,env=dict(os.environ,BC1_TEST_DIR=folder),text=True,capture_output=True)
            log=trial.stdout+trial.stderr
            if mutation:
                mismatch=re.search(r"hardware: mismatch in slot 260 at byte (\d+)",log)
                assert trial.returncode and "hardware: slot 2 matched 8 bytes and guard" in log and mismatch and int(mismatch[1]) < 16,log
                print(mutation,"control first differs at byte",mismatch[1])
            else:
                assert trial.returncode==0 and all(f"{dev}: slot 260 matched {112*n+8} bytes and guard" in log for dev in ("hardware","warp")),log
                print(mode,n,"BC3 blocks, cursor, status and scratch matched on hardware and WARP")

