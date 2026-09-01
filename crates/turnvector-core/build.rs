#![forbid(unsafe_code)]

#[path = "src/c17_layout.rs"]
mod c17_layout;

use c17_layout::*;
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::process::Command;

const R_PHYSICAL: u64 = 16_530;
const F_PHYSICAL: u64 = 2_057;
const Q_HISTORY: u64 = 7_312;
const E_HISTORY: u64 = 1_152;
const C_CELLS: u64 = 6_912;
const V_AXES: u64 = 6;
const HORIZONS: [u64; 3] = [1_000_000, 10_000_000, 20_000_000];
const EXPECTED_CEILING: u64 = 63_942_176;
const S3_SOURCES: &[&str] = &[
    "Cargo.toml",
    "src/admission.rs",
    "src/bounded.rs",
    "src/c17_layout.rs",
    "src/certification.rs",
    "src/closure_control.rs",
    "src/core.rs",
    "src/lib.rs",
    "src/model_descriptor.rs",
    "src/model_descriptor/sha256.rs",
    "src/model_registry.rs",
    "src/request_book.rs",
    "src/request_book/c17.rs",
    "src/resource_ledger.rs",
    "src/reusable.rs",
    "src/scheduler.rs",
    "src/scheduling.rs",
    "src/support.rs",
    "src/support/c17.rs",
    "src/support/c17/membership.rs",
    "src/support/c17/semantic.rs",
    "src/support/c17/topology.rs",
    "src/transition_coordinator.rs",
    "src/turn_plans.rs",
    "src/turns.rs",
    "src/work.rs",
];
const SHA_INITIAL: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];
#[rustfmt::skip]
const SHA_ROUNDS: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

fn main() {
    for path in S3_SOURCES.iter().copied().chain([
        "build.rs",
        "src/c17_generated.rs",
        "../../scripts/generate_daemon_core_build.py",
    ]) {
        println!("cargo:rerun-if-changed={path}");
    }
    println!("cargo:rustc-check-cfg=cfg(turnvector_c17_generated)");
    println!("cargo:rustc-check-cfg=cfg(turnvector_c17_probe)");
    let pointer_width = env::var("CARGO_CFG_TARGET_POINTER_WIDTH")
        .expect("Cargo supplies the target pointer width");
    assert_eq!(pointer_width, "64", "C17 requires a 64-bit target");
    assert_eq!(
        size_of::<Box<[u8]>>(),
        16,
        "C17 freezes Box slice descriptors"
    );

    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest dir"));
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output dir"));
    let probe = compile_defining_probe(&manifest, &output);
    let first_b03 = run_defining_probe(&probe);
    let second_b03 = run_defining_probe(&probe);
    assert_eq!(
        first_b03, second_b03,
        "B03 defining-module probe output changed between runs"
    );
    let values = parse_probe(&first_b03);
    let ceiling = support_ceiling(&values).expect("C17 support ceiling arithmetic overflowed");
    assert_eq!(ceiling, EXPECTED_CEILING, "generated C17 ceiling changed");
    verify_embedded_ceiling(ceiling);
    let first_b04 = b04(&first_b03, &values, ceiling);
    let second_b04 = b04(&second_b03, &parse_probe(&second_b03), ceiling);
    assert_eq!(first_b04, second_b04, "B04 output changed between runs");

    let generator = manifest.join("../../scripts/generate_daemon_core_build.py");
    let s3 = source_digest(&manifest, S3_SOURCES);
    let d3 = chained_digest(b"turnvector.c17.s3-d3\0", &[&s3, &first_b03]);
    let s4 = sha256(include_bytes!("build.rs"));
    let d4 = chained_digest(b"turnvector.c17.s4-d3-d4\0", &[&s4, &d3, &first_b04]);
    let s5 = sha256(&fs::read(generator).expect("read B05 generator source"));

    let b05_b03 = run_defining_probe(&probe);
    assert_eq!(first_b03, b05_b03, "B05 independent D3 rerun changed");
    let b05_values = parse_probe(&b05_b03);
    let b05_ceiling = support_ceiling(&b05_values).expect("B05 ceiling arithmetic");
    let b05_b04 = b04(&b05_b03, &b05_values, b05_ceiling);
    assert_eq!(first_b04, b05_b04, "B05 independent D4 rerun changed");
    let b05 = b05(s3, d3, s4, d4, s5);
    let d5 = chained_digest(b"turnvector.c17.s5-d3-d4-d5\0", &[&s5, &d3, &d4, &b05]);

    write(&output, "c17-b03.txt", &first_b03);
    write(&output, "c17-b04.txt", &first_b04);
    write(&output, "c17-b05.txt", &b05);
    write(
        &output,
        "c17-d3.sha256",
        format!("{}\n", hex(d3)).as_bytes(),
    );
    write(
        &output,
        "c17-d4.sha256",
        format!("{}\n", hex(d4)).as_bytes(),
    );
    write(
        &output,
        "c17-d5.sha256",
        format!("{}\n", hex(d5)).as_bytes(),
    );
    println!("cargo:rustc-cfg=turnvector_c17_generated");
}

