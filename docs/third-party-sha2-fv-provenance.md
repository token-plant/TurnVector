# sha2-fv SHA-256 Derivative Provenance

TurnVector's planned private Model Descriptor SHA-256 implementation is a repository-owned derivative of `sha2-fv` v0.1.0 from `remix7531/hashes-fv` at commit `5b33fdba3e77dceca487e35bc65610758d00b40c`.

| Field | Frozen value |
|---|---|
| Upstream source | `https://github.com/remix7531/hashes-fv` |
| Upstream authors | RustCrypto Developers; `remix7531 <remix7531@mailbox.org>` |
| Version | `v0.1.0` |
| Commit | `5b33fdba3e77dceca487e35bc65610758d00b40c` |
| Crate archive | `https://static.crates.io/crates/sha2-fv/sha2-fv-0.1.0.crate` |
| Source archive SHA-256 | `bf37a87bd5984f268882c2652c2c63d120d91aaf30ee27dfac9b473959e2d9fc` |
| Upstream Rust-source license | MIT OR Apache-2.0 |
| TurnVector selection | Apache-2.0 |
| TurnVector consumer | private `turnvector-core::model_descriptor::sha256` only |

The implementation row must retain the upstream Apache-2.0 notice, an exact inventory of copied source paths, and a path-by-path record of every TurnVector modification. The permitted extraction is SHA-256-only portable source. It removes or excludes other digest variants, generic algorithm and streaming surfaces, allocation, hardware dispatch, and unrelated build integration; adapts the source to one bounded one-shot private API; and adds exact compression-block Work accounting. It introduces no `unsafe`, crates.io or path dependency, Cargo or lockfile change, or public crypto seam.

This Markdown notice is governance evidence, not a runtime identity input. The copied Rust source retains an in-source provenance header with the same upstream commit, archive digest, and selected license; those source bytes enter the normal B03 source closure, while tests check that the header and this notice agree.

The upstream GPL proof tree is neither copied nor linked. TurnVector makes no claim that the derivative inherits formal verification. NIST/FIPS SHA-256 known-answer tests and differential tests against the exact upstream revision are test evidence only; they are not CAVP validation, FIPS certification, or formal verification.
