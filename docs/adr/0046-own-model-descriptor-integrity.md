# Own Model Descriptor Integrity in One Private Module

TurnVector owns Model Descriptor integrity in one private deep `turnvector-core::model_descriptor` module. It is not a crate, public trait, replaceable dependency, or Backend capability. Its private `sha256.rs` submodule is consumed only by Model Descriptor integrity and exposes no generic crypto, streaming, or incremental-hash seam. The module accepts bounded raw Backend descriptor claims and a Manifest expectation, then returns only a field-private, non-forgeable `VerifiedModelDescriptor` containing the exact frame, independently derived typed identity and hash, and nonzero vocabulary. Registration, Bootstrap, and post-load validation all use this one verifier and compare exact sealed values.

The canonical V1 frame is exactly:

```text
u32be(1) || u32be(nonzero vocabulary) || u32be(nonzero payload_len) || payload
```

The complete frame is at most 16,384 bytes, so payload is at most 16,372 bytes. The parser rejects unknown version, zero vocabulary, zero payload length, disagreement between declared and actual length, oversize input, padding, and trailing bytes. The exact V1 payload carries build-bound descriptor semantics, including capabilities and conservative residency resource/time bounds. This integrity module treats it only as opaque exact bytes: it does not normalize or interpret those semantics, grant authority from them, or collapse distinct byte strings. Their owning later modules validate and use the sealed payload. The registry admits at most 256 Revisions and owns an independent fixed 4,194,304-byte descriptor arena, exactly `256 * 16,384`; it cannot borrow from another arena or capacity pool.

Identity and evidence hash are independent derivations over the complete frame:

```text
ModelDescriptorId =
  SHA256("turnvector:identity:model-descriptor" || 0x00 || u32be(1) || frame)

ModelDescriptorHash = {
  schema_version = 1,
  digest = SHA256(
    "turnvector:evidence:model-descriptor" || 0x00 || u32be(1) || frame
  )
}
```

Equal digest bytes do not erase the type or schema distinction. A raw Backend ID, hash, vocabulary, or frame is an untrusted claim. The verifier parses the frame, derives both values and vocabulary, compares every raw claim plus the Manifest's expected typed hash, and only then seals the value. It performs all rejection before durable copy or mutation. Persistence may store the complete frame and sealed fields for exact readback, but no stored field, SQL constraint, Backend result, or Manifest digest can bypass rederivation.

The SHA-256 implementation is a repository-owned, SHA-256-only portable derivative extracted from `sha2-fv` v0.1.0 in `remix7531/hashes-fv` at commit `5b33fdba3e77dceca487e35bc65610758d00b40c`; the source archive SHA-256 is `bf37a87bd5984f268882c2652c2c63d120d91aaf30ee27dfac9b473959e2d9fc`. TurnVector selects the upstream Apache-2.0 option and retains that license, the exact upstream revision and archive digest, the copied-source inventory, and a path-by-path modification record. The GPL proof tree is neither copied nor linked. The derivative does not inherit or claim upstream formal verification. NIST/FIPS known-answer vectors and differential results against the exact upstream revision are tests, not CAVP validation, FIPS certification, or formal proof.

There is no crates.io or path dependency and no Cargo or lockfile change. The implementation is safe Rust with no `unsafe`, allocation, hardware dispatch, generic digest abstraction, streaming wrapper, or runtime-selected algorithm. It is one exact bounded one-shot SHA-256 path. Each identity preimage is at most 16,425 bytes, below `2^61`, so bit-length encoding is checked without widening the domain; a maximum preimage consumes exactly 257 compression blocks, and both derivations consume at most 514. Hot-Path Work meters both hashes by their exact actual compression-block counts and also meters bounded parsing, comparison, and copy. Success and rejection fixtures cover frame boundaries, every malformed branch, independent domain separation, exact Work, known-answer vectors, upstream differential evidence, zero allocations, and no mutation before verification.

## Consequences

- Model Descriptor codec and integrity details remain local to one deep private module; `model_registry`, Core coordination, Backend adapters, storage, and protocol consume sealed domain values rather than reimplementing parsing or hashing.
- Changing the frame, either domain, schema version, size bound, arena bound, or SHA source is a determining runtime-source input and changes the daemon Core build identity.
- C10 delivery is ordered as fixed SHA-256, canonical verification, sealed registry state, then atomic Core coordination. No intermediate row grants runtime registration behavior or exposes a partial validation bypass.
