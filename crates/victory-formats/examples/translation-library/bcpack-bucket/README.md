# BCPack bucket lookup

Include `bucket.hlsli` in a compute shader and pass the byte offsets of the selected model's two rows in a ByteAddressBuffer. The starts row contains ten little-endian uints aligned to four bytes, including a 65536 sentinel after the last bucket. The hints row contains 256 bytes, one initial bucket per 256 slots. Supply a validated model and a slot from 0 through 65535.

The fixture checks two synthetic models against a linear interval search: a two-bucket model split at 257, and a nine-bucket model with eight transitions in its first window. It also compiles a strict-comparison mutant and expects the hardware assertion to fail.

On Windows, build the existing test runner described in the parent README (synthetic-tests/bc1-records.diff.txt). Run `python verify.py PATH_TO_DXC PATH_TO_TEST_EXE` with Python 3 in normal mode. The runner executes the compiled shader on hardware and WARP. This checks the bucket lookup independently of the rest of the entropy decoder.
