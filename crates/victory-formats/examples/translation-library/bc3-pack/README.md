# Pack a BC3 block on the GPU

This compute shader translates victory-formats' bc3_pack. It takes the eight endpoint bytes and 32 unpacked pixel ranks of a reconstructed block. It writes a sixteen-byte BC3 block: eight alpha bytes followed by eight color bytes.

Bind a raw SRV at descriptor 0 and a raw UAV at descriptor 1, and dispatch one group. The input begins with a little-endian uint block count followed by 40 bytes per block. Bytes 0..5 of each record hold R, G, B for the two RGB565 endpoints, with 5, 6 and 5 bits respectively. Bytes 6..7 hold the two biased alpha endpoints. The sixteen alpha ranks start at offset 8 and the sixteen color ranks at 24. The output buffer needs 16 bytes per block. The host validates buffer extents and 32-bit byte offsets before dispatch; 4 + 40 times the block count must fit in the input SRV. Threads process independent blocks. The shared alpha helper also serves the standalone BC4 alpha example.

On Windows, build the sized test runner as described in [the BC4 stream example](../bc4-stream/README.md), then run:

    python verify.py PATH_TO_RUSTC PATH_TO_DXC PATH_TO_SIZED_TEST_EXE FLAMINGO_CHECKOUT

The test compiles the packing functions from the pinned Rust source, checks the first block against a hand-derived vector, and compares every output byte on hardware and WARP. The BC4 regression uses nonzero padding bytes. Cases at 64 and 65 blocks cover the workgroup boundary. The 449-block batch includes 192 cases isolating every defined rank at each of the sixteen pixel positions, separately for alpha and color. Negative controls drop the alpha index bits crossing the word boundary or fetch color ranks from the alpha region. This example starts after entropy decoding and prediction.
