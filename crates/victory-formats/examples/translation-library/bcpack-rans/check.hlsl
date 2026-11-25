#include "decode-symbol.hlsli"
[numthreads(64,1,1)]
void main(uint3 id : SV_DispatchThreadID) {
    if (id.x >= 64 || id.y != 0 || id.z != 0) return;
    ByteAddressBuffer src = ResourceDescriptorHeap[0];
    RWByteAddressBuffer dst = ResourceDescriptorHeap[1];
    for (uint chunk = id.x; chunk < src.Load(0); chunk += 64) {
        uint matches = 0;
        for (uint k = 0; k < 128; ++k) {
            uint slot = chunk * 128 + k;
            uint hi = slot == 1 ? 256 : 257 + ((slot * 40503u) % 65279u);
            uint x = (hi << 16) | slot;
            uint4 expected = src.Load4(668 + 16 * slot);
            uint2 a = bcpack_decode_symbol(src, 4, 44, 300, x);
            uint2 b = bcpack_decode_symbol(src, 336, 376, 632, x);
            matches += uint(all(a == expected.xy) && all(b == expected.zw));
        }
        dst.Store(chunk * 4, matches);
    }
}
