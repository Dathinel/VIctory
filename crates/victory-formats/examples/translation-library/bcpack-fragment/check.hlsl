#include "next-symbol.hlsli"
[numthreads(64,1,1)]
void main(uint3 id : SV_DispatchThreadID) {
    if (id.x >= 64 || id.y != 0 || id.z != 0) return;
    ByteAddressBuffer src = ResourceDescriptorHeap[0];
    RWByteAddressBuffer dst = ResourceDescriptorHeap[1];
    uint count = src.Load(0);
    for (uint i = id.x; i < count; i += 64) {
        uint4 item = src.Load4(4 + 16 * i);
        uint pos = item.y;
        uint end = pos + item.z;
        uint n = src.Load(item.w);
        uint state = 0;
        uint matches = 0;
        for (uint k = 0; k < n; ++k) {
            uint symbol;
            if (!bcpack_next_symbol(src, item.x, item.x+40, item.x+296, pos, end, state, symbol)) break;
            uint address = item.w + 4 + k;
            uint want = (src.Load(address & ~3u) >> ((address & 3)*8)) & 255;
            matches += uint(symbol == want);
        }
        uint extra;
        bool more = bcpack_next_symbol(src, item.x, item.x+40, item.x+296, pos, end, state, extra);
        dst.Store2(8*i, uint2(matches, uint(!more && pos == end && state == (1u<<24))));
    }
}
