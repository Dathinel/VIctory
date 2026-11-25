# One BCPack entropy-lane step

The shader translates the single-lane transition in Flamingo's public BCPack decoder. The caller delivers bytes, consumes output symbols, and schedules the lanes. Both byte and symbol fields use 256 for an empty slot. The table buffer contains 128 validated model rows as described in the shader. Use a valid width and a header whose alphabet reduction fits it.

At the fragment boundary, a byte can bring a temporary state to 2^24 while exhausting the payload. With reset clear the lane keeps its state from before that byte. With reset set the lane stores zero. At exactly 2^24 with payload bytes remaining, a pending byte stays latched.

The differential test compiles the original Rust Lane::step and bucket lookup from commit 5b2c0d07b08c5eab275d54e62a481276721991bc. It compares ten fields after each of eight steps in 260 synthetic traces. Three controls alter the carried state, reset behavior and retention of a pending byte during symbol emission. The test uses the synthetic Windows GPU runner described in the parent README.

    python verify.py PATH_TO_RUSTC PATH_TO_DXC PATH_TO_TEST_EXE FLAMINGO_CHECKOUT ../bcpack-bucket/bucket.hlsli ../bcpack-rans/decode-symbol.hlsli

The traces exercise the transition directly. They do not simulate the outer lane scheduler's byte budget, stalls, or record ordering.

