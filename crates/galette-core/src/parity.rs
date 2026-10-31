//! Repairing a rip with the CUETools Database's parity data.
//!
//! The disc's audio, as 16-bit words from the start of track 1, is cut into
//! rows of [`ROW`] words (10 sectors). The words at one position of every
//! row form a Reed-Solomon codeword over GF(2^16) (polynomial 0x1100B), of
//! which the database keeps 16 (sometimes 8) syndromes:
//! `S_x = Σ w_r α^(x (n - r))` for `x < 16`, over rows `1..=n`, where `n` is
//! the number of whole rows minus 2: the first row and the last one or two
//! are left out, like the CRCs.
//!
//! Our syndromes XOR the database's are the syndromes of the errors in our
//! audio. Each column with at most 8 wrong words is corrected
//! (Berlekamp-Massey, Chien search, Forney), as CUETools does. Words known
//! to be doubtful (C2 errors) are erasures, which cost half: a column can
//! have `2 × errors + erasures ≤ 16`.

use std::sync::OnceLock;

/// Words per row: 10 sectors of stereo samples.
pub const ROW: usize = 2 * 10 * 588;
/// Most syndromes per column in the database.
pub const NPAR: usize = 16;
/// Non-zero elements of GF(2^16).
const ORDER: usize = 65_535;
const POLYNOMIAL: u32 = 0x1100B;

struct Field {
    /// α^i, twice over so that a sum of two logarithms needs no modulo.
    exp: Vec<u16>,
    log: Vec<u16>,
}

fn field() -> &'static Field {
    static FIELD: OnceLock<Field> = OnceLock::new();
    FIELD.get_or_init(|| {
        let mut exp = vec![0u16; 2 * ORDER];
        let mut log = vec![0u16; ORDER + 1];
        let mut x: u32 = 1;
        for i in 0..ORDER {
            exp[i] = x as u16;
            exp[ORDER + i] = x as u16;
            log[x as usize] = i as u16;
            x <<= 1;
            if x & 0x1_0000 != 0 {
                x ^= POLYNOMIAL;
            }
        }
        Field { exp, log }
    })
}

impl Field {
    fn mul(&self, a: u16, b: u16) -> u16 {
        if a == 0 || b == 0 { 0 } else { self.exp[self.log[a as usize] as usize + self.log[b as usize] as usize] }
    }

    /// `a × α^e`.
    fn mul_exp(&self, a: u16, e: usize) -> u16 {
        if a == 0 { 0 } else { self.exp[self.log[a as usize] as usize + e % ORDER] }
    }

    fn inv(&self, a: u16) -> u16 {
        self.exp[ORDER - self.log[a as usize] as usize]
    }
}

/// The syndromes of our audio, built while the tracks are ripped, in order.
pub struct Syndromes {
    /// `n`: rows `1..=n` are in the codewords.
    rows: usize,
    total: usize,
    /// Words given so far.
    position: usize,
    /// `[column * NPAR + x]`.
    syndromes: Vec<u16>,
    /// Rows 0, 1, n and n + 1: an entry stored at another offset has its
    /// rows shifted across those.
    edges: [Vec<u16>; 4],
}

impl Syndromes {
    /// For a disc of `samples` samples; `None` if it is too short or too
    /// long for the code.
    pub fn new(samples: usize) -> Option<Self> {
        let total = 2 * samples;
        let rows = (total / ROW).checked_sub(2).filter(|&n| n >= 1 && n + NPAR < ORDER)?;
        Some(Self {
            rows,
            total,
            position: 0,
            syndromes: vec![0; ROW * NPAR],
            edges: std::array::from_fn(|_| vec![0; ROW]),
        })
    }

    /// Adds the next track's audio (16-bit stereo, little endian).
    pub fn add(&mut self, audio: &[u8]) {
        let f = field();
        let mut rest = audio.as_chunks::<2>().0;
        while !rest.is_empty() {
            let (row, column) = (self.position / ROW, self.position % ROW);
            let (part, tail) = rest.split_at((ROW - column).min(rest.len()));
            for (edge, at) in self.edges.iter_mut().zip([0, 1, self.rows, self.rows + 1]) {
                if row == at {
                    for (dst, src) in edge[column..].iter_mut().zip(part) {
                        *dst = u16::from_le_bytes(*src);
                    }
                }
            }
            if (1..=self.rows).contains(&row) {
                // Horner's rule: S_x = S_x α^x + w, one row after another.
                for (j, word) in part.iter().enumerate() {
                    let word = u16::from_le_bytes(*word);
                    let column = &mut self.syndromes[(column + j) * NPAR..][..NPAR];
                    for (x, s) in column.iter_mut().enumerate() {
                        *s = f.mul_exp(*s, x) ^ word;
                    }
                }
            }
            self.position += part.len();
            rest = tail;
        }
    }

