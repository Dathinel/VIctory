#include "../bc-alpha-pack.hlsli"

// SRV: block count followed by 40 bytes per block, matching ep[8], ix[32].
// UAV: packed BC3 blocks. The caller validates both extents. Dispatch one group.
[numthreads(64, 1, 1)]
void main(uint thread : SV_GroupIndex) {
    ByteAddressBuffer src = ResourceDescriptorHeap[0];
    RWByteAddressBuffer dst = ResourceDescriptorHeap[1];
    uint count = src.Load(0);
    for (uint block = thread; block < count; block += 64) {
        uint base = 4 + 40 * block;
        uint2 ep = src.Load2(base);
        uint2 alpha = bc_alpha_pack(src, ep.y >> 16, base + 8);
        uint c0 = ((ep.x & 255) << 11) | (((ep.x >> 8) & 255) << 5) | ((ep.x >> 16) & 255);
        uint c1 = ((ep.x >> 24) << 11) | ((ep.y & 255) << 5) | ((ep.y >> 8) & 255);
        uint colors = (c0 & 65535) | (c1 << 16);
        uint indices = 0;
        static const uint color_code[4] = {0, 2, 3, 1};
        for (uint word = 0; word < 4; ++word) {
            uint ranks = src.Load(base + 24 + 4 * word);
            for (uint byte = 0; byte < 4; ++byte) {
                uint rank = (ranks >> (8 * byte)) & 3;
                indices |= color_code[rank] << (8 * word + 2 * byte);
            }
        }
        dst.Store4(16 * block, uint4(alpha, colors, indices));
    }
}
