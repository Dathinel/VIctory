#include "../bcpack-schedule/lanes.hlsli"
#include "../../bc5/bc5-references.hlsli"
#include "../bc-alpha-pack.hlsli"

// Host supplies validated extents and model rows; dispatch one group per stream.
uint table_byte(ByteAddressBuffer src, uint base, uint index) {
    return (src.Load(base + (index & ~3u)) >> (8 * (index & 3))) & 255;
}
uint4 predictor(ByteAddressBuffer src, RWByteAddressBuffer dst, uint block, uint back) {
    if (back == 0) return uint4(0x8080, 0, 0, 0);
    if (back <= block) return dst.Load4(16 * (block - back));
    return src.Load4(328 + 16 * (back - block - 1));
}
uint residual(uint value) {
    return (value >> 1) ^ (0u - (value & 1));
}
uint alpha_code(uint2 bits, uint p) {
    uint shift = 16 + 3*p;
    if (shift == 31) return ((bits.x >> 31) | (bits.y << 1)) & 7;
    if (shift < 32) return (bits.x >> shift) & 7;
    return (bits.y >> (shift - 32)) & 7;
}
[numthreads(1,1,1)]
void main(uint3 id : SV_DispatchThreadID) {
    if (any(id != 0)) return;
    ByteAddressBuffer src = ResourceDescriptorHeap[0];
    RWByteAddressBuffer dst = ResourceDescriptorHeap[1];
    uint n = src.Load(0);
    uint status = 16 * n;
    if (n == 0) { dst.Store2(status,uint2(2,0)); return; }
    uint rows = 328 + 255 * 16;
    uint widths = rows + 128 * 332;
    uint start = widths + 48;
    uint cursor = start;
    uint scratch = status + 8;
    bool ok = bcpack_lanes(src,dst,rows,widths,12,2,n,cursor,start+src.Load(4),scratch);
    dst.Store2(status,uint2(ok ? 1 : 3,cursor-start));
    if (!ok) return;
    DeviceMemoryBarrier();
    const uint color_rank[4] = {0,3,1,2};
    const uint color_code[4] = {0,2,3,1};
    for (uint i = 0; i < n; ++i) {
        uint rec = scratch + 96*i;
        uint2 back = bc5_references(dst.Load4(rec));
        uint4 ep = predictor(src,dst,i,back.x);
        uint4 ix = predictor(src,dst,i,back.y);
        uint2 ad = dst.Load2(rec + 4*4);
        uint a0 = ((ep.x & 255) + residual(ad.x)) & 255;
        uint a1 = (((ep.x >> 8) & 255) + residual(ad.y)) & 255;
        uint rgb = 0;
        for (uint e = 0; e < 2; ++e) {
            uint ref_color = ep.z >> (16*e);
            uint r = ((ref_color >> 11) + residual(dst.Load(rec + 4*(14+e)))) & 31;
            uint g = ((ref_color >> 5) + residual(dst.Load(rec + 4*(16+e)))) & 63;
            uint b = (ref_color + residual(dst.Load(rec + 4*(18+e)))) & 31;
            rgb |= ((r << 11) | (g << 5) | b) << (16*e);
        }
        uint4 alpha_ranks = 0;
        uint color = 0;
        for (uint p = 0; p < 16; ++p) {
            uint pair = table_byte(src,8,dst.Load(rec + 4*(6+p/2)) & 63);
            uint delta = (pair >> (4*(p&1))) & 7;
            uint old = alpha_code(ix.xy,p);
            uint rank = old == 0 ? 0 : (old == 1 ? 7 : old - 1);
            alpha_ranks[p/4] |= ((rank+delta)&7) << (8*(p&3));
            uint x = p&3, y = p/4;
            uint quad = (y/2)*2 + x/2;
            uint ranks = table_byte(src,72,dst.Load(rec + 4*(20+quad)));
            uint increment = (ranks >> (2*((y&1)*2 + (x&1)))) & 3;
            uint previous = color_rank[(ix.w >> (2*p)) & 3];
            color |= color_code[(previous+increment)&3] << (2*p);
        }
        uint2 alpha = bc_alpha_pack_words((a0 | (a1<<8)) ^ 0x8080,alpha_ranks);
        dst.Store4(16*i,uint4(alpha,rgb,color));
        DeviceMemoryBarrier();
    }
}