fn compile_defining_probe(manifest: &Path, output: &Path) -> PathBuf {
    let executable = output.join(format!(
        "turnvector-c17-layout-probe{}",
        env::consts::EXE_SUFFIX
    ));
    let rustc = env::var_os("RUSTC").expect("Cargo supplies rustc");
    let binary = Command::new(rustc)
        .current_dir(manifest)
        .arg("--crate-name")
        .arg("turnvector_c17_layout_probe")
        .arg("--crate-type=bin")
        .arg("--edition=2024")
        .arg("--cfg")
        .arg("turnvector_c17_generated")
        .arg("--cfg")
        .arg("turnvector_c17_probe")
        .arg("--check-cfg")
        .arg("cfg(turnvector_c17_generated)")
        .arg("--check-cfg")
        .arg("cfg(turnvector_c17_probe)")
        .arg("-Adead_code")
        .arg("src/lib.rs")
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("invoke defining-module rustc probe");
    assert!(
        binary.status.success(),
        "compile defining-module C17 executable probe failed:\n{}",
        String::from_utf8_lossy(&binary.stderr)
    );
    executable
}

fn run_defining_probe(executable: &Path) -> Vec<u8> {
    let result = Command::new(executable)
        .output()
        .expect("execute defining-module C17 probe");
    assert!(
        result.status.success(),
        "defining-module C17 probe failed:\n{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        result.stderr.is_empty(),
        "defining-module C17 probe wrote stderr"
    );
    result.stdout
}

fn parse_probe(payload: &[u8]) -> BTreeMap<String, u64> {
    let text = std::str::from_utf8(payload).expect("B03 probe is UTF-8");
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("turnvector.c17.b03.v1"));
    let mut values = BTreeMap::new();
    for line in lines {
        let (name, value) = line.split_once('=').expect("B03 name=value row");
        let Ok(value) = value.parse::<u64>() else {
            continue;
        };
        assert!(
            values.insert(name.to_owned(), value).is_none(),
            "duplicate scalar B03 row {name}"
        );
    }
    values
}

fn probe_value(values: &BTreeMap<String, u64>, name: &str) -> u64 {
    *values
        .get(name)
        .unwrap_or_else(|| panic!("missing B03 row {name}"))
}

fn source_digest(manifest: &Path, paths: &[&str]) -> [u8; 32] {
    let mut input = b"turnvector.c17.s3.v1\0".to_vec();
    for path in paths {
        let bytes = fs::read(manifest.join(path))
            .unwrap_or_else(|error| panic!("read S3 source {path}: {error}"));
        input.extend_from_slice(&(path.len() as u64).to_le_bytes());
        input.extend_from_slice(path.as_bytes());
        input.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        input.extend_from_slice(&bytes);
    }
    sha256(&input)
}

