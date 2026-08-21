#![allow(dead_code, reason = "C10d and C10e consume this private verifier")]

mod sha256;

use crate::WorkDimension::{CandidateWork, CopiedBytes, InvariantChecks, VisitedEntities};
use crate::{HotPathWorkWitness, WorkBudgetError, WorkMeter};

pub(super) const MAX_FRAME_BYTES: usize = 16_384;
pub(super) const MAX_PAYLOAD_BYTES: usize = MAX_FRAME_BYTES - 12;
const FRAME_VERSION: u32 = 1;
const HEADER_BYTES: usize = 12;
const DOMAIN_BYTES: usize = 36;
const HASH_PREFIX_BYTES: usize = DOMAIN_BYTES + 1 + std::mem::size_of::<u32>();
const ID_DOMAIN: &[u8; DOMAIN_BYTES] = b"turnvector:identity:model-descriptor";
const HASH_DOMAIN: &[u8; DOMAIN_BYTES] = b"turnvector:evidence:model-descriptor";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ModelDescriptorId([u8; 32]);
impl ModelDescriptorId {
    pub(super) const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ModelDescriptorHash {
    schema_version: u32,
    digest: [u8; 32],
}
impl ModelDescriptorHash {
    pub(super) fn from_manifest(
        schema_version: u32,
        digest: [u8; 32],
    ) -> Result<Self, ModelDescriptorError> {
        if schema_version != FRAME_VERSION {
            return Err(ModelDescriptorError::HashSchema);
        }
        Ok(Self {
            schema_version,
            digest,
        })
    }
    pub(super) const fn schema_version(self) -> u32 {
        self.schema_version
    }
    pub(super) const fn digest(self) -> [u8; 32] {
        self.digest
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct RawModelDescriptor<'a> {
    pub(super) frame: &'a [u8],
    pub(super) id: [u8; 32],
    pub(super) hash_schema_version: u32,
    pub(super) hash: [u8; 32],
    pub(super) vocabulary: u32,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct VerifiedModelDescriptor {
    frame: [u8; MAX_FRAME_BYTES],
    frame_len: u16,
    id: ModelDescriptorId,
    hash: ModelDescriptorHash,
    vocabulary: u32,
}
impl VerifiedModelDescriptor {
    pub(super) fn frame(&self) -> &[u8] {
        &self.frame[..usize::from(self.frame_len)]
    }
    pub(super) const fn id(&self) -> ModelDescriptorId {
        self.id
    }
    pub(super) const fn hash(&self) -> ModelDescriptorHash {
        self.hash
    }
    pub(super) const fn vocabulary(&self) -> u32 {
        self.vocabulary
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ModelDescriptorError {
    FrameTooLong,
    Version,
    Vocabulary,
    PayloadLength,
    VocabularyClaim,
    IdClaim,
    HashClaim,
    ManifestHash,
    HashSchema,
    Work(WorkBudgetError),
}
impl From<sha256::Sha256Error> for ModelDescriptorError {
    fn from(error: sha256::Sha256Error) -> Self {
        match error {
            sha256::Sha256Error::InputTooLong => Self::FrameTooLong,
            sha256::Sha256Error::Work(error) => Self::Work(error),
        }
    }
}

pub(super) fn verify(
    raw: RawModelDescriptor<'_>,
    expected: ModelDescriptorHash,
    work: &mut WorkMeter,
) -> Result<VerifiedModelDescriptor, ModelDescriptorError> {
    let vocabulary = parse(raw.frame, work)?;
    let preimage_bytes = HASH_PREFIX_BYTES + raw.frame.len();
    let blocks = compression_blocks(preimage_bytes) as u64;
    preflight(work, [2 * blocks, (2 * preimage_bytes) as u64, 0, 2, 4])?;
    charge(work, [0, (2 * preimage_bytes) as u64, 0, 2, 4])?;

    let id = ModelDescriptorId(digest(ID_DOMAIN, raw.frame, work)?);
    let hash = ModelDescriptorHash {
        schema_version: FRAME_VERSION,
        digest: digest(HASH_DOMAIN, raw.frame, work)?,
    };
    let vocabulary_matches = raw.vocabulary == vocabulary;
    let id_matches = raw.id == id.0;
    let hash_matches = raw.hash_schema_version == hash.schema_version && raw.hash == hash.digest;
    let manifest_matches = expected == hash;
    if !vocabulary_matches {
        return Err(ModelDescriptorError::VocabularyClaim);
    }
    if !id_matches {
        return Err(ModelDescriptorError::IdClaim);
    }
    if !hash_matches {
        return Err(ModelDescriptorError::HashClaim);
    }
    if !manifest_matches {
        return Err(ModelDescriptorError::ManifestHash);
    }

    charge(work, [0, raw.frame.len() as u64, 0, 0, 0])?;
    let mut frame = [0; MAX_FRAME_BYTES];
    frame[..raw.frame.len()].copy_from_slice(raw.frame);
    Ok(VerifiedModelDescriptor {
        frame,
        frame_len: u16::try_from(raw.frame.len()).expect("frame is bounded"),
        id,
        hash,
        vocabulary,
    })
}

fn parse(frame: &[u8], work: &mut WorkMeter) -> Result<u32, ModelDescriptorError> {
    let checks = if frame.len() > MAX_FRAME_BYTES {
        1
    } else if frame.len() < HEADER_BYTES {
        2
    } else {
        6
    };
    charge(work, [1, 0, 0, 0, checks])?;
    if frame.len() > MAX_FRAME_BYTES {
        return Err(ModelDescriptorError::FrameTooLong);
    }
    if frame.len() < HEADER_BYTES {
        return Err(ModelDescriptorError::PayloadLength);
    }
    let header: &[u8; HEADER_BYTES] = frame[..HEADER_BYTES].try_into().expect("fixed header");
    let version = u32::from_be_bytes(header[..4].try_into().expect("version bytes"));
    let vocabulary = u32::from_be_bytes(header[4..8].try_into().expect("vocabulary bytes"));
    let payload_len = u32::from_be_bytes(header[8..].try_into().expect("payload bytes"));
    if version != FRAME_VERSION {
        return Err(ModelDescriptorError::Version);
    }
    if vocabulary == 0 {
        return Err(ModelDescriptorError::Vocabulary);
    }
    if payload_len == 0 {
        return Err(ModelDescriptorError::PayloadLength);
    }
    if usize::try_from(payload_len).ok() != Some(frame.len() - HEADER_BYTES) {
        return Err(ModelDescriptorError::PayloadLength);
    }
    Ok(vocabulary)
}

fn digest(
    domain: &[u8; DOMAIN_BYTES],
    frame: &[u8],
    work: &mut WorkMeter,
) -> Result<[u8; 32], ModelDescriptorError> {
    let mut input = [0; sha256::MAX_INPUT_BYTES];
    input[..DOMAIN_BYTES].copy_from_slice(domain);
    input[DOMAIN_BYTES] = 0;
    input[DOMAIN_BYTES + 1..HASH_PREFIX_BYTES].copy_from_slice(&FRAME_VERSION.to_be_bytes());
    let length = HASH_PREFIX_BYTES + frame.len();
    input[HASH_PREFIX_BYTES..length].copy_from_slice(frame);
    Ok(sha256::digest(&input[..length], work)?)
}

fn charge(work: &mut WorkMeter, values: [u64; 5]) -> Result<(), ModelDescriptorError> {
    preflight(work, values)?;
    for (dimension, amount) in [
        (VisitedEntities, values[0]),
        (CopiedBytes, values[1]),
        (CandidateWork, values[3]),
        (InvariantChecks, values[4]),
    ] {
        work.record(dimension, amount)
            .map_err(ModelDescriptorError::Work)?;
    }
    Ok(())
}

fn preflight(work: &WorkMeter, values: [u64; 5]) -> Result<(), ModelDescriptorError> {
    work.ensure(HotPathWorkWitness::new(values))
        .map_err(ModelDescriptorError::Work)
}

fn compression_blocks(length: usize) -> usize {
    length / 64 + 1 + usize::from(length % 64 >= 56)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{HotPathWorkBudget, HotPathWorkWitness, WorkBudgetError, WorkMeter};
    use ModelDescriptorError::*;
    const FRAME: [u8; 13] = [0, 0, 0, 1, 0, 0, 0, 7, 0, 0, 0, 1, b'x'];
    #[rustfmt::skip]
    const ID: [u8; 32] = [0xc9, 0x1c, 0x14, 0x09, 0x1c, 0xea, 0x08, 0xf4, 0x58, 0xa4, 0xe2, 0x75, 0x96, 0xc1, 0x5b, 0x2c, 0xf0, 0xc8, 0x74, 0x34, 0x2d, 0x30, 0x3e, 0xad, 0xe8, 0x9f, 0x29, 0x0e, 0xd0, 0x13, 0x38, 0x21];
    #[rustfmt::skip]
    const HASH: [u8; 32] = [0xe2, 0x24, 0x6d, 0x47, 0x7f, 0x70, 0xd3, 0xe6, 0x58, 0x8b, 0xb5, 0x45, 0xe2, 0x14, 0xc0, 0xbb, 0xa1, 0x76, 0x6e, 0xf3, 0x39, 0x7a, 0x50, 0x71, 0x89, 0x29, 0xc9, 0x4f, 0xe9, 0x62, 0x1e, 0x9b];
    #[rustfmt::skip]
    fn expected(hash: [u8; 32]) -> ModelDescriptorHash { ModelDescriptorHash::from_manifest(1, hash).unwrap() }
    #[rustfmt::skip]
    fn raw(frame: &[u8], vocabulary: u32, id: [u8; 32], hash: [u8; 32]) -> RawModelDescriptor<'_> { RawModelDescriptor { frame, id, hash_schema_version: 1, hash, vocabulary } }
    #[rustfmt::skip]
    fn meter() -> WorkMeter { WorkMeter::new(HotPathWorkBudget::binary_maximum()) }
    #[rustfmt::skip]
    fn rejected(raw: RawModelDescriptor<'_>, expected: ModelDescriptorHash, error: ModelDescriptorError, witness: [u64; 5]) { let mut work = meter(); assert_eq!(verify(raw, expected, &mut work), Err(error)); assert_eq!(work.witness(), HotPathWorkWitness::new(witness)); }
    #[test]
    #[rustfmt::skip]
    fn validates_canonical_frames_claims_and_work() {
        let valid = raw(&FRAME, 7, ID, HASH); let mut work = meter(); let sealed = verify(valid, expected(HASH), &mut work).unwrap();
        assert_eq!((sealed.frame(), sealed.id().bytes(), sealed.hash().digest(), sealed.vocabulary()), (&FRAME[..], ID, HASH, 7)); assert_eq!(sealed.hash().schema_version(), 1); assert_ne!(ID, HASH); assert_eq!(work.witness(), HotPathWorkWitness::new([3, 121, 0, 2, 10]));
        let mut repeat = meter(); assert_eq!(sealed, verify(valid, expected(HASH), &mut repeat).unwrap());
        for index in 0..FRAME.len() { let mut drift = FRAME; drift[index] ^= 1; let mut drift_work = meter(); let result = verify(raw(&drift, 7, ID, HASH), expected(HASH), &mut drift_work); if index == 12 { assert_eq!(result, Err(IdClaim)); } else { assert!(result.is_err()); } assert_eq!(drift_work.witness(), HotPathWorkWitness::new(if (4..=7).contains(&index) || index == 12 { [3, 108, 0, 2, 10] } else { [1, 0, 0, 0, 6] }), "index {index}"); }
        for (claim, manifest, error) in [(RawModelDescriptor { vocabulary: 8, ..valid }, expected(HASH), VocabularyClaim), (RawModelDescriptor { id: [0; 32], ..valid }, expected(HASH), IdClaim), (RawModelDescriptor { hash_schema_version: 2, ..valid }, expected(HASH), HashClaim), (RawModelDescriptor { hash: [0; 32], ..valid }, expected(HASH), HashClaim), (valid, expected([0; 32]), ManifestHash)] { rejected(claim, manifest, error, [3, 108, 0, 2, 10]); }
        let mut version = FRAME; version[3] = 2; let mut vocabulary = FRAME; vocabulary[7] = 0; let mut empty = FRAME; empty[11] = 0; let mut trailing = FRAME; trailing[11] = 2; let oversize = [0; MAX_FRAME_BYTES + 1];
        for (frame, error, witness) in [(&oversize[..], FrameTooLong, [1, 0, 0, 0, 1]), (&FRAME[..11], PayloadLength, [1, 0, 0, 0, 2]), (&version[..], Version, [1, 0, 0, 0, 6]), (&vocabulary[..], Vocabulary, [1, 0, 0, 0, 6]), (&empty[..], PayloadLength, [1, 0, 0, 0, 6]), (&trailing[..], PayloadLength, [1, 0, 0, 0, 6])] { rejected(raw(frame, 7, ID, HASH), expected(HASH), error, witness); }
        assert_eq!(ModelDescriptorHash::from_manifest(2, HASH), Err(HashSchema));
        let mut full = meter(); full.record(VisitedEntities, 1_704_575).unwrap(); let before = full.witness(); assert_eq!(verify(valid, expected(HASH), &mut full), Err(Work(WorkBudgetError::BudgetExceeded(VisitedEntities, 1_704_575, 1_704_576)))); assert_eq!(full.witness(), before);
        let budget = HotPathWorkBudget::try_new(HotPathWorkWitness::new([1_000_000, 108, 0, 2, 2_100])).unwrap(); let mut late = WorkMeter::new(budget); assert_eq!(verify(valid, expected(HASH), &mut late), Err(Work(WorkBudgetError::BudgetExceeded(CopiedBytes, 108, 121)))); assert_eq!(late.witness(), HotPathWorkWitness::new([3, 108, 0, 2, 10]));
    }
    #[test]
    #[rustfmt::skip]
    fn accepts_the_maximum_frame() {
        let mut frame = [0; MAX_FRAME_BYTES]; frame[..12].copy_from_slice(&[0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 63, 244]); let mut claims = meter(); let id = digest(ID_DOMAIN, &frame, &mut claims).unwrap(); let hash = digest(HASH_DOMAIN, &frame, &mut claims).unwrap(); let mut work = meter(); let sealed = verify(raw(&frame, 1, id, hash), expected(hash), &mut work).unwrap(); assert_eq!(sealed.frame(), &frame); assert_eq!(work.witness(), HotPathWorkWitness::new([515, 49_234, 0, 2, 10]));
    }
}
