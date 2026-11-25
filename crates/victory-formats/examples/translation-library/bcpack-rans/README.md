# BCPack rANS symbol and state

This shader helper translates the symbol-producing branch of the BCPack lane decoder in victory-formats. After byte handling, the caller invokes it when the lane state exceeds 2^24. It returns the symbol in x and the next unsigned 32-bit state in y. Pass the byte offsets of the selected model's starts, window hints and frequencies. See the sibling bcpack-bucket example for the first two layouts. Frequencies are nine aligned 32-bit unsigned values from the validated model.

The shader uses unsigned division where the Rust implementation multiplies by a precomputed reciprocal. The fixture uses the full slot range of two synthetic models, one high word per slot, and checks both outputs. Symbol zero uses a one-smaller frequency and subtracts one from the rank. Slot zero is included as an arithmetic edge case. This test does not exercise fragment parsing or lane renormalization.

From this directory on Windows with Python 3, run:

    python compare-upstream.py PATH_TO_RUSTC FLAMINGO_CHECKOUT
    python verify.py PATH_TO_DXC PATH_TO_TEST_EXE ../bcpack-bucket/bucket.hlsli

The first check reads the pinned public Rust function and arithmetic branch from the checkout's Git history. The second uses the synthetic BC1 GPU runner described in the library README. Both compare with the same independently expanded symbol table. The fixture includes the first state above the decoding threshold. Negative controls change the zero-symbol frequency and the nonzero-symbol quotient; both must fail on hardware.
