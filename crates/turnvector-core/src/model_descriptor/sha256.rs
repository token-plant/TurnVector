// SPDX-License-Identifier: Apache-2.0
// SHA-256 derivative of sha2-fv v0.1.0, remix7531/hashes-fv commit 5b33fdba3e77dceca487e35bc65610758d00b40c.
// Archive SHA-256 bf37a87bd5984f268882c2652c2c63d120d91aaf30ee27dfac9b473959e2d9fc. Selected upstream license: Apache-2.0.
// src/lib.rs -> checked bounded one-shot/padding with exact compression-block Work; src/consts.rs -> H256_256/K32 only.
// src/sha256.rs -> inline byte decoding; src/sha256/soft/compact.rs -> inline single-block compression.
// Removed: other digest variants, generic algorithms, streaming, allocation, hardware dispatch, features, unrelated build integration, and public/hazmat APIs.

use crate::{WorkBudgetError, WorkDimension, WorkMeter};
pub(super) const MAX_INPUT_BYTES: usize = 16_425;
const INITIAL_STATE: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];
#[rustfmt::skip]
const ROUND_CONSTANTS: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Sha256Error {
    InputTooLong,
    Work(WorkBudgetError),
}
#[allow(dead_code)]
pub(super) fn digest(input: &[u8], work: &mut WorkMeter) -> Result<[u8; 32], Sha256Error> {
    if input.len() > MAX_INPUT_BYTES {
        return Err(Sha256Error::InputTooLong);
    }
    let full_blocks = input.len() / 64;
    let remaining = input.len() % 64;
    let compression_blocks = full_blocks + 1 + usize::from(remaining >= 56);
    work.record(WorkDimension::VisitedEntities, compression_blocks as u64)
        .map_err(Sha256Error::Work)?;
    let mut state = INITIAL_STATE;
    for chunk in input[..full_blocks * 64].chunks_exact(64) {
        compress(&mut state, chunk.try_into().expect("64-byte chunk"));
    }
    let bit_len = u64::try_from(input.len())
        .ok()
        .and_then(|length| length.checked_mul(8))
        .ok_or(Sha256Error::InputTooLong)?;
    let mut final_block = [0_u8; 64];
    final_block[..remaining].copy_from_slice(&input[full_blocks * 64..]);
    final_block[remaining] = 0x80;
    if remaining >= 56 {
        compress(&mut state, &final_block);
        final_block = [0; 64];
    }
    final_block[56..].copy_from_slice(&bit_len.to_be_bytes());
    compress(&mut state, &final_block);
    let mut output = [0_u8; 32];
    for (bytes, word) in output.chunks_exact_mut(4).zip(state) {
        bytes.copy_from_slice(&word.to_be_bytes());
    }
    Ok(output)
}

