#include "../bcpack-rans/decode-symbol.hlsli"

// Input is a contiguous fragment payload. Its validated model is selected by
// the caller. The byte cursor and state survive from one symbol to the next.
bool bcpack_next_symbol(ByteAddressBuffer src, uint starts, uint hints, uint freq,
                        inout uint cursor, uint end, inout uint state, out uint symbol) {
    symbol = 0;
    while (state < (1u << 24) && cursor < end) {
        uint word = src.Load(cursor & ~3u);
        state = (state << 8) | ((word >> (8 * (cursor & 3))) & 255);
        ++cursor;
    }
    if (state <= (1u << 24)) return false;
    uint2 result = bcpack_decode_symbol(src, starts, hints, freq, state);
    symbol = result.x;
    state = result.y;
    return true;
}
