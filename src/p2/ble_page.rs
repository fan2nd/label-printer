//! Build the P2 BLE page before the transport slices it at ATT write boundaries.
use super::{density_command, validate_bitmap, SETUP, STRIDE};

const RUNS: [usize; 16] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 24, 36, 48, 120];

fn number(out: &mut Vec<u8>, value: usize) {
    if value >= 192 {
        out.push(0xc0 | (value >> 8) as u8);
    }
    out.push(value as u8);
}

// RLE5D encodes the XOR against the previous printed row, MSB first.
// Trailing zeros are implicit. Each symbol carries a colour and a run index.
fn difference(previous: &[u8], row: &[u8]) -> Vec<u8> {
    let mut delta = [0u8; STRIDE];
    for ((dst, a), b) in delta.iter_mut().zip(previous).zip(row) {
        *dst = a ^ b;
    }
    let mut symbols = Vec::new();
    if let Some(last) = delta.iter().rposition(|b| *b != 0) {
        let end = (last + 1) * 8 - delta[last].trailing_zeros() as usize;
        let bit = |i: usize| (delta[i / 8] >> (7 - i % 8)) & 1;
        let mut start = 0;
        while start < end {
            let colour = bit(start);
            let mut next = start + 1;
            while next < end && bit(next) == colour {
                next += 1;
            }
            let mut length = next - start;
            while length > 0 {
                let index = RUNS.partition_point(|&run| run <= length) - 1;
                symbols.push((colour << 4) | index as u8);
                length -= RUNS[index];
            }
            start = next;
        }
    }
    let mut out = vec![0x1f, 0x2d];
    number(&mut out, symbols.len());
    let mut buffer = 0u16;
    let mut bits = 0;
    for symbol in symbols {
        buffer = (buffer << 5) | u16::from(symbol);
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    if bits > 0 {
        out.push((buffer << (8 - bits)) as u8);
    }
    out
}

pub(super) fn encode(page: &[u8], density: u8) -> Result<Vec<u8>, String> {
    validate_bitmap(page)?;
    let mut out = SETUP.concat();
    out.extend(density_command(density)?);
    let mut rows = page.chunks_exact(STRIDE).peekable();
    let mut previous = None;
    while let Some(row) = rows.next() {
        let mut count = 1;
        while rows.next_if_eq(&row).is_some() {
            count += 1;
        }
        if let Some(raw) = super::ink_command(row) {
            let diff = previous.map(|base| difference(base, row));
            out.extend(diff.filter(|d| d.len() < raw.len()).unwrap_or(raw));
            if count > 1 {
                out.extend([0x1f, 0x2e]);
                number(&mut out, count - 2);
            }
            previous = Some(row);
        } else {
            while count > 0 {
                let feed = count.min(255);
                out.extend([0x1b, 0x4a, feed as u8]);
                count -= feed;
            }
            previous = None;
        }
    }
    out.push(0x0c);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_number(input: &mut &[u8]) -> usize {
        let first = input[0] as usize;
        *input = &input[1..];
        if first < 192 {
            return first;
        }
        let value = (first & 63) * 256 + input[0] as usize;
        *input = &input[1..];
        value
    }

    fn restored(page: &[u8], capacity: usize) -> (Vec<u8>, usize) {
        let stream = encode(page, 1).unwrap();
        let wire = stream
            .chunks(capacity)
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        let mut cmd = &wire[SETUP.concat().len() + density_command(1).unwrap().len()..];
        let mut output = Vec::new();
        let mut last = [0; STRIDE];
        let mut differences = 0;
        while cmd != [0x0c] {
            let op = (cmd[0], cmd[1]);
            cmd = &cmd[2..];
            match op {
                (0x1b, 0x4a) => {
                    last.fill(0);
                    output.extend(last.repeat(read_number(&mut cmd)));
                }
                (0x1f, 0x2e) => output.extend(last.repeat(read_number(&mut cmd) + 1)),
                (0x1f, 0x2b) => {
                    let offset = read_number(&mut cmd);
                    let length = read_number(&mut cmd);
                    last.fill(0);
                    last[offset..offset + length].copy_from_slice(&cmd[..length]);
                    cmd = &cmd[length..];
                    output.extend(last);
                }
                (0x1f, 0x2d) => {
                    differences += 1;
                    let count = read_number(&mut cmd);
                    let mut dot = 0;
                    for i in 0..count {
                        let sym = (i * 5..i * 5 + 5).fold(0usize, |acc, bit| {
                            acc * 2 + usize::from((cmd[bit / 8] >> (7 - bit % 8)) & 1)
                        });
                        let end = dot + RUNS[sym & 15];
                        assert!(end <= STRIDE * 8);
                        if sym >= 16 {
                            for pos in dot..end {
                                last[pos / 8] ^= 0x80 >> (pos % 8);
                            }
                        }
                        dot = end;
                    }
                    cmd = &cmd[(count * 5).div_ceil(8)..];
                    output.extend(last);
                }
                _ => panic!("unexpected command {op:?}"),
            }
        }
        (output, differences)
    }

    #[test]
    fn changes_after_full_repeated_and_blank_rows_round_trip() {
        let mut row = [0xa5; STRIDE];
        let mut page = row.repeat(3);
        for i in 0..STRIDE {
            row[i] ^= 0x11;
            page.extend(row);
        }
        page.extend([0; STRIDE]);
        page.extend(row);
        row[STRIDE - 1] ^= 1;
        page.extend(row);
        for capacity in [20, 182, 511] {
            let (actual, differences) = restored(&page, capacity);
            assert_eq!(actual, page);
            assert!(differences >= STRIDE);
        }
    }
}
