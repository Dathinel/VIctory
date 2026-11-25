// Preconditions: validated tables, 4-aligned starts, and slot < 65536. Each start is a uint, each window hint is a byte.
// The caller selects the model by passing its two row offsets in bytes.
uint bcpack_bucket_of(ByteAddressBuffer tables, uint starts, uint hints, uint slot) {
    uint window = slot >> 8;
    uint address = hints + window;
    uint bucket = (tables.Load(address & ~3u) >> (8 * (address & 3))) & 255u;
    while (tables.Load(starts + 4 * (bucket + 1)) <= slot)
        ++bucket;
    return bucket;
}
