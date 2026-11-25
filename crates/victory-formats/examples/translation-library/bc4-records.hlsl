// SRV: count, 64 rank bytes, 255 packed dictionary blocks, 12 bytes per record.
// UAV: 8 bytes per output. Caller validates extents. One stream per dispatch.
#include "bc4-reconstruct.hlsli"

[numthreads(1, 1, 1)]
void main(uint3 id : SV_DispatchThreadID) {
    if (any(id != 0)) return;
    ByteAddressBuffer src = ResourceDescriptorHeap[0];
    RWByteAddressBuffer dst = ResourceDescriptorHeap[1];
    uint count = src.Load(0);
    for (uint i = 0; i < count; ++i) {
        uint offset = 2108 + 12 * i;
        bc4_reconstruct(src, dst, i, src.Load3(offset));
        DeviceMemoryBarrier();
    }
}