    /// Our syndromes as they would be for audio starting `offset` samples
    /// later (|offset| < 2940), the way an entry at that offset was stored.
    fn shifted(&self, offset: i32) -> Vec<u16> {
        let f = field();
        let n = self.rows;
        let [row_0, row_1, row_n, row_after] = &self.edges;
        let mut out = vec![0; ROW * NPAR];
        for j in 0..ROW {
            let c = j as isize + 2 * offset as isize;
            let out = &mut out[j * NPAR..][..NPAR];
            if (0..ROW as isize).contains(&c) {
                out.copy_from_slice(&self.syndromes[c as usize * NPAR..][..NPAR]);
            } else if c >= ROW as isize {
                // Rows 2..=n+1 of our column c - ROW.
                let c = c as usize - ROW;
                for (x, s) in out.iter_mut().enumerate() {
                    let ours = self.syndromes[c * NPAR + x] ^ f.mul_exp(row_1[c], x * (n - 1));
                    *s = f.mul_exp(ours, x) ^ row_after[c];
                }
            } else {
                // Rows 0..=n-1 of our column c + ROW.
                let c = (c + ROW as isize) as usize;
                for (x, s) in out.iter_mut().enumerate() {
                    let ours = self.syndromes[c * NPAR + x] ^ row_n[c];
                    *s = f.mul_exp(ours, ORDER - x) ^ f.mul_exp(row_0[c], x * (n - 1));
                }
            }
        }
        out
    }
}

/// The corrections found for our audio.
#[derive(Debug, Default)]
pub struct Repair {
    /// Word (16-bit sample of one channel) from the start of track 1, and
    /// what to XOR it with.
    pub fixes: Vec<(usize, u16)>,
    /// Columns with too many errors to correct.
    pub failed_columns: usize,
}

/// Compares our audio with an entry's parity file (`npar` syndromes per
/// column), the entry being stored `offset` samples away from our audio.
/// `erasures` are doubtful words, from the start of track 1. `None` if the
/// file is not what was expected.
pub fn repair(ours: &Syndromes, parity: &[u8], npar: usize, offset: i32, erasures: &[usize]) -> Option<Repair> {
    if !(2..=NPAR).contains(&npar) || parity.len() != npar * ROW * 2 || offset.unsigned_abs() as usize >= ROW / 4 {
        return None;
    }
    // The file is syndrome-major: all columns of S_0, then of S_1…
    let theirs = |j: usize, x: usize| u16::from_le_bytes([parity[(x * ROW + j) * 2], parity[(x * ROW + j) * 2 + 1]]);
    let shifted = ours.shifted(offset);
    // The erasures of each column, as positions (degrees) in its codeword.
    let mut erased: Vec<Vec<usize>> = vec![Vec::new(); ROW];
    for &word in erasures {
        let theirs = word as isize - 2 * offset as isize;
        let (row, column) = (theirs.div_euclid(ROW as isize), theirs.rem_euclid(ROW as isize) as usize);
        if (1..=ours.rows as isize).contains(&row) {
            erased[column].push(ours.rows - row as usize);
        }
    }
    let mut result = Repair::default();
    let mut syndromes = vec![0u16; npar];
    for j in 0..ROW {
        for (x, s) in syndromes.iter_mut().enumerate() {
            *s = shifted[j * NPAR + x] ^ theirs(j, x);
        }
        if syndromes.iter().all(|&s| s == 0) {
            continue;
        }
        // Erasures first; if C2 missed too many errors, without them.
        let errors = decode(&syndromes, ours.rows, &erased[j]).or_else(|| decode(&syndromes, ours.rows, &[]));
        let Some(errors) = errors else {
            result.failed_columns += 1;
            continue;
        };
        for (degree, value) in errors.into_iter().filter(|&(_, value)| value != 0) {
            let row = ours.rows - degree;
            let word = (row * ROW + j) as isize + 2 * offset as isize;
            match usize::try_from(word) {
                Ok(word) if word < ours.total => result.fixes.push((word, value)),
                _ => result.failed_columns += 1,
            }
        }
    }
    result.fixes.sort_unstable();
    Some(result)
}

