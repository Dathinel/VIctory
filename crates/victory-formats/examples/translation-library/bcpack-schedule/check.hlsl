#include "lanes.hlsli"
[numthreads(1,1,1)]
void main(uint3 id : SV_DispatchThreadID) {
    if (any(id != 0)) return;
    ByteAddressBuffer src = ResourceDescriptorHeap[0];
    RWByteAddressBuffer dst = ResourceDescriptorHeap[1];
    uint4 args = src.Load4(0);
    uint start = 16 + 332 * 128 + 4 * args.x;
    uint cursor = start;
    for (uint i = 0; i < args.x * args.y * args.z; ++i) dst.Store(8 + 4*i, 0xa5);
    bool ok = bcpack_lanes(src,dst,16,16+332*128,args.x,args.y,args.z,cursor,start+args.w,8);
    dst.Store2(0,uint2(uint(ok),cursor-start));
}