fn verify_embedded_ceiling(expected: u64) {
    let source = include_str!("src/c17_generated.rs");
    let marker = "SUPPORT_LEDGER_CEILING_BYTES: u64 = ";
    let value = source
        .split_once(marker)
        .and_then(|(_, tail)| tail.split_once(';'))
        .map(|(value, _)| value.replace('_', ""))
        .and_then(|value| value.parse::<u64>().ok());
    assert_eq!(
        value,
        Some(expected),
        "embedded support ceiling is not B04 output"
    );
}

fn checked_product(left: u64, right: u64) -> Option<u64> {
    left.checked_mul(right)
}

fn checked_sum(values: impl IntoIterator<Item = u64>) -> Option<u64> {
    values
        .into_iter()
        .try_fold(0u64, |total, value| total.checked_add(value))
}

fn index_bytes(
    leaf_bytes: u64,
    header_bytes: u64,
    branch_bytes: u64,
    descriptor_bytes: u64,
    leaves: u64,
) -> Option<u64> {
    let branches = leaves.checked_sub(1)?;
    checked_sum([
        header_bytes,
        checked_product(4, descriptor_bytes)?,
        checked_product(leaves, leaf_bytes)?,
        checked_product(branches, branch_bytes)?,
        checked_product(leaves, 4)?,
        checked_product(branches, 4)?,
    ])
}

fn arena_bytes(inline_bytes: u64, slot: u64, capacity: u64) -> Option<u64> {
    inline_bytes.checked_add(checked_product(capacity, slot.checked_add(4)?)?)
}

fn landed_backing(values: &BTreeMap<String, u64>) -> Option<u64> {
    checked_sum([
        checked_product(
            R_PHYSICAL,
            probe_value(values, "support.legacy_record")
                .checked_add(4)?
                .checked_add(probe_value(values, "support.legacy_avl_node").checked_mul(2)?)?,
        )?,
        checked_product(F_PHYSICAL, probe_value(values, "support.funding_claim"))?,
        checked_product(Q_HISTORY, probe_value(values, "support.monotonic_time"))?,
        checked_product(
            E_HISTORY,
            probe_value(values, "support.bundle_record_slot").checked_add(4)?,
        )?,
        checked_product(
            11 * E_HISTORY,
            probe_value(values, "support.c16_leaf").checked_add(4)?,
        )?,
        checked_product(
            (11 * E_HISTORY).checked_sub(1)?,
            probe_value(values, "support.c16_branch").checked_add(4)?,
        )?,
        checked_product(
            C_CELLS,
            probe_value(values, "support.cell_slot").checked_add(4)?,
        )?,
    ])
}

fn support_ceiling(values: &BTreeMap<String, u64>) -> Option<u64> {
    let index_header = probe_value(values, "index.header_size");
    let branch = index_header;
    let descriptor = probe_value(values, "box_slice.size");
    let arena_inline = probe_value(values, "arena.inline_size");
    let arena = |slot: &str, capacity: usize| {
        arena_bytes(arena_inline, probe_value(values, slot), capacity as u64)
    };
    let values = [
        probe_value(values, "support.landed_prefix"),
        landed_backing(values)?,
        probe_value(values, "c17_header.size"),
        index_bytes(
            probe_value(values, "leaf.32_8"),
            index_header,
            branch,
            descriptor,
            RAW_CAPACITY as u64,
        )?,
        index_bytes(
            probe_value(values, "leaf.17_8"),
            index_header,
            branch,
            descriptor,
            AUTHORITY_CAPACITY as u64,
        )?,
        index_bytes(
            probe_value(values, "leaf.17_8"),
            index_header,
            branch,
            descriptor,
            LOCAL_CAPACITY as u64,
        )?,
        arena("group.size", ROOT_GROUP_CAPACITY)?,
        arena("external_head.size", EXTERNAL_HEAD_CAPACITY)?,
        arena("formation.size", FORMATION_CAPACITY)?,
        arena("funder.size", FUNDER_CAPACITY)?,
        arena("member.size", MEMBER_CAPACITY)?,
        arena("initial_wrapper.size", INITIAL_WRAPPER_CAPACITY)?,
        arena("owner_header.size", SUPPORT_HISTORIES)?,
        arena("owner_row.size", SUPPORT_HISTORIES)?,
        arena("owner_index.size", SUPPORT_HISTORIES)?,
        arena("owner.size", SUPPORT_HISTORIES)?,
        arena("link.size", LINK_CAPACITY)?,
        arena("membership.size", MEMBERSHIP_CAPACITY)?,
        arena("mutation.size", MUTATION_CAPACITY)?,
        arena("lifecycle_record.size", LIFECYCLE_CAPACITY)?,
        probe_value(values, "pending_header.size"),
    ];
    checked_sum(values)
}

