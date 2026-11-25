# Serial BCPack lane scheduler

This HLSL example translates the scheduler in victory-formats. One invocation reads the shared stream in lane order, accepts up to eight bytes per round, and delivers symbols in block, lane, and position order. It reuses the sibling BCPack lane and rANS helpers. Its output uses one uint per symbol. It is a reference implementation; throughput has not been measured.

Pass 1 to 16 widths, a positive symbols-per-lane count, the requested block count and validated model tables. The host must validate all table, stream and output extents, including four-byte padding for the final input load, and ensure byte offsets and intermediate products fit in uint. The output UAV must accommodate all requested records. On an early end the function returns false and leaves already delivered symbols in place. Initialize output before comparing a partial result.

On Windows, run this from the example directory with a Flamingo checkout containing the pinned base, rustc, DXC and the synthetic GPU test executable described in the parent README:

    python verify.py PATH_TO_RUSTC PATH_TO_DXC PATH_TO_TEST_EXE FLAMINGO_CHECKOUT ../bcpack-bucket/bucket.hlsli ../bcpack-rans/decode-symbol.hlsli ../bcpack-lane/lane-step.hlsli

The test compiles Flamingo's Rust scheduler and its synthetic fragment and stream generators. For complete streams, Rust checks the decoded records against the generated symbols. For half-length inputs the test uses the Rust decoder's status, consumed bytes and partial output as the reference. Complete streams have eleven extra bytes that Rust and the shader must leave unread. The zero-block case produces no records. The cases span one, eight, nine, twelve and sixteen lanes. Six additional cases take their exact lane widths and symbols per lane from Flamingo for BC1, BC3, BC4, BC5, BC6H and BC7. These cases compare entropy records before block reconstruction. Two negative controls incorrectly fail the ninth request or a lane waiting for its peers.
