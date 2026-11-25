#include "../bcpack-bucket/bucket.hlsli"

// Decoding branch only: the caller has handled renormalization and x > 2^24.
// Frequencies are a 4-aligned row of nine uints in the validated model.
uint2 bcpack_decode_symbol(ByteAddressBuffer tables, uint starts, uint hints, uint frequencies, uint x) {
    uint slot = x & 65535;
    uint b = bcpack_bucket_of(tables, starts, hints, slot);
    uint f = tables.Load(frequencies + 4 * b);
    uint hi = x >> 16;
    if (b == 0)
        return uint2(0, hi * (f - 1) + slot - 1);

    uint offset = slot - tables.Load(starts + 4 * b);
    uint quotient = offset / f;
    uint symbol = (1u << (b - 1)) + quotient;
    return uint2(symbol, hi * f + offset - quotient * f);
}