fn b04(b03: &[u8], values: &BTreeMap<String, u64>, ceiling: u64) -> Vec<u8> {
    let slot = |name: &str| probe_value(values, name).checked_add(4);
    let reduction_r20 = checked_sum([
        3_072_000,
        checked_product(6_400, slot("formation.size").expect("Formation width"))
            .expect("R20 Formation reduction"),
        checked_product(25_600, slot("funder.size").expect("Funder width"))
            .expect("R20 Funder reduction"),
        checked_product(3_712, slot("initial_wrapper.size").expect("Wrapper width"))
            .expect("R20 Wrapper reduction"),
        checked_product(6_400, slot("mutation.size").expect("Mutation width"))
            .expect("R20 Mutation reduction"),
    ])
    .expect("R20 reduction");
    let reduction_r19 = checked_sum([
        9_707_520,
        checked_product(3_456, slot("group.size").expect("Group width"))
            .expect("R19 Group reduction"),
        checked_product(20_224, slot("formation.size").expect("Formation width"))
            .expect("R19 Formation reduction"),
        checked_product(80_896, slot("funder.size").expect("Funder width"))
            .expect("R19 Funder reduction"),
        checked_product(13_824, slot("member.size").expect("Member width"))
            .expect("R19 Member reduction"),
        checked_product(17_536, slot("initial_wrapper.size").expect("Wrapper width"))
            .expect("R19 Wrapper reduction"),
        checked_product(16_768, slot("mutation.size").expect("Mutation width"))
            .expect("R19 Mutation reduction"),
    ])
    .expect("R19 reduction");
    let local_index = index_bytes(
        probe_value(values, "leaf.17_8"),
        probe_value(values, "index.header_size"),
        probe_value(values, "index.header_size"),
        probe_value(values, "box_slice.size"),
        LOCAL_CAPACITY as u64,
    )
    .expect("Local index");
    format!(
        concat!(
            "turnvector.c17.b04.v1\n",
            "b03_sha256={}\n",
            "landed_prefix={}\n",
            "landed_backing={}\n",
            "legacy_record_bytes={}\n",
            "legacy_avl_node_bytes={}\n",
            "funding_claim_bytes={}\n",
            "group_bytes={}\n",
            "formation_bytes={}\n",
            "funder_bytes={}\n",
            "member_bytes={}\n",
            "wrapper_bytes={}\n",
            "mutation_bytes={}\n",
            "v={}\n",
            "horizons={},{},{}\n",
            "q_history={}\n",
            "r_physical={}\n",
            "f_physical={}\n",
            "e_history={}\n",
            "c_cells={}\n",
            "formation_max={}\n",
            "local_index_bytes={}\n",
            "reduction_r20={}\n",
            "reduction_r19={}\n",
            "support_ledger_ceiling_bytes={}\n"
        ),
        hex(sha256(b03)),
        probe_value(values, "support.landed_prefix"),
        landed_backing(values).expect("landed backing"),
        probe_value(values, "support.legacy_record"),
        probe_value(values, "support.legacy_avl_node"),
        probe_value(values, "support.funding_claim"),
        probe_value(values, "group.size"),
        probe_value(values, "formation.size"),
        probe_value(values, "funder.size"),
        probe_value(values, "member.size"),
        probe_value(values, "initial_wrapper.size"),
        probe_value(values, "mutation.size"),
        V_AXES,
        HORIZONS[0],
        HORIZONS[1],
        HORIZONS[2],
        Q_HISTORY,
        R_PHYSICAL,
        F_PHYSICAL,
        E_HISTORY,
        C_CELLS,
        FORMATION_CAPACITY,
        local_index,
        reduction_r20,
        reduction_r19,
        ceiling,
    )
    .into_bytes()
}

