import struct


def synthetic_models():
    tables = bytearray()
    expected = []
    for frequencies in ([257, 65279], [1] * 8 + [511]):
        starts = []
        symbols = []
        for b, f in enumerate(frequencies):
            starts.append(len(symbols))
            for _ in range(1 if b == 0 else 1 << (b - 1)):
                symbol = 0 if not symbols else symbols[-1][0] + 1
                symbols.extend((symbol, f, rank) for rank in range(f))
        assert len(symbols) == 65536
        tables.extend(struct.pack("<10I", *(starts + [65536] * (10 - len(starts)))))
        tables.extend(sum(s <= w * 256 for s in starts[1:]) for w in range(256))
        tables.extend(struct.pack("<9I", *(list(frequencies) + [0] * (9 - len(starts)))))
        row = []
        for slot, (symbol, f, rank) in enumerate(symbols):
            high = 256 if slot == 1 else 257 + (slot * 40503 % 65279)
            next_state = high * f + rank
            if symbol == 0:
                next_state -= high + 1
            row.append((symbol, next_state & 0xffffffff))
        expected.append(row)
    words = b"".join(struct.pack("<4I", *a, *b) for a, b in zip(*expected))
    return tables, words
