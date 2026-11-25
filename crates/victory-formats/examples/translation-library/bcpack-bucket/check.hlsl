#include "bucket.hlsli"
[numthreads(64,1,1)]
void main(uint3 id : SV_DispatchThreadID) {
    if (id.x >= 64 || id.y != 0 || id.z != 0) return;
    ByteAddressBuffer src = ResourceDescriptorHeap[0];
    RWByteAddressBuffer dst = ResourceDescriptorHeap[1];
    for (uint chunk = id.x; chunk < src.Load(0); chunk += 64) {
        uint matches = 0;
        for (uint k = 0; k < 128; ++k) {
            uint slot = chunk * 128 + k;
            uint expected = src.Load(596 + 4 * slot);
            uint a = bcpack_bucket_of(src, 4, 44, slot);
            uint b = bcpack_bucket_of(src, 300, 340, slot);
            matches += uint((a | (b << 8)) == expected);
        }
        dst.Store(4 * chunk, matches);
    }
}
