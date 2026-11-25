#include "../bcpack-schedule/lanes.hlsli"
#include "../bc4-reconstruct.hlsli"

// A single invocation owns the output, status and scratch for one stream.
[numthreads(1,1,1)]
void main(uint3 id : SV_DispatchThreadID) {
    if (any(id != 0)) return;
    ByteAddressBuffer src = ResourceDescriptorHeap[0];
    RWByteAddressBuffer dst = ResourceDescriptorHeap[1];
    uint count = src.Load(0);
    if (count == 0) { dst.Store2(0, uint2(2, 0)); return; }
    uint status = 8 * count;
    uint scratch = status + 8;
    uint rows = 2112;
    uint widths = rows + 128 * 332;
    uint start = widths + 12 * 4;
    uint cursor = start;
    bool ok = bcpack_lanes(src, dst, rows, widths, 12, 1, count, cursor, start + src.Load(2108), scratch);
    dst.Store2(status, uint2(ok ? 1 : 3, cursor - start));
    if (!ok) return;
    DeviceMemoryBarrier();
    for (uint i = 0; i < count; ++i) {
        uint3 record;
        for (uint j = 0; j < 3; ++j) {
            uint4 b = dst.Load4(scratch + 48 * i + 16 * j);
            record[j] = b.x | (b.y << 8) | (b.z << 16) | (b.w << 24);
        }
        bc4_reconstruct(src, dst, i, record);
        DeviceMemoryBarrier();
    }
}
