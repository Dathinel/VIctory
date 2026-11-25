#[test]
fn bc3_stream_roundtrip() {
    let mut t = Tables::parse(&tables()).unwrap();
    let widths = [3, 3, 7, 5, 5, 5, 5, 4, 5, 4, 7, 7];
    for (i, v) in t.bc3.iter_mut().enumerate() {
        for (j, e) in v.ep.iter_mut().enumerate() {
            *e = ((i * 29 + j * 51) as u8) & [31, 63, 31, 31, 63, 31, 255, 255][j];
        }
        for (j, k) in v.ix.iter_mut().enumerate() {
            *k = ((i * 13 + j * 5 + j / 7) as u8) & if j < 16 { 7 } else { 3 };
        }
    }
    for (i, v) in t.rank3a.iter_mut().enumerate() {
        *v = ((i * 3 + 1) as u8 & 7) | ((((i / 8 + 2) as u8) & 7) << 4);
    }
    for (i, v) in t.rank3c.iter_mut().enumerate() {
        *v = (i as u8).rotate_left(3) ^ 0x6b;
    }
    let nibble =
        |v: u8, offset: u8| (0..4).fold(0, |acc, k| acc | ((v >> (2 * k + offset)) & 1) << k);
    for n in [0usize, 1, 2, 260] {
        let records: Vec<Vec<u8>> = (0..n)
            .map(|i| {
                let mut rec: Vec<u8> = (0..24)
                    .map(|j| ((73 * i + 29 * j + i * j + 11) & ((2 << widths[j / 2]) - 1)) as u8)
                    .collect();
                let (ep, ix): (u8, u8) = match i {
                    0 => (255, 0),
                    1 => (0, 1),
                    2 => (1, 2),
                    3 => (0, 255),
                    4 => (4, 4),
                    259 => (255, 1),
                    _ => ((i * 53 + 17) as u8, (i * 113 + 7) as u8),
                };
                let delta = ep.wrapping_sub(ix);
                let encoded = (delta << 1) ^ ((delta as i8 >> 7) as u8);
                rec[..4].copy_from_slice(&[
                    nibble(encoded, 0),
                    nibble(encoded, 1),
                    nibble(ix, 0),
                    nibble(ix, 1),
                ]);
                assert_eq!(references(&rec, true), (ep as usize, ix as usize));
                rec
            })
            .collect();
        let stream = stream(&t, &widths, 2, &records);
        for mode in ["full", "exact", "minus-one", "short"] {
            let short = mode == "short" || mode == "minus-one";
            let mut payload = stream.clone();
            if mode == "minus-one" {
                payload.truncate(stream.len().saturating_sub(1));
            } else if short {
                payload.truncate(stream.len() / 2);
            } else if mode == "full" {
                payload.extend([0xa5; 7]);
            }
            let mut input = (n as u32).to_le_bytes().to_vec();
            input.extend((payload.len() as u32).to_le_bytes());
            input.extend(t.rank3a);
            input.extend(t.rank3c);
            for entry in &t.bc3 {
                let mut block = [0; 16];
                bc3_pack(entry, &mut block);
                input.extend(block);
            }
            assert_eq!(input.len(), 4408);
            for m in 0..128 {
                for v in t.start[m] {
                    input.extend(v.to_le_bytes());
                }
                input.extend(t.bucket[m]);
                for v in t.freq[m] {
                    input.extend(v.to_le_bytes());
                }
            }
            for v in widths {
                input.extend(u32::from(v).to_le_bytes());
            }
            input.extend(&payload);
            input.resize(input.len().next_multiple_of(4), 0);
            let mut blocks = vec![0; n * 16];
            let result = decode(3, &payload, &mut blocks, &t);
            if mode != "minus-one" { assert_eq!(result.is_ok(), n != 0 && !short); }
            let mut bytes = payload.iter().copied();
            let mut scratch = vec![0; n * 24];
            let decoded = lanes(&t, &widths, 2, n, |_| bytes.next(), &mut scratch);
            let consumed = payload.len() - bytes.len();
            if !short {
                assert_eq!(scratch, records.concat());
                assert_eq!(consumed, stream.len());
            }
            assert_eq!(decoded.is_ok(), n == 0 || result.is_ok());
            blocks.extend(
                (if n == 0 {
                    2u32
                } else if result.is_err() {
                    3
                } else {
                    1
                })
                .to_le_bytes(),
            );
            blocks.extend((consumed as u32).to_le_bytes());
            for v in scratch {
                blocks.extend(u32::from(v).to_le_bytes());
            }

            std::fs::write(format!("{mode}-{n}.input"), input).unwrap();
            std::fs::write(format!("{mode}-{n}.expected"), blocks).unwrap();
        }
    }
}