/// The errors whose syndromes are `s` (`S_x = Σ e X^x`, `X = α^degree`), in a
/// codeword of `n` positions, some known to be at the `erased` degrees:
/// `(degree, value)` (an erased word may turn out right, of value 0), or
/// `None` if `2 × errors + erasures > s.len()`.
fn decode(s: &[u16], n: usize, erased: &[usize]) -> Option<Vec<(usize, u16)>> {
    let f = field();
    let len = s.len();
    if erased.len() > len {
        return None;
    }
    // Berlekamp-Massey from the erasure locator Γ = Π (1 + α^d x): the
    // locator Λ of errors and erasures.
    let mut lambda = vec![0u16; len + 1];
    lambda[0] = 1;
    for (k, &degree) in erased.iter().enumerate() {
        for i in (1..=k + 1).rev() {
            lambda[i] ^= f.mul_exp(lambda[i - 1], degree);
        }
    }
    let mut previous = lambda.clone();
    let (mut errors, mut gap, mut last_delta) = (erased.len(), 1, 1u16);
    for i in erased.len()..len {
        let mut delta = 0;
        for k in 0..=errors.min(i) {
            delta ^= f.mul(lambda[k], s[i - k]);
        }
        if delta == 0 {
            gap += 1;
            continue;
        }
        let scale = f.mul(delta, f.inv(last_delta));
        let before = lambda.clone();
        for k in 0..=len - gap {
            lambda[k + gap] ^= f.mul(scale, previous[k]);
        }
        if 2 * errors <= i + erased.len() {
            errors = i + 1 + erased.len() - errors;
            previous = before;
            last_delta = delta;
            gap = 1;
        } else {
            gap += 1;
        }
    }
    // `errors` counts the erasures too.
    if errors == 0 || 2 * errors > len + erased.len() || lambda[errors + 1..].iter().any(|&c| c != 0) {
        return None;
    }

    // Chien search: the roots X^-1 of Λ among the code's positions.
    let mut terms: Vec<u16> = lambda[..=errors].to_vec();
    let mut degrees = Vec::new();
    for degree in 0..n {
        if terms.iter().fold(0, |acc, &t| acc ^ t) == 0 {
            degrees.push(degree);
            if degrees.len() == errors {
                break;
            }
        }
        for (k, t) in terms.iter_mut().enumerate() {
            *t = f.mul_exp(*t, ORDER - k);
        }
    }
    if degrees.len() != errors {
        return None;
    }

    // Forney: e = X Ω(X^-1) / Λ'(X^-1), with Ω = S Λ mod x^len.
    let omega: Vec<u16> =
        (0..len).map(|i| (0..=errors.min(i)).fold(0, |acc, k| acc ^ f.mul(s[i - k], lambda[k]))).collect();
    degrees
        .into_iter()
        .map(|degree| {
            let inverse = ORDER - degree;
            let numerator = omega.iter().enumerate().fold(0, |acc, (i, &o)| acc ^ f.mul_exp(o, inverse * i % ORDER));
            let denominator = (1..=errors)
                .step_by(2)
                .fold(0, |acc, k| acc ^ f.mul_exp(lambda[k], inverse * (k - 1) % ORDER));
            (denominator != 0).then(|| (degree, f.mul_exp(f.mul(numerator, f.inv(denominator)), degree)))
        })
        .collect()
}