fn compress(state: &mut [u32; 8], block: &[u8; 64]) {
    let mut schedule = [0_u32; 16];
    for (index, word) in schedule.iter_mut().enumerate() {
        *word = u32::from_be_bytes(block[index * 4..index * 4 + 4].try_into().expect("word"));
    }
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for (index, constant) in ROUND_CONSTANTS.into_iter().enumerate() {
        let word = if index < 16 {
            schedule[index]
        } else {
            let x = schedule[(index - 15) % 16];
            let y = schedule[(index - 2) % 16];
            let next = schedule[(index - 16) % 16]
                .wrapping_add(x.rotate_right(7) ^ x.rotate_right(18) ^ (x >> 3))
                .wrapping_add(schedule[(index - 7) % 16])
                .wrapping_add(y.rotate_right(17) ^ y.rotate_right(19) ^ (y >> 10));
            schedule[index % 16] = next;
            next
        };
        let sum1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let choice = (e & f) ^ ((!e) & g);
        let temp1 = h
            .wrapping_add(sum1)
            .wrapping_add(choice)
            .wrapping_add(constant)
            .wrapping_add(word);
        let sum0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let temp2 = sum0.wrapping_add((a & b) ^ (a & c) ^ (b & c));
        (h, g, f, e, d, c, b, a) = (
            g,
            f,
            e,
            d.wrapping_add(temp1),
            c,
            b,
            a,
            temp1.wrapping_add(temp2),
        );
    }
    for (slot, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
        *slot = slot.wrapping_add(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{HotPathWorkBudget, HotPathWorkWitness, WorkMeter};
    #[rustfmt::skip]
    const NIST_CASES: &[(&[u8], u64, &str)] = &[
        (b"", 1, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
        (b"abc", 1, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),
        (b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq", 2, "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"),
    ];
    #[rustfmt::skip]
    const UPSTREAM_CASES: &[(usize, u64, &str)] = &[
        (1, 1, "e7cf46a078fed4fafd0b5e3aff144802b853f8ae459a4f0c14add3314b7cc3a6"),
        (55, 1, "2900465fcb533e05a158fd2b3be0e5e3b03740d83060aa3580e0d98a96bf2384"),
        (56, 2, "31454ff48ef36af2f08fd511bdc37d9d5855ac23e992e5ff5445cb6b7674a674"),
        (63, 2, "5f6401b96532c36de4e65beec0409b69b1d181864c8009b7a04f43e5d56350d1"),
        (64, 2, "94eb5de4943613fd048dc93393ab06877405faa39c11f53e9386083339833e7e"),
        (65, 2, "fc518669b6eb4b4dd91827ecacef86689c725bd5bab888fd3b26dbb196eec954"),
        (119, 2, "b0dc41b1a384e2f1203f0351b38fbeaafceef577ce1191d5bfc25da39f721eae"),
        (120, 3, "5df24dd802ac26132ce608dcb5f09841eef039ee0f152acf98d26d17fe4e88e6"),
        (127, 3, "0fe729ff19257bd6fec853acc2ea355f6b34b58e6c0f684c3e188fcdfcd9baae"),
        (128, 3, "0aedd4856f8eba0963627336ad5144a9a7dbe12498e6066f0165fc97d8ddee4c"),
        (129, 3, "4f1757ae4bffbae86d775b831765b75af154d52f7deaa46dd378051a2d3ad57f"),
        (16_424, 257, "91c72f14d1601b0a7951550bbf6f706a8a0d5f0459e481743541b57272b67f39"),
        (16_425, 257, "3203090ab46cda163935d8130430826e21fa864f2fc3d74ceaa90cc0f11bf78c"),
    ];
    fn hex(value: &str) -> [u8; 32] {
        fn digit(value: u8) -> u8 {
            match value {
                b'0'..=b'9' => value - b'0',
                b'a'..=b'f' => value - b'a' + 10,
                _ => panic!("invalid fixture hex"),
            }
        }
        assert_eq!(value.len(), 64);
        let mut output = [0; 32];
        for (slot, pair) in output.iter_mut().zip(value.as_bytes().chunks_exact(2)) {
            *slot = digit(pair[0]) << 4 | digit(pair[1]);
        }
        output
    }
    fn assert_case(input: &[u8], blocks: u64, expected: &str) {
        let mut work = WorkMeter::new(HotPathWorkBudget::binary_maximum());
        assert_eq!(digest(input, &mut work).unwrap(), hex(expected));
        assert_eq!(
            work.witness(),
            HotPathWorkWitness::new([blocks, 0, 0, 0, 0])
        );
    }
    #[test]
    fn matches_nist_known_answers() {
        for &(input, blocks, expected) in NIST_CASES {
            assert_case(input, blocks, expected);
        }
    }
    #[test]
    fn matches_frozen_upstream_at_padding_and_maximum_boundaries() {
        let mut input = [0_u8; MAX_INPUT_BYTES];
        for (index, byte) in input.iter_mut().enumerate() {
            *byte = (index as u8).wrapping_mul(37).wrapping_add(11);
        }
        for &(length, blocks, expected) in UPSTREAM_CASES {
            assert_case(&input[..length], blocks, expected);
        }
    }
    #[test]
    fn rejections_preserve_the_complete_work_witness() {
        let input = [0_u8; MAX_INPUT_BYTES + 1];
        let mut work = WorkMeter::new(HotPathWorkBudget::binary_maximum());
        assert_eq!(digest(&input, &mut work), Err(Sha256Error::InputTooLong));
        assert_eq!(work.witness(), HotPathWorkWitness::default());

        let input = [0_u8; MAX_INPUT_BYTES];
        let mut work = WorkMeter::new(HotPathWorkBudget::binary_maximum());
        work.record(WorkDimension::VisitedEntities, 999_744)
            .unwrap();
        let before = work.witness();
        let expected = Sha256Error::Work(WorkBudgetError::BudgetExceeded(
            WorkDimension::VisitedEntities,
            1_000_000,
            1_000_001,
        ));
        assert_eq!(digest(&input, &mut work), Err(expected));
        assert_eq!(work.witness(), before);
    }
}