fn b05(s3: [u8; 32], d3: [u8; 32], s4: [u8; 32], d4: [u8; 32], s5: [u8; 32]) -> Vec<u8> {
    format!(
        concat!(
            "turnvector.c17.b05.v1\n",
            "s3={}\n",
            "d3={}\n",
            "s4={}\n",
            "d4={}\n",
            "s5={}\n",
            "chain=s3->d3;(s4,d3)->d4;(s5,d3,d4)->d5\n",
            "b03_double_run=identical\n",
            "b04_double_run=identical\n"
        ),
        hex(s3),
        hex(d3),
        hex(s4),
        hex(d4),
        hex(s5),
    )
    .into_bytes()
}

fn write(directory: &Path, name: &str, payload: &[u8]) {
    fs::write(directory.join(name), payload)
        .unwrap_or_else(|error| panic!("write {name}: {error}"));
}

fn chained_digest(domain: &[u8], values: &[&[u8]]) -> [u8; 32] {
    let length = domain.len() + values.iter().map(|value| value.len()).sum::<usize>();
    let mut input = Vec::with_capacity(length);
    input.extend_from_slice(domain);
    for value in values {
        input.extend_from_slice(value);
    }
    sha256(&input)
}

fn hex(value: [u8; 32]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(64);
    for byte in value {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 15) as usize] as char);
    }
    output
}

fn sha256(input: &[u8]) -> [u8; 32] {
    let mut state = SHA_INITIAL;
    let full = input.len() / 64;
    for block in input[..full * 64].chunks_exact(64) {
        compress(&mut state, block.try_into().expect("SHA block"));
    }
    let remainder = input.len() % 64;
    let mut final_block = [0; 64];
    final_block[..remainder].copy_from_slice(&input[full * 64..]);
    final_block[remainder] = 0x80;
    if remainder >= 56 {
        compress(&mut state, &final_block);
        final_block = [0; 64];
    }
    let bits = u64::try_from(input.len())
        .expect("build input fits u64")
        .checked_mul(8)
        .expect("SHA bit length");
    final_block[56..].copy_from_slice(&bits.to_be_bytes());
    compress(&mut state, &final_block);
    let mut output = [0; 32];
    for (bytes, word) in output.chunks_exact_mut(4).zip(state) {
        bytes.copy_from_slice(&word.to_be_bytes());
    }
    output
}

fn compress(state: &mut [u32; 8], block: &[u8; 64]) {
    let mut words = [0; 16];
    for (index, word) in words.iter_mut().enumerate() {
        *word = u32::from_be_bytes(
            block[index * 4..index * 4 + 4]
                .try_into()
                .expect("SHA word"),
        );
    }
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for (index, constant) in SHA_ROUNDS.into_iter().enumerate() {
        let word = if index < 16 {
            words[index]
        } else {
            let x = words[(index - 15) % 16];
            let y = words[(index - 2) % 16];
            let word = words[(index - 16) % 16]
                .wrapping_add(x.rotate_right(7) ^ x.rotate_right(18) ^ (x >> 3))
                .wrapping_add(words[(index - 7) % 16])
                .wrapping_add(y.rotate_right(17) ^ y.rotate_right(19) ^ (y >> 10));
            words[index % 16] = word;
            word
        };
        let first = h
            .wrapping_add(e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25))
            .wrapping_add((e & f) ^ ((!e) & g))
            .wrapping_add(constant)
            .wrapping_add(word);
        let second = (a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22))
            .wrapping_add((a & b) ^ (a & c) ^ (b & c));
        (h, g, f, e, d, c, b, a) = (
            g,
            f,
            e,
            d.wrapping_add(first),
            c,
            b,
            a,
            first.wrapping_add(second),
        );
    }
    for (slot, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
        *slot = slot.wrapping_add(value);
    }
}