/// The parity file an entry of `audio` would have, stored `offset` samples
/// away from it.
#[cfg(test)]
pub(crate) fn parity_file(audio: &[u8], offset: i32) -> Vec<u8> {
    let mut shifted = vec![0u8; audio.len()];
    for (i, byte) in shifted.iter_mut().enumerate() {
        let src = i as isize + 4 * offset as isize;
        if (0..audio.len() as isize).contains(&src) {
            *byte = audio[src as usize];
        }
    }
    let mut syndromes = Syndromes::new(audio.len() / 4).unwrap();
    syndromes.add(&shifted);
    let mut file = vec![0u8; NPAR * ROW * 2];
    for j in 0..ROW {
        for x in 0..NPAR {
            file[(x * ROW + j) * 2..][..2].copy_from_slice(&syndromes.syndromes[j * NPAR + x].to_le_bytes());
        }
    }
    file
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(len: usize, seed: u32) -> Vec<u8> {
        let mut seed = seed;
        (0..len)
            .map(|_| {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                (seed >> 16) as u8
            })
            .collect()
    }

    fn rip(audio: &[u8]) -> Syndromes {
        let mut syndromes = Syndromes::new(audio.len() / 4).unwrap();
        // In uneven pieces, like tracks.
        for piece in audio.chunks(ROW * 7 + 6) {
            syndromes.add(piece);
        }
        syndromes
    }

    #[test]
    fn decodes_up_to_half_the_syndromes() {
        let f = field();
        let errors = [(4394, 1063u16), (1188, 43671), (25_748, 44_387), (23_417, 23_213), (1489, 58_675)];
        let mut s = vec![0u16; NPAR];
        for &(degree, value) in &errors {
            for (x, s) in s.iter_mut().enumerate() {
                *s ^= f.mul_exp(value, degree * x);
            }
        }
        let mut found = decode(&s, 30_000, &[]).unwrap();
        found.sort_unstable();
        let mut expected = errors.to_vec();
        expected.sort_unstable();
        assert_eq!(found, expected);
    }

    #[test]
    fn erasures_count_half() {
        let f = field();
        // 12 known positions (one actually right) and 2 unknown: 2 × 2 + 12 = 16.
        let erased: Vec<usize> = (0..12).map(|k| 100 + 37 * k).collect();
        let mut errors: Vec<(usize, u16)> = erased[1..].iter().map(|&d| (d, d as u16 * 7 + 1)).collect();
        errors.extend([(9000, 4242), (17, 999)]);
        let mut s = vec![0u16; NPAR];
        for &(degree, value) in &errors {
            for (x, s) in s.iter_mut().enumerate() {
                *s ^= f.mul_exp(value, degree * x);
            }
        }
        let mut found: Vec<(usize, u16)> =
            decode(&s, 30_000, &erased).unwrap().into_iter().filter(|&(_, v)| v != 0).collect();
        found.sort_unstable();
        errors.sort_unstable();
        assert_eq!(found, errors);
        // Without the erasures, 13 errors are too many.
        assert!(decode(&s, 30_000, &[]).is_none_or(|e| e.len() <= 8 && e != errors));
    }

    #[test]
    fn repairs_damaged_audio_at_any_offset() {
        let audio = noise(40 * ROW * 2, 3);
        for offset in [0, 6, -6, 667, -1995] {
            let parity = parity_file(&audio, offset);
            let mut damaged = audio.clone();
            // A scratch: a run of words in a few rows, and a lone sample.
            for word in (5 * ROW + 100..5 * ROW + 108).chain([20 * ROW + 3, 33 * ROW + ROW - 1]) {
                damaged[2 * word] ^= 0x5A;
            }
            let repair = repair(&rip(&damaged), &parity, NPAR, offset, &[]).unwrap();
            assert_eq!(repair.failed_columns, 0);
            for (word, value) in repair.fixes {
                damaged[2 * word] ^= value as u8;
                damaged[2 * word + 1] ^= (value >> 8) as u8;
            }
            assert!(damaged == audio, "offset {offset}");
        }
    }

    #[test]
    fn reports_columns_beyond_repair() {
        let audio = noise(40 * ROW * 2, 5);
        let parity = parity_file(&audio, 0);
        let mut damaged = audio.clone();
        // The same column wrong in 9 rows: one more than 16 syndromes fix.
        for row in 2..11 {
            damaged[2 * (row * ROW + 50)] ^= 1;
        }
        let repair = repair(&rip(&damaged), &parity, NPAR, 0, &[]).unwrap();
        assert_eq!(repair.failed_columns, 1);
        assert!(repair.fixes.is_empty());

        // Known as doubtful (C2), the same 9 words and 3 more are fixed.
        let erasures: Vec<usize> = (2..14).map(|row| row * ROW + 50).collect();
        let with_erasures = super::repair(&rip(&damaged), &parity, NPAR, 0, &erasures).unwrap();
        assert_eq!((with_erasures.failed_columns, with_erasures.fixes.len()), (0, 9));
    }
}
