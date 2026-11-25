use std::io::Write;
#[derive(Debug)]
struct Error(&'static str);
struct Tables {
    freq: [[u32; 9]; 128],
    start: [[u32; 10]; 128],
    bucket: [[u8; 256]; 128],
    recip: [[u64; 9]; 128],
}
fn main() {
    let mut t = Tables {
        freq: [[0; 9]; 128],
        start: [[65536; 10]; 128],
        bucket: [[0; 256]; 128],
        recip: [[0; 9]; 128],
    };
    let mut table = Vec::new();
    for m in 0..128 {
        let g = m / 16;
        let f = 1u32 << (15 - g);
        let mut at = 0;
        for b in 0..g + 2 {
            t.start[m][b] = at;
            t.freq[m][b] = f;
            t.recip[m][b] = (1u64 << 32).div_ceil(f as u64);
            at += f << b.saturating_sub(1);
        }
        assert_eq!(at, 65536);
        for w in 0..256 {
            t.bucket[m][w] = t.start[m][1..]
                .iter()
                .filter(|&&s| s <= w as u32 * 256)
                .count() as u8;
        }
        for v in t.start[m] {
            table.extend(v.to_le_bytes());
        }
        table.extend(t.bucket[m]);
        for v in t.freq[m] {
            table.extend(v.to_le_bytes());
        }
    }
    for (case, count, per, blocks) in [
        (0, 1, 1, 2),
        (1, 8, 2, 17),
        (2, 9, 3, 15),
        (3, 12, 1, 39),
        (4, 16, 4, 8),
        (5, 16, 2, 0),
        (6, 1, 1, 510),
        (7, 12, 1, 20),
        (8, 12, 2, 15),
        (9, 12, 1, 19),
        (10, 12, 2, 15),
        (11, 16, 2, 13),
        (12, 16, 2, 15),
    ] {
        let widths: Vec<u8> = if case >= 7 {
            let (w, p) = layout([1, 3, 4, 5, 6, 7][case - 7]);
            assert_eq!((w.len(), p), (count, per));
            w.to_vec()
        } else {
            (0..count).map(|j| ((j * 5 + case + 1) % 8) as u8).collect()
        };
        let mut rng = 359u32;
        let records: Vec<Vec<u8>> = (0..blocks)
            .map(|_| {
                (0..count * per)
                    .map(|k| {
                        rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
                        ((rng >> 16) & ((2u32 << widths[k / per]) - 1)) as u8
                    })
                    .collect()
            })
            .collect();
        let mut packed = stream(&t, &widths, per, &records);
        let valid_size = packed.len();
        packed.extend([0xa5; 11]);
        for cut in [false, true] {
            let size = if cut { valid_size / 2 } else { packed.len() };
            let mut cursor = 0;
            let mut out = vec![0xa5u8; count * per * blocks];
            let result = lanes(
                &t,
                &widths,
                per,
                blocks,
                |_| {
                    if cursor == size {
                        None
                    } else {
                        cursor += 1;
                        Some(packed[cursor - 1])
                    }
                },
                &mut out,
            );
            assert_eq!(result.is_ok(), !cut || blocks == 0);
            if !cut {
                assert_eq!(cursor, valid_size);
                assert!(cursor < size);
            }
            if result.is_ok() {
                assert_eq!(out, records.concat());
            }
            let mut input = Vec::new();
            for v in [count, per, blocks, size] {
                input.extend((v as u32).to_le_bytes());
            }
            input.extend(&table);
            for &v in &widths {
                input.extend((v as u32).to_le_bytes());
            }
            input.extend(&packed[..size]);
            input.resize(input.len().next_multiple_of(4), 0);
            let mut want = Vec::new();
            for v in [result.is_ok() as u32, cursor as u32] {
                want.extend(v.to_le_bytes());
            }
            for v in out {
                want.extend((v as u32).to_le_bytes());
            }
            let name = format!("case-{case}-{}", cut as u8);
            std::fs::write(format!("{name}.in"), input).unwrap();
            std::fs::File::create(format!("{name}.out"))
                .unwrap()
                .write_all(&want)
                .unwrap();
        }
    }
}
