//! Rust source for a validated module (see main.rs).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use wtime_types::{FeatureSet, FuncType, HeapType, ResourceLimits, ValType};
use wtime_validate::{
    DecodedModule, ExternalKind, Immediate, ImportKind, Instruction, SegmentMode, validate,
};

type Res<T> = Result<T, String>;

/// Output crate kind: standalone Goose CLI executable or Blot ABI 8 shared library.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    GooseBin,
    BlotCdylib,
}

/// The native stack the program runs on by default (`--stack-limit-mib`
/// sets it): enough for the Goose compilers many times over, and what a
/// runaway recursion costs before it traps (the interpreter's 4,000,000
/// frames take about 700 MiB).
const STACK_BYTES: u64 = 1 << 30;
/// A functions file is closed once it holds this many bytes. rustc's
/// lints copy the rest of a file to make some suggestions, so their cost
/// grows with the file.
const FILE_BYTES: usize = 1 << 20;

/// Locals the module's functions declare at most, together (the Goose
/// compilers declare about 56,000), and blocks nested at most (they nest
/// 39 deep; rustc's own stack overflows near 1,000).
pub(crate) const MAX_LOCALS: usize = 1_000_000;
pub(crate) const MAX_NESTING: usize = 512;

/// Pending expressions on the operand stack at most (see `push_pure`).
pub(crate) const MAX_PENDING: usize = 64;

/// The generated functions' source is refused past this size (the 64-bit
/// Goose compiler, a 9 MB module, makes about 130 MB).
pub(crate) const MAX_SOURCE_BYTES: usize = 1 << 30;

/// Functions without calls (or small acyclic subtrees) with at most this many
/// locals, instructions and call depth skip the stack check.
const LEAF_LOCALS: usize = 256;
const LEAF_INSTRUCTIONS: usize = 2_000;
const LEAF_CHAIN_DEPTH: usize = 8;

/// Globals held in locals add at most about this many lines per
/// instruction of a function (see `Function::emit`).
pub(crate) const GLOBAL_LINES_PER_INSTRUCTION: usize = 8;

/// At most this many module-wide preserved globals are passed in registers.
const MAX_REG_GLOBALS: usize = 4;

/// Non-recursive inlining thresholds.
const INLINE_ALWAYS_LEAF_INSTRUCTIONS: usize = 90;
const INLINE_ALWAYS_SMALL_LEAF_INSTRUCTIONS: usize = 160;
const INLINE_ALWAYS_SMALL_LEAF_MAX_CALLERS: usize = 8;
const INLINE_INSTRUCTIONS: usize = 80;
const INLINE_LEAF_INSTRUCTIONS: usize = 350;
const INLINE_LEAF_MAX_CALLERS: usize = 3;
const INLINE_SINGLE_CALLER_INSTRUCTIONS: usize = 400;

/// Chunked 64-bit flat-region specialization bounds (for non-growing regions
/// guarded by `if mf`).
const FLAT_REGION_MAX_INSTRUCTIONS: usize = 2_000;
const FLAT_LOOP_MIN_MEM_OPS: usize = 2;
const FLAT_BLOCK_MIN_MEM_OPS: usize = 12;

/// Modules whose generated function source fits under this threshold build
/// with a single codegen unit for whole-program optimization.
const SINGLE_CGU_BYTES: usize = 4 << 20;

/// Target source bytes per part crate for large modules (balanced via LPT).
const TARGET_PART_BYTES: usize = 6 << 20;
const MAX_PARTS: usize = 12;

/// `memory.copy`s of a constant length up to this many bytes are inlined.
pub(crate) const COPY_INLINE: usize = 256;

/// `call_indirect` dispatchers with at least this many matching targets use
/// an O(1) static function-pointer table instead of a `match` branch tree.
pub(crate) const CI_TABLE_MIN_TARGETS: usize = 5;
pub(crate) const CI_TABLE_MAX_SPAN: usize = 65_536;

/// Validates a module with `wtime-aot`'s generous AOT resource limits.
pub(crate) fn validate_module(bytes: &[u8]) -> Res<wtime_validate::ValidatedModule> {
    let limits = ResourceLimits {
        max_module_bytes: 256 << 20,
        max_instructions: 100_000_000,
        max_memory_pages: 1 << 48,
        max_memory_bytes: u64::MAX,
        max_functions: 1 << 24,
        ..ResourceLimits::default()
    };
    validate(bytes, &FeatureSet::all_phase3_core(), &limits)
        .map_err(|e| format!("invalid module: {e}"))
}

/// The declared maximum of the module's 64-bit memory in pages (the index
/// type's limit where it declares none), if it has one: what a flat memory
/// would allocate.
pub fn memory64_max(bytes: &[u8]) -> Res<Option<u64>> {
    let limits = ResourceLimits {
        max_module_bytes: 256 << 20,
        max_instructions: 100_000_000,
        ..ResourceLimits::default()
    };
    let validated = validate(bytes, &FeatureSet::all_phase3_core(), &limits)
        .map_err(|e| format!("invalid module: {e}"))?;
    Ok(validated
        .decoded()
        .memories
        .first()
        .filter(|mt| mt.index_type == wtime_types::IndexType::I64)
        .map(|mt| mt.limits.max.unwrap_or(1 << 48)))
}

/// `flat64`: a 64-bit memory is allocated whole at its maximum
/// (`Memory::new_flat`) and accessed by `m64`'s flat accesses.
#[cfg(test)]
pub fn generate(
    bytes: &[u8],
    out: &Path,
    name: &str,
    rt: &Path,
    module_path: &str,
    flat64: bool,
) -> Res<()> {
    generate_target(bytes, out, name, rt, module_path, flat64, Target::GooseBin)
}

/// Generates a crate for `target` (Goose CLI binary or Blot ABI 8 cdylib).
#[allow(dead_code)]
pub fn generate_target(
    bytes: &[u8],
    out: &Path,
    name: &str,
    rt: &Path,
    module_path: &str,
    flat64: bool,
    target: Target,
) -> Res<()> {
    generate_target_with_deadline(bytes, out, name, rt, module_path, flat64, target, None)
}

/// Generates a crate for `target` while cooperatively polling `deadline`.
#[allow(clippy::too_many_arguments)]
pub fn generate_target_with_deadline(
    bytes: &[u8],
    out: &Path,
    name: &str,
    rt: &Path,
    module_path: &str,
    flat64: bool,
    target: Target,
    deadline: Option<std::time::Instant>,
) -> Res<()> {
    let check_dl = |phase: &str| -> Res<()> {
        if let Some(dl) = deadline
            && std::time::Instant::now() >= dl
        {
            return Err(format!("{phase}: exceeded build time limit"));
        }
        Ok(())
    };
    check_dl("rust codegen init")?;
    // Room for programs several times the size of the 64-bit Goose compiler
    // (9 MB, about 5 million instructions), and no more.
    let limits = ResourceLimits {
        max_module_bytes: 256 << 20,
        max_instructions: 100_000_000,
        max_memory_pages: 1 << 48,
        max_memory_bytes: u64::MAX,
        max_functions: 1 << 24,
        ..ResourceLimits::default()
    };
    let validated = validate(bytes, &FeatureSet::all_phase3_core(), &limits)
        .map_err(|e| format!("invalid module: {e}"))?;
    check_dl("rust codegen validate")?;
    let m = validated.decoded();
    let module = Module::new(m, flat64, target)?;
    check_dl("rust codegen analyze")?;
    let write = |path: &Path, text: &str| {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
    };
    let ident = |crate_name: &str| crate_name.replace('-', "_");
    let core = format!("{name}-core");
    let rt = rt.display().to_string();

    // Generate all defined functions, then balance them across part crates
    // with Longest-Processing-Time-first bin-packing so no single part crate
    // accumulates all of the largest functions and stalls parallel builds.
    let all_fns = module.functions(m, deadline)?;
    check_dl("rust codegen functions")?;
    let total_fn_bytes: usize = all_fns.iter().map(|(_, t)| t.len()).sum();
    let n_parts = if total_fn_bytes <= SINGLE_CGU_BYTES {
        1
    } else {
        (total_fn_bytes / TARGET_PART_BYTES + 1).clamp(2, MAX_PARTS)
    };
    let mut parts: Vec<Vec<(usize, String)>> = (0..n_parts).map(|_| Vec::new()).collect();
    let mut part_load = vec![0usize; n_parts];
    let mut order: Vec<usize> = (0..all_fns.len()).collect();
    if n_parts > 1 {
        order.sort_by_key(|&k| (std::cmp::Reverse(all_fns[k].1.len()), all_fns[k].0));
    }
    let mut all_slots: Vec<Option<(usize, String)>> = all_fns.into_iter().map(Some).collect();
    let mut part_of = vec![0; module.types.len()];
    for k in order {
        let (index, text) = all_slots[k].take().unwrap();
        let best = (0..n_parts).min_by_key(|&p| part_load[p]).unwrap();
        part_load[best] += text.len();
        part_of[index] = best;
        parts[best].push((index, text));
    }
    for part in &mut parts {
        part.sort_by_key(|(idx, _)| *idx);
    }
    let part_names: Vec<String> = (0..parts.len()).map(|j| format!("{name}-p{j}")).collect();

    let package = |crate_name: &str, dependencies: &str| {
        format!(
            "# Generated by wtime-aot. Do not edit.\n[package]\nname = \"{crate_name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n[dependencies]\n{dependencies}"
        )
    };
    let mut members = String::from("\"core\"");
    let mut dependencies = format!("{core} = {{ path = \"core\" }}\n");
    for (j, part) in part_names.iter().enumerate() {
        write!(members, ", \"p{j}\"").unwrap();
        writeln!(dependencies, "{part} = {{ path = \"p{j}\" }}").unwrap();
    }
    let cgu = if total_fn_bytes <= SINGLE_CGU_BYTES {
        1
    } else {
        16
    };
    let lib_section = if target == Target::BlotCdylib {
        "\n[lib]\ncrate-type = [\"cdylib\"]\n"
    } else {
        ""
    };
    write(
        &out.join("Cargo.toml"),
        &(package(name, &dependencies)
            + lib_section
            + &format!(
                "\n# Isolated from any enclosing workspace. The functions are split into\n\
                 # crates of their own so that rustc's front end, which is sequential\n\
                 # within a crate, runs on each part in parallel and in less memory.\n\
                 [workspace]\nmembers = [{members}]\n\n\
                 [profile.release]\nopt-level = 3\ncodegen-units = {cgu}\nlto = \"thin\"\npanic = \"abort\"\ndebug = 0\n"
            )),
    )?;
    write(
        &out.join("core/Cargo.toml"),
        &package(&core, &format!("wtime-aot-rt = {{ path = {rt:?} }}\n")),
    )?;
    write(&out.join("core/src/lib.rs"), &module.core(m)?)?;
    let file_head = format!(
        "// Generated by wtime-aot. Do not edit.\nuse {}::*;\n\n",
        ident(&core)
    );
    for (j, part) in parts.iter().enumerate() {
        write(
            &out.join(format!("p{j}/Cargo.toml")),
            &package(
                &part_names[j],
                &format!("{core} = {{ path = \"../core\" }}\n"),
            ),
        )?;
        let mut lib = String::from(
            "// Generated by wtime-aot. Do not edit.\n#![forbid(unsafe_code)]\n#![allow(unused, non_snake_case, unreachable_code, unused_parens, unused_mut, unused_assignments, clippy::all)]\n\n",
        );
        let mut file = file_head.clone();
        let mut k = 0;
        for (n, (_, text)) in part.iter().enumerate() {
            file.push_str(text);
            if file.len() >= FILE_BYTES || n + 1 == part.len() {
                write(&out.join(format!("p{j}/src/f{k}.rs")), &file)?;
                writeln!(lib, "mod f{k};\npub use f{k}::*;").unwrap();
                file = file_head.clone();
                k += 1;
            }
        }
        write(&out.join(format!("p{j}/src/lib.rs")), &lib)?;
    }
    let mut data = Vec::new();
    let paths: Vec<String> = (0..module.types.len())
        .map(|f| format!("{}::f{f}", ident(&part_names[part_of[f]])))
        .collect();
    match target {
        Target::GooseBin => {
            let main = module.main(m, &mut data, module_path, &ident(&core), &paths)?;
            write(&out.join("src/main.rs"), &main)?;
        }
        Target::BlotCdylib => {
            let lib = module.blot_lib(m, &mut data, &ident(&core), &paths)?;
            write(&out.join("src/lib.rs"), &lib)?;
        }
    }
    std::fs::write(out.join("src/data.bin"), &data).map_err(|e| format!("data.bin: {e}"))?;
    Ok(())
}

fn rust_type(t: ValType) -> Res<&'static str> {
    Ok(match t {
        ValType::I32 => "i32",
        ValType::I64 => "i64",
        ValType::F32 => "f32",
        ValType::F64 => "f64",
        ValType::V128 => "u128",
        ValType::Ref(_) => "u64",
    })
}

fn zero(t: ValType) -> Res<&'static str> {
    Ok(match t {
        ValType::I32 => "0i32",
        ValType::I64 => "0i64",
        ValType::F32 => "0f32",
        ValType::F64 => "0f64",
        ValType::V128 => "0u128",
        ValType::Ref(_) => "0u64",
    })
}

/// The Rust type of a function's results: (), T or a tuple.
fn result_type(results: &[ValType]) -> Res<String> {
    Ok(match results {
        [] => "()".into(),
        [t] => rust_type(*t)?.into(),
        ts => format!(
            "({})",
            ts.iter()
                .map(|t| rust_type(*t))
                .collect::<Res<Vec<_>>>()?
                .join(", ")
        ),
    })
}

/// `Ok(...)` of values as a function returns them.
fn ok_values(values: &[String]) -> String {
    match values {
        [] => "Ok(())".into(),
        [v] => format!("Ok({v})"),
        vs => format!("Ok(({}))", vs.join(", ")),
    }
}

/// Native Rust return ABI for generated Wasm functions.
/// Using `Option<Trap>` / `(T, Option<Trap>)` puts the payload and the 1-byte
/// trap tag in two separate registers (`ScalarPair`), avoiding `Result<i32, Trap>`'s
/// 64-bit packed bit-mangling and `Result<(i32, i32), Trap>`'s `sret` stack spills.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RetAbi {
    Void,
    Scalar(ValType),
    Pair32(ValType, ValType),
    Result,
}

fn ret_abi(results: &[ValType]) -> RetAbi {
    match results {
        [] => RetAbi::Void,
        // `Result<i32, Trap>` packs `(val << 32) | tag` into one 64-bit register
        // (1 register clobbered instead of 2, and `shr $32` gives LLVM known-zero
        // upper 32 bits for loop-idiom/popcnt passes). `f32`/`i64`/`f64`/`ref`
        // use `ScalarPair` (`XMM0`/`RAX` + `DL`), and 2x32-bit pairs use `Pair32`
        // (`RAX` + `DL`) to avoid `Result<(i32, i32), Trap>`'s `sret` stack spill.
        [t @ (ValType::I64 | ValType::F32 | ValType::F64 | ValType::Ref(_))] => RetAbi::Scalar(*t),
        [
            a @ (ValType::I32 | ValType::F32),
            b @ (ValType::I32 | ValType::F32),
        ] => RetAbi::Pair32(*a, *b),
        _ => RetAbi::Result,
    }
}

fn fn_return_type(results: &[ValType]) -> Res<String> {
    Ok(match ret_abi(results) {
        RetAbi::Void => "Option<Trap>".into(),
        RetAbi::Scalar(t) => format!("({}, Option<Trap>)", rust_type(t)?),
        RetAbi::Pair32(_, _) => "(u64, Option<Trap>)".into(),
        RetAbi::Result => format!("R<{}>", result_type(results)?),
    })
}

fn ret_err_expr(abi: RetAbi, err: &str) -> Res<String> {
    Ok(match abi {
        RetAbi::Void => format!("cold_trap({err})"),
        RetAbi::Scalar(t) => format!("({}, cold_trap({err}))", zero(t)?),
        RetAbi::Pair32(_, _) => format!("(0u64, cold_trap({err}))"),
        RetAbi::Result => format!("Err({err})"),
    })
}

fn try_suffix(abi: RetAbi) -> Res<String> {
    Ok(match abi {
        RetAbi::Void => ", void".into(),
        RetAbi::Scalar(t) => format!(", {}", zero(t)?),
        RetAbi::Pair32(_, _) => ", 0u64".into(),
        RetAbi::Result => String::new(),
    })
}

fn pack_u32_lane(t: ValType, expr: &str) -> String {
    if t == ValType::F32 {
        format!("({expr}).to_bits() as u64")
    } else {
        format!("({expr}) as u32 as u64")
    }
}

fn unpack_u32_lane(t: ValType, expr: &str) -> String {
    if t == ValType::F32 {
        format!("f32::from_bits({expr})")
    } else {
        format!("({expr}) as i32")
    }
}

fn ok_return_expr(abi: RetAbi, values: &[String]) -> String {
    match abi {
        RetAbi::Void => "None".into(),
        RetAbi::Scalar(_) => format!("({}, None)", values[0]),
        RetAbi::Pair32(a, b) => {
            let lo = pack_u32_lane(a, &values[0]);
            let hi = pack_u32_lane(b, &values[1]);
            format!("((({lo}) | (({hi}) << 32)), None)")
        }
        RetAbi::Result => ok_values(values),
    }
}

/// Converts a `Result<T, Trap>` expression into `RetAbi` (used by `call_indirect`
/// wrappers around host imports).
fn result_to_ret_abi(results: &[ValType], expr: &str) -> Res<String> {
    Ok(match ret_abi(results) {
        RetAbi::Void => format!("match {expr} {{ Ok(()) => None, Err(t) => cold_trap(t) }}"),
        RetAbi::Scalar(t) => format!(
            "match {expr} {{ Ok(v) => (v, None), Err(t) => ({}, cold_trap(t)) }}",
            zero(t)?
        ),
        RetAbi::Pair32(a, b) => {
            let lo = pack_u32_lane(a, "v0");
            let hi = pack_u32_lane(b, "v1");
            format!(
                "match {expr} {{ Ok((v0, v1)) => ((({lo}) | (({hi}) << 32)), None), Err(t) => (0u64, cold_trap(t)) }}"
            )
        }
        RetAbi::Result => expr.to_string(),
    })
}

/// Compact dynamic bitset for per-function global read/write sets.
#[derive(Clone, Default)]
pub(crate) struct BitSet {
    words: Vec<u64>,
}

impl BitSet {
    pub(crate) fn new(bits: usize) -> BitSet {
        BitSet {
            words: vec![0; bits.div_ceil(64)],
        }
    }
    pub(crate) fn insert(&mut self, bit: u32) {
        let idx = (bit as usize) / 64;
        if idx >= self.words.len() {
            self.words.resize(idx + 1, 0);
        }
        self.words[idx] |= 1u64 << (bit % 64);
    }
    pub(crate) fn contains(&self, bit: u32) -> bool {
        self.words
            .get((bit as usize) / 64)
            .is_some_and(|w| (w & (1u64 << (bit % 64))) != 0)
    }
    pub(crate) fn remove(&mut self, bit: u32) {
        if let Some(w) = self.words.get_mut((bit as usize) / 64) {
            *w &= !(1u64 << (bit % 64));
        }
    }
    pub(crate) fn set(&mut self, bit: u32, value: bool) {
        if value {
            self.insert(bit);
        } else {
            self.remove(bit);
        }
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.words.iter().all(|&w| w == 0)
    }
    pub(crate) fn union_with(&mut self, other: &BitSet) -> bool {
        if self.words.len() < other.words.len() {
            self.words.resize(other.words.len(), 0);
        }
        let mut changed = false;
        for (a, &b) in self.words.iter_mut().zip(&other.words) {
            let next = *a | b;
            if next != *a {
                *a = next;
                changed = true;
            }
        }
        changed
    }
    pub(crate) fn intersect_with(&mut self, other: &BitSet) {
        for (a, &b) in self.words.iter_mut().zip(&other.words) {
            *a &= b;
        }
        if self.words.len() > other.words.len() {
            for a in &mut self.words[other.words.len()..] {
                *a = 0;
            }
        }
    }
}

/// Compile-time constant value of an effectively immutable global.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConstVal {
    I32(i32),
    I64(i64),
    F32(u32),
    F64(u64),
}

impl ConstVal {
    fn rust_literal(self) -> String {
        match self {
            ConstVal::I32(v) => format!("({v}i32)"),
            ConstVal::I64(v) => format!("({v}i64)"),
            ConstVal::F32(bits) => format!("f32::from_bits({bits:#x})"),
            ConstVal::F64(bits) => format!("f64::from_bits({bits:#x})"),
        }
    }
}

/// Evaluates a Wasm constant expression if all referenced globals are known constants.
pub(crate) fn eval_constant(
    expression: &[Instruction],
    consts: &[Option<ConstVal>],
) -> Option<ConstVal> {
    let mut stack: Vec<ConstVal> = Vec::new();
    for i in expression {
        let value = match (i.opcode, &i.immediate) {
            (0x0b, _) => break,
            (0x41, Immediate::I32(v)) => ConstVal::I32(*v),
            (0x42, Immediate::I64(v)) => ConstVal::I64(*v),
            (0x43, Immediate::F32(v)) => ConstVal::F32(*v),
            (0x44, Immediate::F64(v)) => ConstVal::F64(*v),
            (0x23, Immediate::Index(g)) => (*consts.get(*g as usize)?)?,
            (op @ (0x6a..=0x6c), _) => {
                let (Some(ConstVal::I32(b)), Some(ConstVal::I32(a))) = (stack.pop(), stack.pop())
                else {
                    return None;
                };
                ConstVal::I32(match op {
                    0x6a => a.wrapping_add(b),
                    0x6b => a.wrapping_sub(b),
                    _ => a.wrapping_mul(b),
                })
            }
            (op @ (0x7c..=0x7e), _) => {
                let (Some(ConstVal::I64(b)), Some(ConstVal::I64(a))) = (stack.pop(), stack.pop())
                else {
                    return None;
                };
                ConstVal::I64(match op {
                    0x7c => a.wrapping_add(b),
                    0x7d => a.wrapping_sub(b),
                    _ => a.wrapping_mul(b),
                })
            }
            _ => return None,
        };
        stack.push(value);
    }
    if stack.len() == 1 { stack.pop() } else { None }
}

/// Visits live instructions of a function body, skipping dead code after
/// unconditional branches identically to `Function::instruction`.
pub(crate) fn visit_live(code: &[Instruction], noreturn: &[bool], mut f: impl FnMut(&Instruction)) {
    let mut unreachable_stack = vec![false];
    let mut dead = 0usize;
    for i in code {
        let op = i.opcode;
        if *unreachable_stack.last().unwrap_or(&false) {
            match op {
                0x02..=0x04 => {
                    dead += 1;
                    continue;
                }
                0x05 if dead > 0 => continue,
                0x0b if dead > 0 => {
                    dead -= 1;
                    continue;
                }
                0x05 | 0x0b => {}
                _ => continue,
            }
        }
        f(i);
        match op {
            0x02..=0x04 => unreachable_stack.push(false),
            0x05 => {
                if let Some(u) = unreachable_stack.last_mut() {
                    *u = false;
                }
                dead = 0;
            }
            0x0b => {
                unreachable_stack.pop();
                if unreachable_stack.is_empty() {
                    break;
                }
            }
            0x00 | 0x0c | 0x0e | 0x0f => {
                if let Some(u) = unreachable_stack.last_mut() {
                    *u = true;
                }
            }
            0x10 => {
                if let Immediate::Index(idx) = i.immediate
                    && noreturn.get(idx as usize).copied().unwrap_or(false)
                    && let Some(u) = unreachable_stack.last_mut()
                {
                    *u = true;
                }
            }
            _ => {}
        }
    }
}

/// Sound structured-control-flow proof that every reachable `global.set g` in
/// `code` writes a value equal to `g`'s own value (assuming `g` starts equal to
/// its constant initializer and no callee ever modifies it). If every defined
/// function passes, `g` is provably constant across the entire module.
fn function_writes_only_same_global(
    code: &[Instruction],
    n_locals: usize,
    types: &[FuncType],
    module_types: &[FuncType],
    noreturn: &[bool],
    g: u32,
) -> bool {
    let mut writes_g = false;
    visit_live(code, noreturn, |i| {
        if i.opcode == 0x24 && matches!(i.immediate, Immediate::Index(x) if x == g) {
            writes_g = true;
        }
    });
    if !writes_g {
        return true;
    }

    let mut loop_mod_locals: BTreeMap<usize, Vec<u32>> = BTreeMap::new();
    {
        let mut open_loops: Vec<(usize, usize)> = Vec::new();
        let mut depth = 0usize;
        for (pc, i) in code.iter().enumerate() {
            match i.opcode {
                0x02 | 0x04 => depth += 1,
                0x03 => {
                    depth += 1;
                    open_loops.push((pc, depth));
                }
                0x0b => {
                    if open_loops.last().is_some_and(|&(_, d)| d == depth) {
                        open_loops.pop();
                    }
                    depth = depth.saturating_sub(1);
                }
                0x21 | 0x22 => {
                    if let Immediate::Index(x) = i.immediate {
                        for &(lpc, _) in &open_loops {
                            let v = loop_mod_locals.entry(lpc).or_default();
                            if !v.contains(&x) {
                                v.push(x);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    struct CFrame {
        kind: Kind,
        height: usize,
        results: usize,
        unreachable: bool,
        end_reached: bool,
        end_locals: Vec<bool>,
        if_locals: Vec<bool>,
        has_else: bool,
    }

    let merge = |frame: &mut CFrame, locals: &[bool]| {
        if !frame.end_reached {
            frame.end_reached = true;
            frame.end_locals = locals.to_vec();
        } else {
            for (a, &b) in frame.end_locals.iter_mut().zip(locals) {
                *a &= b;
            }
        }
    };

    let mut locals = vec![false; n_locals];
    let mut stack: Vec<bool> = Vec::new();
    let mut dead = 0usize;
    let mut frames = vec![CFrame {
        kind: Kind::Function,
        height: 0,
        results: 0,
        unreachable: false,
        end_reached: false,
        end_locals: Vec::new(),
        if_locals: Vec::new(),
        has_else: false,
    }];

    for (pc, i) in code.iter().enumerate() {
        let op = i.opcode;
        if frames.last().is_some_and(|f| f.unreachable) {
            match op {
                0x02..=0x04 => {
                    dead += 1;
                    continue;
                }
                0x05 if dead > 0 => continue,
                0x0b if dead > 0 => {
                    dead -= 1;
                    continue;
                }
                0x05 | 0x0b => {}
                _ => continue,
            }
        }
        let block_results = |imm: &Immediate| -> Option<usize> {
            match imm {
                Immediate::Block(-64) => Some(0),
                Immediate::Block(-5..=-1 | -16 | -17) | Immediate::BlockType(_) => Some(1),
                Immediate::Block(n) if *n >= 0 => {
                    let ft = module_types.get(*n as usize)?;
                    if ft.params.is_empty() {
                        Some(ft.results.len())
                    } else {
                        None
                    }
                }
                _ => None,
            }
        };
        match op {
            0x00 => frames.last_mut().unwrap().unreachable = true,
            0x01 => {}
            0x02 | 0x03 => {
                let Some(results) = block_results(&i.immediate) else {
                    return false;
                };
                if op == 0x03
                    && let Some(mods) = loop_mod_locals.get(&pc)
                {
                    for &x in mods {
                        if let Some(slot) = locals.get_mut(x as usize) {
                            *slot = false;
                        }
                    }
                }
                frames.push(CFrame {
                    kind: if op == 0x03 { Kind::Loop } else { Kind::Block },
                    height: stack.len(),
                    results,
                    unreachable: false,
                    end_reached: false,
                    end_locals: Vec::new(),
                    if_locals: Vec::new(),
                    has_else: false,
                });
            }
            0x04 => {
                let Some(results) = block_results(&i.immediate) else {
                    return false;
                };
                if stack.pop().is_none() {
                    return false;
                }
                frames.push(CFrame {
                    kind: Kind::If,
                    height: stack.len(),
                    results,
                    unreachable: false,
                    end_reached: false,
                    end_locals: Vec::new(),
                    if_locals: locals.clone(),
                    has_else: false,
                });
            }
            0x05 => {
                let Some(frame) = frames.last_mut() else {
                    return false;
                };
                if !frame.unreachable {
                    merge(frame, &locals);
                }
                frame.has_else = true;
                frame.unreachable = false;
                locals.clone_from(&frame.if_locals);
                stack.truncate(frame.height);
                dead = 0;
            }
            0x0b => {
                let Some(mut frame) = frames.pop() else {
                    return false;
                };
                if frame.kind == Kind::Function {
                    return true;
                }
                if !frame.unreachable {
                    merge(&mut frame, &locals);
                }
                if frame.kind == Kind::If && !frame.has_else {
                    let if_locals = std::mem::take(&mut frame.if_locals);
                    merge(&mut frame, &if_locals);
                }
                stack.truncate(frame.height);
                if frame.end_reached {
                    locals = frame.end_locals;
                    stack.resize(stack.len() + frame.results, false);
                } else if let Some(parent) = frames.last_mut() {
                    parent.unreachable = true;
                }
            }
            0x0c => {
                let Immediate::Index(d) = i.immediate else {
                    return false;
                };
                let Some(at) = frames.len().checked_sub(d as usize + 1) else {
                    return false;
                };
                if matches!(frames[at].kind, Kind::Block | Kind::If) {
                    merge(&mut frames[at], &locals);
                }
                frames.last_mut().unwrap().unreachable = true;
            }
            0x0d => {
                let Immediate::Index(d) = i.immediate else {
                    return false;
                };
                if stack.pop().is_none() {
                    return false;
                }
                let Some(at) = frames.len().checked_sub(d as usize + 1) else {
                    return false;
                };
                if matches!(frames[at].kind, Kind::Block | Kind::If) {
                    merge(&mut frames[at], &locals);
                }
            }
            0x0e => {
                let Immediate::BrTable(ref targets, default) = i.immediate else {
                    return false;
                };
                if stack.pop().is_none() {
                    return false;
                }
                for &d in targets.iter().chain(std::iter::once(&default)) {
                    let Some(at) = frames.len().checked_sub(d as usize + 1) else {
                        return false;
                    };
                    if matches!(frames[at].kind, Kind::Block | Kind::If) {
                        merge(&mut frames[at], &locals);
                    }
                }
                frames.last_mut().unwrap().unreachable = true;
            }
            0x0f => frames.last_mut().unwrap().unreachable = true,
            0x10 => {
                let Immediate::Index(f) = i.immediate else {
                    return false;
                };
                let Some(ty) = types.get(f as usize) else {
                    return false;
                };
                if stack.len() < ty.params.len() {
                    return false;
                }
                stack.truncate(stack.len() - ty.params.len());
                if noreturn.get(f as usize).copied().unwrap_or(false) {
                    frames.last_mut().unwrap().unreachable = true;
                } else {
                    stack.resize(stack.len() + ty.results.len(), false);
                }
            }
            0x11 => {
                let Immediate::Pair(ti, _) = i.immediate else {
                    return false;
                };
                let Some(ty) = module_types.get(ti as usize) else {
                    return false;
                };
                if stack.len() < ty.params.len() + 1 {
                    return false;
                }
                stack.truncate(stack.len() - (ty.params.len() + 1));
                stack.resize(stack.len() + ty.results.len(), false);
            }
            0x1a => {
                if stack.pop().is_none() {
                    return false;
                }
            }
            0x1b | 0x1c => {
                if stack.len() < 3 {
                    return false;
                }
                let _c = stack.pop().unwrap();
                let b = stack.pop().unwrap();
                let a = stack.pop().unwrap();
                stack.push(a && b);
            }
            0x20 => {
                let Immediate::Index(x) = i.immediate else {
                    return false;
                };
                let Some(&v) = locals.get(x as usize) else {
                    return false;
                };
                stack.push(v);
            }
            0x21 => {
                let Immediate::Index(x) = i.immediate else {
                    return false;
                };
                let Some(v) = stack.pop() else { return false };
                let Some(slot) = locals.get_mut(x as usize) else {
                    return false;
                };
                *slot = v;
            }
            0x22 => {
                let Immediate::Index(x) = i.immediate else {
                    return false;
                };
                let Some(&v) = stack.last() else { return false };
                let Some(slot) = locals.get_mut(x as usize) else {
                    return false;
                };
                *slot = v;
            }
            0x23 => {
                let Immediate::Index(x) = i.immediate else {
                    return false;
                };
                stack.push(x == g);
            }
            0x24 => {
                let Immediate::Index(x) = i.immediate else {
                    return false;
                };
                let Some(v) = stack.pop() else { return false };
                if x == g && !v {
                    return false;
                }
            }
            0x25 => {
                if stack.pop().is_none() {
                    return false;
                }
                stack.push(false);
            }
            0x26 => {
                if stack.len() < 2 {
                    return false;
                }
                stack.truncate(stack.len() - 2);
            }
            0xd0 => stack.push(false),
            0xd1 => {
                if stack.pop().is_none() {
                    return false;
                }
                stack.push(false);
            }
            0x28..=0x35 | 0xfd00 => {
                if stack.pop().is_none() {
                    return false;
                }
                stack.push(false);
            }
            0x36..=0x3e | 0xfd0b => {
                if stack.len() < 2 {
                    return false;
                }
                stack.truncate(stack.len() - 2);
            }
            0x3f => stack.push(false),
            0x40 => {
                if stack.pop().is_none() {
                    return false;
                }
                stack.push(false);
            }
            0x41..=0x44 | 0xfd0c => stack.push(false),
            0xfc0a | 0xfc0b => {
                if stack.len() < 3 {
                    return false;
                }
                stack.truncate(stack.len() - 3);
            }
            0xfd0f..=0xfd12 | 0xfd1d => {
                if stack.pop().is_none() {
                    return false;
                }
                stack.push(false);
            }
            0xfd1e => {
                if stack.len() < 2 {
                    return false;
                }
                stack.truncate(stack.len() - 2);
                stack.push(false);
            }
            _ => match numeric(op) {
                Some((arity, _, _)) => {
                    if stack.len() < arity {
                        return false;
                    }
                    stack.truncate(stack.len() - arity);
                    stack.push(false);
                }
                None => return false,
            },
        }
    }
    true
}

/// Precomputes for each `loop` instruction pc which locals are written inside
/// its body (invalidated on loop entry).
fn compute_loop_mod_locals(code: &[Instruction]) -> BTreeMap<usize, Vec<u32>> {
    let mut loop_mod_locals: BTreeMap<usize, Vec<u32>> = BTreeMap::new();
    let mut open_loops: Vec<(usize, usize)> = Vec::new();
    let mut depth = 0usize;
    for (pc, i) in code.iter().enumerate() {
        match i.opcode {
            0x02 | 0x04 => depth += 1,
            0x03 => {
                depth += 1;
                open_loops.push((pc, depth));
            }
            0x0b => {
                if open_loops.last().is_some_and(|&(_, d)| d == depth) {
                    open_loops.pop();
                }
                depth = depth.saturating_sub(1);
            }
            0x21 | 0x22 => {
                if let Immediate::Index(x) = i.immediate {
                    for &(lpc, _) in &open_loops {
                        let v = loop_mod_locals.entry(lpc).or_default();
                        if !v.contains(&x) {
                            v.push(x);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    loop_mod_locals
}

/// Computes a sound superset of all callees (direct and `call_indirect` targets)
/// that may execute on a path reaching a normal exit of `code`. Uses `BitSet`
/// so large functions with thousands of `if` blocks finish in microseconds.
fn compute_normal_exit_callees(
    code: &[Instruction],
    n_funcs: usize,
    ci_index: &BTreeMap<(u32, u32), usize>,
    ci_targets: &[Vec<(u32, usize)>],
    noreturn: &[bool],
) -> Vec<usize> {
    // Track per-loop calls so backedges are handled soundly in a single pass.
    let mut loop_calls: BTreeMap<usize, Vec<u32>> = BTreeMap::new();
    {
        let mut open_loops: Vec<(usize, usize)> = Vec::new();
        let mut depth = 0usize;
        let mut unreachable_stack = vec![false];
        let mut dead = 0usize;
        for (pc, i) in code.iter().enumerate() {
            let op = i.opcode;
            if *unreachable_stack.last().unwrap_or(&false) {
                match op {
                    0x02..=0x04 => {
                        dead += 1;
                        continue;
                    }
                    0x05 if dead > 0 => continue,
                    0x0b if dead > 0 => {
                        dead -= 1;
                        continue;
                    }
                    0x05 | 0x0b => {}
                    _ => continue,
                }
            }
            match op {
                0x02 | 0x04 => {
                    depth += 1;
                    unreachable_stack.push(false);
                }
                0x03 => {
                    depth += 1;
                    open_loops.push((pc, depth));
                    unreachable_stack.push(false);
                }
                0x05 => {
                    if let Some(u) = unreachable_stack.last_mut() {
                        *u = false;
                    }
                    dead = 0;
                }
                0x0b => {
                    if open_loops.last().is_some_and(|&(_, d)| d == depth) {
                        open_loops.pop();
                    }
                    depth = depth.saturating_sub(1);
                    unreachable_stack.pop();
                    if unreachable_stack.is_empty() {
                        break;
                    }
                }
                0x00 | 0x0c | 0x0e | 0x0f => {
                    if let Some(u) = unreachable_stack.last_mut() {
                        *u = true;
                    }
                }
                0x10 => {
                    if let Immediate::Index(f) = i.immediate {
                        if noreturn.get(f as usize).copied().unwrap_or(false) {
                            if let Some(u) = unreachable_stack.last_mut() {
                                *u = true;
                            }
                        } else {
                            for &(lpc, _) in &open_loops {
                                loop_calls.entry(lpc).or_default().push(f);
                            }
                        }
                    }
                }
                0x11 => {
                    if let Immediate::Pair(ti, tbl) = i.immediate
                        && let Some(&ci_id) = ci_index.get(&(tbl, ti))
                    {
                        for &(_, f) in &ci_targets[ci_id] {
                            if !noreturn.get(f).copied().unwrap_or(false) {
                                for &(lpc, _) in &open_loops {
                                    loop_calls.entry(lpc).or_default().push(f as u32);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    struct EFrame {
        kind: Kind,
        unreachable: bool,
        end_reached: bool,
        end_calls: BitSet,
        if_calls: BitSet,
        has_else: bool,
    }
    let merge = |f: &mut EFrame, calls: &BitSet| {
        if !f.end_reached {
            f.end_reached = true;
            f.end_calls = calls.clone();
        } else {
            f.end_calls.union_with(calls);
        }
    };

    let mut cur = BitSet::new(n_funcs);
    let mut exit_calls = BitSet::new(n_funcs);
    let mut dead = 0usize;
    let mut frames = vec![EFrame {
        kind: Kind::Function,
        unreachable: false,
        end_reached: false,
        end_calls: BitSet::default(),
        if_calls: BitSet::default(),
        has_else: false,
    }];

    for (pc, i) in code.iter().enumerate() {
        let op = i.opcode;
        if frames.last().is_some_and(|f| f.unreachable) {
            match op {
                0x02..=0x04 => {
                    dead += 1;
                    continue;
                }
                0x05 if dead > 0 => continue,
                0x0b if dead > 0 => {
                    dead -= 1;
                    continue;
                }
                0x05 | 0x0b => {}
                _ => continue,
            }
        }
        match op {
            0x00 => frames.last_mut().unwrap().unreachable = true,
            0x02 => frames.push(EFrame {
                kind: Kind::Block,
                unreachable: false,
                end_reached: false,
                end_calls: BitSet::default(),
                if_calls: BitSet::default(),
                has_else: false,
            }),
            0x03 => {
                if let Some(lc) = loop_calls.get(&pc) {
                    for &c in lc {
                        cur.insert(c);
                    }
                }
                frames.push(EFrame {
                    kind: Kind::Loop,
                    unreachable: false,
                    end_reached: false,
                    end_calls: BitSet::default(),
                    if_calls: BitSet::default(),
                    has_else: false,
                });
            }
            0x04 => frames.push(EFrame {
                kind: Kind::If,
                unreachable: false,
                end_reached: false,
                end_calls: BitSet::default(),
                if_calls: cur.clone(),
                has_else: false,
            }),
            0x05 => {
                let Some(f) = frames.last_mut() else { break };
                if !f.unreachable {
                    merge(f, &cur);
                }
                f.has_else = true;
                f.unreachable = false;
                cur.clone_from(&f.if_calls);
                dead = 0;
            }
            0x0b => {
                let Some(mut f) = frames.pop() else { break };
                if f.kind == Kind::Function {
                    if !f.unreachable {
                        exit_calls.union_with(&cur);
                    }
                    break;
                }
                if !f.unreachable {
                    merge(&mut f, &cur);
                }
                if f.kind == Kind::If && !f.has_else {
                    let ic = std::mem::take(&mut f.if_calls);
                    merge(&mut f, &ic);
                }
                if f.end_reached {
                    cur = f.end_calls;
                } else if let Some(parent) = frames.last_mut() {
                    parent.unreachable = true;
                }
            }
            0x0c => {
                if let Immediate::Index(d) = i.immediate
                    && let Some(at) = frames.len().checked_sub(d as usize + 1)
                {
                    match frames[at].kind {
                        Kind::Function => {
                            exit_calls.union_with(&cur);
                        }
                        Kind::Block | Kind::If => merge(&mut frames[at], &cur),
                        Kind::Loop => {}
                    }
                }
                frames.last_mut().unwrap().unreachable = true;
            }
            0x0d => {
                if let Immediate::Index(d) = i.immediate
                    && let Some(at) = frames.len().checked_sub(d as usize + 1)
                {
                    match frames[at].kind {
                        Kind::Function => {
                            exit_calls.union_with(&cur);
                        }
                        Kind::Block | Kind::If => merge(&mut frames[at], &cur),
                        Kind::Loop => {}
                    }
                }
            }
            0x0e => {
                if let Immediate::BrTable(ref targets, default) = i.immediate {
                    for &d in targets.iter().chain(std::iter::once(&default)) {
                        if let Some(at) = frames.len().checked_sub(d as usize + 1) {
                            match frames[at].kind {
                                Kind::Function => {
                                    exit_calls.union_with(&cur);
                                }
                                Kind::Block | Kind::If => merge(&mut frames[at], &cur),
                                Kind::Loop => {}
                            }
                        }
                    }
                }
                frames.last_mut().unwrap().unreachable = true;
            }
            0x0f => {
                exit_calls.union_with(&cur);
                frames.last_mut().unwrap().unreachable = true;
            }
            0x10 => {
                if let Immediate::Index(f) = i.immediate {
                    if noreturn.get(f as usize).copied().unwrap_or(false) {
                        frames.last_mut().unwrap().unreachable = true;
                    } else {
                        cur.insert(f);
                    }
                }
            }
            0x11 => {
                if let Immediate::Pair(ti, tbl) = i.immediate
                    && let Some(&ci_id) = ci_index.get(&(tbl, ti))
                {
                    for &(_, f) in &ci_targets[ci_id] {
                        if !noreturn.get(f).copied().unwrap_or(false) {
                            cur.insert(f as u32);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for (wi, &word) in exit_calls.words.iter().enumerate() {
        let mut w = word;
        while w != 0 {
            let tz = w.trailing_zeros() as usize;
            out.push(wi * 64 + tz);
            w &= w - 1;
        }
    }
    out
}

/// Sound structured-control-flow proof that function `code` preserves global `g`
/// on every reachable normal exit, given which callees preserve `g`. Uses a
/// monotonic fixed point for loop-header invariance of `g == entry_g`.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn function_preserves_global(
    code: &[Instruction],
    n_locals: usize,
    types: &[FuncType],
    module_types: &[FuncType],
    ci_index: &BTreeMap<(u32, u32), usize>,
    ci_targets: &[Vec<(u32, usize)>],
    noreturn: &[bool],
    callee_preserves: &[bool],
    g: u32,
) -> bool {
    let loop_mod_locals = compute_loop_mod_locals(code);
    function_preserves_global_with_loops(
        code,
        n_locals,
        types,
        module_types,
        ci_index,
        ci_targets,
        noreturn,
        callee_preserves,
        &loop_mod_locals,
        g,
    )
}

#[allow(clippy::too_many_arguments)]
fn function_preserves_global_with_loops(
    code: &[Instruction],
    n_locals: usize,
    types: &[FuncType],
    module_types: &[FuncType],
    ci_index: &BTreeMap<(u32, u32), usize>,
    ci_targets: &[Vec<(u32, usize)>],
    noreturn: &[bool],
    callee_preserves: &[bool],
    loop_mod_locals: &BTreeMap<usize, Vec<u32>>,
    g: u32,
) -> bool {
    let mut touches_g = false;
    visit_live(code, noreturn, |i| match (i.opcode, &i.immediate) {
        (0x24, Immediate::Index(x)) if *x == g => touches_g = true,
        (0x10, Immediate::Index(f))
            if !noreturn.get(*f as usize).copied().unwrap_or(false)
                && !callee_preserves.get(*f as usize).copied().unwrap_or(false) =>
        {
            touches_g = true;
        }
        (0x11, Immediate::Pair(ti, tbl)) => {
            let Some(&ci_id) = ci_index.get(&(*tbl, *ti)) else {
                touches_g = true;
                return;
            };
            if ci_targets[ci_id].iter().any(|&(_, f)| {
                !noreturn.get(f).copied().unwrap_or(false)
                    && !callee_preserves.get(f).copied().unwrap_or(false)
            }) {
                touches_g = true;
            }
        }
        _ => {}
    });
    if !touches_g {
        return true;
    }

    struct PFrame {
        kind: Kind,
        pc: usize,
        header_g: bool,
        height: usize,
        results: usize,
        unreachable: bool,
        end_reached: bool,
        end_g: bool,
        end_locals: BitSet,
        if_g: bool,
        if_locals: BitSet,
        has_else: bool,
    }

    let merge = |frame: &mut PFrame, g_entry: bool, locals: &BitSet| {
        if !frame.end_reached {
            frame.end_reached = true;
            frame.end_g = g_entry;
            frame.end_locals = locals.clone();
        } else {
            frame.end_g &= g_entry;
            frame.end_locals.intersect_with(locals);
        }
    };

    // Monotonic set of loop pcs whose backedges did NOT preserve `g == entry_g`.
    let mut loop_not_invariant = vec![false; code.len()];
    loop {
        let mut retry = false;
        let mut g_entry = true;
        let mut locals = BitSet::new(n_locals);
        let mut stack: Vec<bool> = Vec::new();
        let mut dead = 0usize;
        let mut frames = vec![PFrame {
            kind: Kind::Function,
            pc: 0,
            header_g: true,
            height: 0,
            results: 0,
            unreachable: false,
            end_reached: false,
            end_g: true,
            end_locals: BitSet::default(),
            if_g: false,
            if_locals: BitSet::default(),
            has_else: false,
        }];

        for (pc, i) in code.iter().enumerate() {
            let op = i.opcode;
            if frames.last().is_some_and(|f| f.unreachable) {
                match op {
                    0x02..=0x04 => {
                        dead += 1;
                        continue;
                    }
                    0x05 if dead > 0 => continue,
                    0x0b if dead > 0 => {
                        dead -= 1;
                        continue;
                    }
                    0x05 | 0x0b => {}
                    _ => continue,
                }
            }
            let block_results = |imm: &Immediate| -> Option<usize> {
                match imm {
                    Immediate::Block(-64) => Some(0),
                    Immediate::Block(-5..=-1 | -16 | -17) | Immediate::BlockType(_) => Some(1),
                    Immediate::Block(n) if *n >= 0 => {
                        let ft = module_types.get(*n as usize)?;
                        if ft.params.is_empty() {
                            Some(ft.results.len())
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            };
            match op {
                0x00 => frames.last_mut().unwrap().unreachable = true,
                0x01 => {}
                0x02 | 0x03 => {
                    let Some(results) = block_results(&i.immediate) else {
                        return false;
                    };
                    if op == 0x03 {
                        if loop_not_invariant[pc] {
                            g_entry = false;
                        }
                        if let Some(mods) = loop_mod_locals.get(&pc) {
                            for &x in mods {
                                locals.remove(x);
                            }
                        }
                    }
                    frames.push(PFrame {
                        kind: if op == 0x03 { Kind::Loop } else { Kind::Block },
                        pc,
                        header_g: g_entry,
                        height: stack.len(),
                        results,
                        unreachable: false,
                        end_reached: false,
                        end_g: true,
                        end_locals: BitSet::default(),
                        if_g: false,
                        if_locals: BitSet::default(),
                        has_else: false,
                    });
                }
                0x04 => {
                    let Some(results) = block_results(&i.immediate) else {
                        return false;
                    };
                    if stack.pop().is_none() {
                        return false;
                    }
                    frames.push(PFrame {
                        kind: Kind::If,
                        pc,
                        header_g: false,
                        height: stack.len(),
                        results,
                        unreachable: false,
                        end_reached: false,
                        end_g: true,
                        end_locals: BitSet::default(),
                        if_g: g_entry,
                        if_locals: locals.clone(),
                        has_else: false,
                    });
                }
                0x05 => {
                    let Some(frame) = frames.last_mut() else {
                        return false;
                    };
                    if !frame.unreachable {
                        merge(frame, g_entry, &locals);
                    }
                    frame.has_else = true;
                    frame.unreachable = false;
                    g_entry = frame.if_g;
                    locals.clone_from(&frame.if_locals);
                    stack.truncate(frame.height);
                    dead = 0;
                }
                0x0b => {
                    let Some(mut frame) = frames.pop() else {
                        return false;
                    };
                    if frame.kind == Kind::Function {
                        if !frame.unreachable && !g_entry {
                            return false;
                        }
                        break;
                    }
                    if !frame.unreachable {
                        merge(&mut frame, g_entry, &locals);
                    }
                    if frame.kind == Kind::If && !frame.has_else {
                        let if_g = frame.if_g;
                        let if_locals = std::mem::take(&mut frame.if_locals);
                        merge(&mut frame, if_g, &if_locals);
                    }
                    stack.truncate(frame.height);
                    if frame.end_reached {
                        g_entry = frame.end_g;
                        locals = frame.end_locals;
                        stack.resize(stack.len() + frame.results, false);
                    } else if let Some(parent) = frames.last_mut() {
                        parent.unreachable = true;
                    }
                }
                0x0c => {
                    let Immediate::Index(d) = i.immediate else {
                        return false;
                    };
                    let Some(at) = frames.len().checked_sub(d as usize + 1) else {
                        return false;
                    };
                    match frames[at].kind {
                        Kind::Function => {
                            if !g_entry {
                                return false;
                            }
                        }
                        Kind::Block | Kind::If => merge(&mut frames[at], g_entry, &locals),
                        Kind::Loop => {
                            if frames[at].header_g && !g_entry && !loop_not_invariant[frames[at].pc]
                            {
                                loop_not_invariant[frames[at].pc] = true;
                                retry = true;
                            }
                        }
                    }
                    frames.last_mut().unwrap().unreachable = true;
                }
                0x0d => {
                    let Immediate::Index(d) = i.immediate else {
                        return false;
                    };
                    if stack.pop().is_none() {
                        return false;
                    }
                    let Some(at) = frames.len().checked_sub(d as usize + 1) else {
                        return false;
                    };
                    match frames[at].kind {
                        Kind::Function => {
                            if !g_entry {
                                return false;
                            }
                        }
                        Kind::Block | Kind::If => merge(&mut frames[at], g_entry, &locals),
                        Kind::Loop => {
                            if frames[at].header_g && !g_entry && !loop_not_invariant[frames[at].pc]
                            {
                                loop_not_invariant[frames[at].pc] = true;
                                retry = true;
                            }
                        }
                    }
                }
                0x0e => {
                    let Immediate::BrTable(ref targets, default) = i.immediate else {
                        return false;
                    };
                    if stack.pop().is_none() {
                        return false;
                    }
                    for &d in targets.iter().chain(std::iter::once(&default)) {
                        let Some(at) = frames.len().checked_sub(d as usize + 1) else {
                            return false;
                        };
                        match frames[at].kind {
                            Kind::Function => {
                                if !g_entry {
                                    return false;
                                }
                            }
                            Kind::Block | Kind::If => merge(&mut frames[at], g_entry, &locals),
                            Kind::Loop => {
                                if frames[at].header_g
                                    && !g_entry
                                    && !loop_not_invariant[frames[at].pc]
                                {
                                    loop_not_invariant[frames[at].pc] = true;
                                    retry = true;
                                }
                            }
                        }
                    }
                    frames.last_mut().unwrap().unreachable = true;
                }
                0x0f => {
                    if !g_entry {
                        return false;
                    }
                    frames.last_mut().unwrap().unreachable = true;
                }
                0x10 => {
                    let Immediate::Index(f) = i.immediate else {
                        return false;
                    };
                    let Some(ty) = types.get(f as usize) else {
                        return false;
                    };
                    if stack.len() < ty.params.len() {
                        return false;
                    }
                    stack.truncate(stack.len() - ty.params.len());
                    if noreturn.get(f as usize).copied().unwrap_or(false) {
                        frames.last_mut().unwrap().unreachable = true;
                    } else {
                        if !callee_preserves.get(f as usize).copied().unwrap_or(false) {
                            g_entry = false;
                        }
                        stack.resize(stack.len() + ty.results.len(), false);
                    }
                }
                0x11 => {
                    let Immediate::Pair(ti, tbl) = i.immediate else {
                        return false;
                    };
                    let Some(ty) = module_types.get(ti as usize) else {
                        return false;
                    };
                    let Some(&ci_id) = ci_index.get(&(tbl, ti)) else {
                        return false;
                    };
                    if stack.len() < ty.params.len() + 1 {
                        return false;
                    }
                    stack.truncate(stack.len() - (ty.params.len() + 1));
                    if ci_targets[ci_id].iter().any(|&(_, f)| {
                        !noreturn.get(f).copied().unwrap_or(false)
                            && !callee_preserves.get(f).copied().unwrap_or(false)
                    }) {
                        g_entry = false;
                    }
                    stack.resize(stack.len() + ty.results.len(), false);
                }
                0x1a => {
                    if stack.pop().is_none() {
                        return false;
                    }
                }
                0x1b | 0x1c => {
                    if stack.len() < 3 {
                        return false;
                    }
                    let _c = stack.pop().unwrap();
                    let b = stack.pop().unwrap();
                    let a = stack.pop().unwrap();
                    stack.push(a && b);
                }
                0x20 => {
                    let Immediate::Index(x) = i.immediate else {
                        return false;
                    };
                    if (x as usize) >= n_locals {
                        return false;
                    }
                    stack.push(locals.contains(x));
                }
                0x21 => {
                    let Immediate::Index(x) = i.immediate else {
                        return false;
                    };
                    let Some(v) = stack.pop() else { return false };
                    if (x as usize) >= n_locals {
                        return false;
                    }
                    locals.set(x, v);
                }
                0x22 => {
                    let Immediate::Index(x) = i.immediate else {
                        return false;
                    };
                    let Some(&v) = stack.last() else { return false };
                    if (x as usize) >= n_locals {
                        return false;
                    }
                    locals.set(x, v);
                }
                0x23 => {
                    let Immediate::Index(x) = i.immediate else {
                        return false;
                    };
                    stack.push(x == g && g_entry);
                }
                0x24 => {
                    let Immediate::Index(x) = i.immediate else {
                        return false;
                    };
                    let Some(v) = stack.pop() else { return false };
                    if x == g {
                        g_entry = v;
                    }
                }
                0x25 => {
                    if stack.pop().is_none() {
                        return false;
                    }
                    stack.push(false);
                }
                0x26 => {
                    if stack.len() < 2 {
                        return false;
                    }
                    stack.truncate(stack.len() - 2);
                }
                0xd0 => stack.push(false),
                0xd1 => {
                    if stack.pop().is_none() {
                        return false;
                    }
                    stack.push(false);
                }
                0x28..=0x35 | 0xfd00 => {
                    if stack.pop().is_none() {
                        return false;
                    }
                    stack.push(false);
                }
                0x36..=0x3e | 0xfd0b => {
                    if stack.len() < 2 {
                        return false;
                    }
                    stack.truncate(stack.len() - 2);
                }
                0x3f => stack.push(false),
                0x40 => {
                    if stack.pop().is_none() {
                        return false;
                    }
                    stack.push(false);
                }
                0x41..=0x44 | 0xfd0c => stack.push(false),
                0xfc0a | 0xfc0b => {
                    if stack.len() < 3 {
                        return false;
                    }
                    stack.truncate(stack.len() - 3);
                }
                0xfd0f..=0xfd12 | 0xfd1d => {
                    if stack.pop().is_none() {
                        return false;
                    }
                    stack.push(false);
                }
                0xfd1e => {
                    if stack.len() < 2 {
                        return false;
                    }
                    stack.truncate(stack.len() - 2);
                    stack.push(false);
                }
                _ => match numeric(op) {
                    Some((arity, _, _)) => {
                        if stack.len() < arity {
                            return false;
                        }
                        stack.truncate(stack.len() - arity);
                        stack.push(false);
                    }
                    None => return false,
                },
            }
        }
        if !retry {
            return true;
        }
    }
}

/// Tarjan's strongly connected components over `0..n` in reverse topological
/// order (callees before callers).
fn tarjan_scc(edges: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let n = edges.len();
    let mut index = 0usize;
    let mut indices = vec![usize::MAX; n];
    let mut lowlink = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack = Vec::new();
    let mut sccs = Vec::new();

    // Iterative DFS so deep call graphs never overflow the generator's stack.
    for start in 0..n {
        if indices[start] != usize::MAX {
            continue;
        }
        let mut call_stack: Vec<(usize, usize)> = vec![(start, 0)];
        indices[start] = index;
        lowlink[start] = index;
        index += 1;
        stack.push(start);
        on_stack[start] = true;

        while let Some((v, next_e)) = call_stack.last_mut() {
            let v = *v;
            if *next_e < edges[v].len() {
                let w = edges[v][*next_e];
                *next_e += 1;
                if indices[w] == usize::MAX {
                    indices[w] = index;
                    lowlink[w] = index;
                    index += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    call_stack.push((w, 0));
                } else if on_stack[w] {
                    lowlink[v] = lowlink[v].min(indices[w]);
                }
            } else {
                call_stack.pop();
                if let Some(&(parent, _)) = call_stack.last() {
                    lowlink[parent] = lowlink[parent].min(lowlink[v]);
                }
                if lowlink[v] == indices[v] {
                    let mut scc = Vec::new();
                    while let Some(w) = stack.pop() {
                        on_stack[w] = false;
                        scc.push(w);
                        if w == v {
                            break;
                        }
                    }
                    sccs.push(scc);
                }
            }
        }
    }
    sccs
}

pub(crate) struct Module {
    pub(crate) target: Target,
    /// Every type declared in the module's type section.
    pub(crate) module_types: Vec<FuncType>,
    /// Every function's type, imports first.
    pub(crate) types: Vec<FuncType>,
    pub(crate) imports: Vec<(String, String)>,
    pub(crate) memory64: bool,
    pub(crate) has_memory: bool,
    /// Minimum initial size in bytes of memory 0 (guaranteed lower bound on `ml`).
    pub(crate) min_bytes: usize,
    /// Fixed capacity in bytes for 32-bit memory (`max_pages * 65536`, 0 if no memory).
    pub(crate) cap32: usize,
    /// Fixed capacity in bytes for flat 64-bit memory (`max_pages * 65536`).
    pub(crate) cap64: Option<usize>,
    /// (type, initial expression) of each global.
    pub(crate) globals: Vec<(ValType, String)>,
    /// Whether each global is declared mutable and can actually change value.
    pub(crate) globals_mutable: Vec<bool>,
    /// Effectively constant globals evaluated to compile-time constants.
    pub(crate) global_consts: Vec<Option<ConstVal>>,
    /// Module-wide preserved globals passed by value in registers.
    pub(crate) reg_globals: Vec<u32>,
    /// Static immutable funcref tables (table index -> length).
    pub(crate) static_func_tables: BTreeMap<u32, usize>,
    /// Fixed-size mutable externref tables stored in `Ctx` (table index -> length).
    pub(crate) mut_extern_tables: BTreeMap<u32, usize>,
    /// Distinct `(table_idx, type_idx)` pairs used by live `call_indirect`.
    pub(crate) ci_sigs: Vec<(u32, u32)>,
    pub(crate) ci_index: BTreeMap<(u32, u32), usize>,
    /// For each `ci_id`: `(slot_idx, func_idx)` matching that signature.
    pub(crate) ci_targets: Vec<Vec<(u32, usize)>>,
    pub(crate) ci_reads: Vec<BitSet>,
    pub(crate) ci_writes: Vec<BitSet>,
    pub(crate) ci_net_writes: Vec<BitSet>,
    pub(crate) ci_may_grow: Vec<bool>,
    pub(crate) ci_reg_globals: Vec<Vec<u32>>,
    /// Whether each import never returns (`goose.abort`, `goose.exit`, `proc_exit`).
    pub(crate) noreturn_imports: Vec<bool>,
    /// Whether each import can re-enter guest functions (`goose.reserve` or Blot arena hooks).
    pub(crate) reserve_imports: Vec<bool>,
    /// Transitive non-const global reads per function.
    pub(crate) fn_reads: Vec<BitSet>,
    /// Transitive non-const global writes per function (including temporary writes
    /// later restored before return, so callers know whether `ctx.g{x}` is touched).
    pub(crate) fn_writes: Vec<BitSet>,
    /// Transitive net writes per function (excluding globals the function is
    /// proven to preserve on every normal exit, so callers skip reloading them).
    pub(crate) fn_net_writes: Vec<BitSet>,
    /// Per-function bitset of globals proven preserved on every normal exit.
    pub(crate) fn_preserves: Vec<BitSet>,
    /// Whether each function (transitively) may execute `memory.grow`.
    pub(crate) fn_may_grow: Vec<bool>,
    /// Whether each function skips the native stack check (small acyclic subtree).
    pub(crate) fn_no_stack_check: Vec<bool>,
    /// Whether each function is part of a recursive SCC.
    pub(crate) fn_in_cycle: Vec<bool>,
    /// Whether each function makes any direct or indirect non-noreturn calls.
    pub(crate) fn_has_calls: Vec<bool>,
    /// Whether each function contains any live `loop` instruction.
    pub(crate) fn_has_loops: Vec<bool>,
    /// Call sites targeting each function (direct + indirect weighted).
    pub(crate) fn_callers: Vec<usize>,
    /// Direct `call` sites targeting each function (excluding `call_indirect` tables).
    pub(crate) fn_direct_callers: Vec<usize>,
    /// Instruction count of each defined function (0 for imports).
    pub(crate) fn_insns: Vec<usize>,
    pub(crate) start: Option<u32>,
    pub(crate) entry: Option<u32>,
    pub(crate) reserve: Option<u32>,
    pub(crate) exported_funcs: BTreeSet<usize>,
    pub(crate) exported_globals: Vec<(String, usize, ValType, bool)>,
    pub(crate) memory_exported: bool,
    /// The 64-bit memory is flat (see `generate`).
    pub(crate) flat64: bool,
    /// The bytes of a 32-bit memory whose minimum and maximum are equal.
    pub(crate) fixed32: Option<u64>,
    /// The bytes of a flat 64-bit memory whose minimum and maximum are equal.
    pub(crate) fixed64: Option<u64>,
}

impl Module {
    pub(crate) fn new(m: &DecodedModule, flat64_req: bool, target: Target) -> Res<Module> {
        let module_types = m.types.clone();
        let mut types = Vec::new();
        let mut imports = Vec::new();
        for import in &m.imports {
            match &import.kind {
                ImportKind::Func(t) | ImportKind::ExactFunc(t) => {
                    types.push(m.types[t.0 as usize].clone());
                    imports.push((import.module.clone(), import.name.clone()));
                }
                _ => {
                    return Err(format!(
                        "import {:?}.{:?} is not a function",
                        import.module, import.name
                    ));
                }
            }
        }
        for t in &m.functions {
            types.push(m.types[t.0 as usize].clone());
        }
        for t in &types {
            for v in t.params.iter().chain(&t.results) {
                rust_type(*v)?;
            }
        }
        if m.memories.len() > 1 {
            return Err("more than one memory is not supported".into());
        }
        let memory64 = m
            .memories
            .first()
            .is_some_and(|mt| mt.index_type == wtime_types::IndexType::I64);
        if target == Target::BlotCdylib && memory64 {
            return Err("Blot cdylib target does not support memory64".into());
        }
        if m.memories
            .first()
            .is_some_and(|mt| mt.page_size_log2 != 16 || mt.shared)
        {
            return Err("custom page sizes and shared memories are not supported".into());
        }
        let flat64 = flat64_req && memory64;
        let min_bytes = m
            .memories
            .first()
            .and_then(|mt| mt.limits.min.checked_mul(65536))
            .and_then(|b| usize::try_from(b).ok())
            .unwrap_or(0);
        let cap32 = match m.memories.first() {
            Some(mt) if !memory64 => (mt.limits.max.unwrap_or(1 << 16).min(1 << 16) << 16) as usize,
            _ => 0,
        };
        let cap64 = if flat64 {
            m.memories.first().and_then(|mt| {
                let max_pages = mt.limits.max.unwrap_or(1 << 20).min(1 << 20);
                max_pages
                    .checked_mul(65536)
                    .and_then(|b| usize::try_from(b).ok())
            })
        } else {
            None
        };
        let fixed32 = m
            .memories
            .first()
            .filter(|mt| !memory64 && mt.limits.max == Some(mt.limits.min))
            .map(|mt| mt.limits.min << 16);
        let fixed64 = m
            .memories
            .first()
            .filter(|mt| flat64 && mt.limits.max == Some(mt.limits.min))
            .and_then(|mt| mt.limits.min.checked_mul(65536));

        let n_funcs = types.len();
        let n_imports = imports.len();
        let n_globals = m.globals.len();
        let mut noreturn_imports = vec![false; n_funcs];
        let mut reserve_imports = vec![false; n_funcs];
        for (k, (module, name)) in imports.iter().enumerate() {
            if matches!(
                (module.as_str(), name.as_str()),
                ("goose", "abort") | ("goose", "exit") | ("wasi_snapshot_preview1", "proc_exit")
            ) {
                noreturn_imports[k] = true;
            }
            if matches!(
                (module.as_str(), name.as_str()),
                ("env", "gs_os_read_file") | ("env", "gs_os_arg") | ("env", "gs_os_getenv")
            ) || module == "blot:host/1"
            {
                reserve_imports[k] = true;
            }
        }

        // Exported functions and globals (for root preservation and Blot cdylib).
        let mut exported_funcs = BTreeSet::new();
        let mut exported_globals = Vec::new();
        let mut exported_mut_globals = BTreeSet::new();
        for exp in &m.exports {
            match exp.kind {
                ExternalKind::Func => {
                    exported_funcs.insert(exp.index as usize);
                }
                ExternalKind::Global => {
                    let gi = exp.index as usize;
                    if let Some(g) = m.globals.get(gi) {
                        exported_globals.push((exp.name.clone(), gi, g.ty.value, g.ty.mutable));
                        if g.ty.mutable {
                            exported_mut_globals.insert(gi);
                        }
                    }
                }
                _ => {}
            }
        }

        // Scan live instructions for global writes, table ops, and call_indirect sites.
        let mut global_written = vec![false; n_globals];
        let mut direct_global_reads: Vec<BitSet> =
            (0..m.code.len()).map(|_| BitSet::new(n_globals)).collect();
        let mut direct_global_writes: Vec<BitSet> =
            (0..m.code.len()).map(|_| BitSet::new(n_globals)).collect();
        let mut mutated_tables = BTreeSet::new();
        let mut read_tables = BTreeSet::new();
        let mut ci_set = BTreeSet::new();
        for (fi, body) in m.code.iter().enumerate() {
            visit_live(&body.instructions, &noreturn_imports, |i| {
                match (i.opcode, &i.immediate) {
                    (0x23, Immediate::Index(x)) => {
                        if (*x as usize) < n_globals {
                            direct_global_reads[fi].insert(*x);
                        }
                    }
                    (0x24, Immediate::Index(x)) => {
                        if let Some(w) = global_written.get_mut(*x as usize) {
                            *w = true;
                            direct_global_writes[fi].insert(*x);
                        }
                    }
                    (0x11, Immediate::Pair(ti, tbl)) => {
                        ci_set.insert((*tbl, *ti));
                    }
                    (0x25, Immediate::Index(tbl)) => {
                        read_tables.insert(*tbl);
                    }
                    (0x26, Immediate::Index(tbl)) => {
                        mutated_tables.insert(*tbl);
                    }
                    (0xfc0c, Immediate::Pair(_, tbl))
                    | (0xfc0e, Immediate::Pair(tbl, _))
                    | (0xfc0f | 0xfc11, Immediate::Index(tbl)) => {
                        mutated_tables.insert(*tbl);
                    }
                    _ => {}
                }
            });
        }

        // Evaluate effectively constant globals: unwritten globals AND globals
        // whose every reachable `global.set` writes back their own value.
        // In Blot cdylib mode, exported mutable globals stay mutable in `Ctx`
        // so `Guest::restore` can update them.
        let mut global_consts: Vec<Option<ConstVal>> = vec![None; n_globals];
        for (k, g) in m.globals.iter().enumerate() {
            if target == Target::BlotCdylib && exported_mut_globals.contains(&k) {
                continue;
            }
            let Some(init_val) = eval_constant(&g.init, &global_consts) else {
                continue;
            };
            if !global_written[k] {
                global_consts[k] = Some(init_val);
            } else {
                let only_same = m.code.iter().enumerate().all(|(fi, body)| {
                    if !direct_global_writes[fi].contains(k as u32) {
                        return true;
                    }
                    if !direct_global_reads[fi].contains(k as u32) {
                        return false;
                    }
                    let n_locals = types[n_imports + fi].params.len() + body.locals.len();
                    function_writes_only_same_global(
                        &body.instructions,
                        n_locals,
                        &types,
                        &module_types,
                        &noreturn_imports,
                        k as u32,
                    )
                });
                if only_same {
                    global_consts[k] = Some(init_val);
                }
            }
        }
        let mut globals = Vec::new();
        let mut globals_mutable = Vec::new();
        for (k, g) in m.globals.iter().enumerate() {
            let init =
                constant(&g.init, k, "", &global_consts).map_err(|e| format!("global {k}: {e}"))?;
            globals.push((g.ty.value, init));
            globals_mutable.push(g.ty.mutable && global_consts[k].is_none());
            rust_type(g.ty.value)?;
        }

        // Classify tables and resolve immutable funcref tables from active element segments.
        let mut static_func_tables = BTreeMap::new();
        let mut static_func_slots: BTreeMap<u32, Vec<Option<u32>>> = BTreeMap::new();
        let mut mut_extern_tables = BTreeMap::new();
        for (ti, tbl) in m.tables.iter().enumerate() {
            let ti_u32 = ti as u32;
            let used_by_ci = ci_set.iter().any(|&(t, _)| t == ti_u32);
            let used_by_rw = read_tables.contains(&ti_u32) || mutated_tables.contains(&ti_u32);
            if !used_by_ci && !used_by_rw {
                continue;
            }
            if tbl.index_type != wtime_types::IndexType::I32 {
                return Err(format!("table {ti}: 64-bit tables are not supported"));
            }
            match tbl.element.heap {
                HeapType::Func
                    if !mutated_tables.contains(&ti_u32) && !read_tables.contains(&ti_u32) =>
                {
                    let len = usize::try_from(tbl.limits.min)
                        .map_err(|_| format!("table {ti} too large"))?;
                    static_func_tables.insert(ti_u32, len);
                    static_func_slots.insert(ti_u32, vec![None; len]);
                }
                HeapType::Extern | HeapType::NoExtern if !used_by_ci => {
                    if tbl.limits.max != Some(tbl.limits.min) || tbl.limits.min > 4096 {
                        return Err(format!(
                            "table {ti}: only fixed-size externref tables up to 4096 entries are supported"
                        ));
                    }
                    mut_extern_tables.insert(ti_u32, tbl.limits.min as usize);
                }
                _ => return Err(format!("table {ti}: unsupported table type or mutation")),
            }
        }
        for elem in &m.elements {
            if let SegmentMode::Active { index, offset } = &elem.mode
                && let Some(slots) = static_func_slots.get_mut(index)
            {
                let Some(ConstVal::I32(off)) = eval_constant(offset, &global_consts) else {
                    return Err(format!(
                        "element segment for table {index} has non-constant offset"
                    ));
                };
                let off = off as u32 as usize;
                if off
                    .checked_add(elem.init.len())
                    .is_none_or(|end| end > slots.len())
                {
                    // Runtime check in `initialize` will trap with TableOutOfBounds; skip static fill.
                    continue;
                }
                for (k, item) in elem.init.iter().enumerate() {
                    match item.as_slice() {
                        [i, end] if i.opcode == 0xd2 && end.opcode == 0x0b => {
                            if let Immediate::Index(f) = i.immediate {
                                slots[off + k] = Some(f);
                            }
                        }
                        [i, end] if i.opcode == 0xd0 && end.opcode == 0x0b => {
                            slots[off + k] = None;
                        }
                        _ => {
                            return Err(format!(
                                "element segment for table {index} has unsupported item"
                            ));
                        }
                    }
                }
            }
        }
        let mut ci_sigs = Vec::new();
        let mut ci_index = BTreeMap::new();
        let mut ci_targets = Vec::new();
        for (tbl, ti) in ci_set {
            let slots = static_func_slots
                .get(&tbl)
                .ok_or_else(|| format!("call_indirect on unsupported table {tbl}"))?;
            let want = module_types
                .get(ti as usize)
                .ok_or_else(|| format!("call_indirect bad type {ti}"))?;
            let id = ci_sigs.len();
            ci_sigs.push((tbl, ti));
            ci_index.insert((tbl, ti), id);
            let mut targets = Vec::new();
            for (slot, func) in slots.iter().enumerate() {
                if let Some(f) = *func
                    && types.get(f as usize) == Some(want)
                {
                    targets.push((slot as u32, f as usize));
                }
            }
            ci_targets.push(targets);
        }

        let export = |name: &str| {
            m.exports
                .iter()
                .find(|e| e.name == name && e.kind == ExternalKind::Func)
                .map(|e| e.index)
        };
        let start = m.start.map(|f| f.0);
        let entry = export("_start");
        let reserve = export("goose.reserve");

        // Build call graph and per-function direct effects.
        let mut edges: Vec<Vec<usize>> = vec![Vec::new(); n_funcs];
        let mut fn_reads: Vec<BitSet> = (0..n_funcs).map(|_| BitSet::new(n_globals)).collect();
        let mut fn_writes: Vec<BitSet> = (0..n_funcs).map(|_| BitSet::new(n_globals)).collect();
        let mut fn_may_grow = vec![false; n_funcs];
        let mut fn_has_calls = vec![false; n_funcs];
        let mut fn_has_loops = vec![false; n_funcs];
        let mut fn_callers = vec![0usize; n_funcs];
        let mut fn_direct_callers = vec![0usize; n_funcs];
        let mut fn_insns = vec![0usize; n_funcs];
        for (k, body) in m.code.iter().enumerate() {
            fn_insns[n_imports + k] = body.instructions.len();
        }
        let fixed_mem = fixed32.is_some() || fixed64.is_some();

        for (k, (module, _)) in imports.iter().enumerate() {
            if reserve_imports[k] {
                if module == "blot:host/1" {
                    for hook in [
                        "blot:allocate",
                        "blot:reset",
                        "blot:finish_invocation",
                        "blot:ownership_enter",
                        "blot:ownership_leave",
                    ] {
                        if let Some(h) = export(hook) {
                            let h = h as usize;
                            if !edges[k].contains(&h) {
                                edges[k].push(h);
                            }
                        }
                    }
                } else if let Some(r) = reserve {
                    edges[k].push(r as usize);
                }
            }
        }
        let mut global_use_fns = vec![0usize; n_globals];
        for (k, body) in m.code.iter().enumerate() {
            let f = n_imports + k;
            let mut seen_g = BitSet::new(n_globals);
            visit_live(&body.instructions, &noreturn_imports, |i| {
                match (i.opcode, &i.immediate) {
                    (0x03, _) => {
                        fn_has_loops[f] = true;
                    }
                    (0x10, Immediate::Index(callee)) => {
                        let c = *callee as usize;
                        if c < n_funcs && !noreturn_imports[c] {
                            fn_has_calls[f] = true;
                            fn_callers[c] += 1;
                            fn_direct_callers[c] += 1;
                            if !edges[f].contains(&c) {
                                edges[f].push(c);
                            }
                        }
                    }
                    (0x11, Immediate::Pair(ti, tbl)) => {
                        fn_has_calls[f] = true;
                        if let Some(&ci_id) = ci_index.get(&(*tbl, *ti)) {
                            for &(_, c) in &ci_targets[ci_id] {
                                if c < n_funcs && !noreturn_imports[c] {
                                    fn_callers[c] += 2;
                                    if !edges[f].contains(&c) {
                                        edges[f].push(c);
                                    }
                                }
                            }
                        }
                    }
                    (0x23, Immediate::Index(x))
                        if (*x as usize) < n_globals && global_consts[*x as usize].is_none() =>
                    {
                        fn_reads[f].insert(*x);
                        seen_g.insert(*x);
                    }
                    (0x24, Immediate::Index(x))
                        if (*x as usize) < n_globals && global_consts[*x as usize].is_none() =>
                    {
                        fn_writes[f].insert(*x);
                        seen_g.insert(*x);
                    }
                    (0x40, _) if !fixed_mem => {
                        fn_may_grow[f] = true;
                    }
                    _ => {}
                }
            });
            for (g, count) in global_use_fns.iter_mut().enumerate() {
                if seen_g.contains(g as u32) {
                    *count += 1;
                }
            }
        }

        // Fixed-point propagation of transitive reads, writes and memory growth.
        let sccs = tarjan_scc(&edges);
        let mut changed = true;
        while changed {
            changed = false;
            for scc in &sccs {
                for &u in scc {
                    for &v in &edges[u] {
                        let v_reads = fn_reads[v].clone();
                        let v_writes = fn_writes[v].clone();
                        if fn_reads[u].union_with(&v_reads) {
                            changed = true;
                        }
                        if fn_writes[u].union_with(&v_writes) {
                            changed = true;
                        }
                        if fn_may_grow[v] && !fn_may_grow[u] {
                            fn_may_grow[u] = true;
                            changed = true;
                        }
                    }
                }
            }
        }

        // Detect recursive SCCs and small acyclic subtrees that skip stack checks.
        let mut fn_in_cycle = vec![false; n_funcs];
        let mut fn_no_stack_check = vec![false; n_funcs];
        let mut chain_locals = vec![0usize; n_funcs];
        let mut chain_insns = vec![0usize; n_funcs];
        let mut chain_depth = vec![0usize; n_funcs];
        for scc in &sccs {
            let cyclic = scc.len() > 1 || (scc.len() == 1 && edges[scc[0]].contains(&scc[0]));
            if cyclic {
                for &u in scc {
                    fn_in_cycle[u] = true;
                }
                continue;
            }
            let u = scc[0];
            if u < n_imports {
                let mut ok = true;
                let mut max_l = 0usize;
                let mut max_i = 0usize;
                let mut max_d = 0usize;
                for &v in &edges[u] {
                    if !fn_no_stack_check[v] {
                        ok = false;
                        break;
                    }
                    max_l = max_l.max(chain_locals[v]);
                    max_i = max_i.max(chain_insns[v]);
                    max_d = max_d.max(chain_depth[v]);
                }
                if ok {
                    fn_no_stack_check[u] = true;
                    chain_locals[u] = max_l;
                    chain_insns[u] = max_i;
                    chain_depth[u] = max_d;
                }
                continue;
            }
            let body = &m.code[u - n_imports];
            let own_locals = types[u].params.len() + body.locals.len();
            let own_insns = body.instructions.len();
            let mut ok = true;
            let mut max_l = 0usize;
            let mut max_i = 0usize;
            let mut max_d = 0usize;
            for &v in &edges[u] {
                if !fn_no_stack_check[v] {
                    ok = false;
                    break;
                }
                max_l = max_l.max(chain_locals[v]);
                max_i = max_i.max(chain_insns[v]);
                max_d = max_d.max(chain_depth[v]);
            }
            let total_l = own_locals.saturating_add(max_l);
            let total_i = own_insns.saturating_add(max_i);
            let total_d = 1usize.saturating_add(max_d);
            let leaf_locals_cap = if target == Target::BlotCdylib {
                64
            } else {
                LEAF_LOCALS
            };
            if ok
                && total_l <= leaf_locals_cap
                && total_i <= LEAF_INSTRUCTIONS
                && total_d <= LEAF_CHAIN_DEPTH
            {
                fn_no_stack_check[u] = true;
                chain_locals[u] = total_l;
                chain_insns[u] = total_i;
                chain_depth[u] = total_d;
            }
        }

        // Greatest fixed-point preservation analysis for every non-const mutable
        // global `g`: finds both module-wide preserved globals (promoted to
        // by-value register arguments, like `g0` and `g3`) AND per-function
        // preserved globals (like `g1` in non-allocating functions, so callers
        // skip reloading `ctx.g1` after calling them).
        let mut fn_preserves: Vec<BitSet> = (0..n_funcs).map(|_| BitSet::new(n_globals)).collect();
        let mut fn_net_writes = fn_writes.clone();
        let mut candidates: Vec<(usize, u32)> = (0..n_globals)
            .filter(|&k| global_consts[k].is_none() && global_use_fns[k] > 0)
            .map(|k| (global_use_fns[k], k as u32))
            .collect();
        candidates.sort_by_key(|&(n, k)| (std::cmp::Reverse(n), k));
        let mut reg_globals = Vec::new();
        let mut loop_mod_cache: Vec<Option<BTreeMap<usize, Vec<u32>>>> = vec![None; m.code.len()];
        let mut normal_exit_cache: Vec<Option<Vec<usize>>> = vec![None; m.code.len()];
        for (_, g) in candidates {
            let mut preserves = vec![true; n_funcs];
            // Only functions whose transitive call subgraph writes `g` can fail
            // to preserve `g`. Iterate in bottom-up SCC order so acyclic call
            // chains converge in a single pass.
            let writers: Vec<usize> = sccs
                .iter()
                .flat_map(|scc| scc.iter().copied())
                .filter(|&f| fn_writes[f].contains(g))
                .collect();
            let mut p_changed = true;
            while p_changed {
                p_changed = false;
                for &f in &writers {
                    if !preserves[f] {
                        continue;
                    }
                    if f < n_imports {
                        if edges[f].iter().any(|&v| !preserves[v]) {
                            preserves[f] = false;
                            p_changed = true;
                        }
                        continue;
                    }
                    let fi = f - n_imports;
                    let writes_here = direct_global_writes[fi].contains(g);
                    let callees_ok = edges[f].iter().all(|&v| preserves[v]);
                    if !writes_here && callees_ok {
                        continue;
                    }
                    // A function that writes `g` without ever reading `g` cannot
                    // save-and-restore `g`'s entry value; conservatively mark
                    // non-preserving in O(1) without running full dataflow.
                    if writes_here && !direct_global_reads[fi].contains(g) {
                        preserves[f] = false;
                        p_changed = true;
                        continue;
                    }
                    let body = &m.code[fi];
                    if !writes_here {
                        let exit_callees = normal_exit_cache[fi].get_or_insert_with(|| {
                            compute_normal_exit_callees(
                                &body.instructions,
                                n_funcs,
                                &ci_index,
                                &ci_targets,
                                &noreturn_imports,
                            )
                        });
                        if exit_callees.iter().any(|&v| !preserves[v]) {
                            preserves[f] = false;
                            p_changed = true;
                        }
                        continue;
                    }
                    let loop_mods = loop_mod_cache[fi]
                        .get_or_insert_with(|| compute_loop_mod_locals(&body.instructions));
                    let n_locals = types[f].params.len() + body.locals.len();
                    if !function_preserves_global_with_loops(
                        &body.instructions,
                        n_locals,
                        &types,
                        &module_types,
                        &ci_index,
                        &ci_targets,
                        &noreturn_imports,
                        &preserves,
                        loop_mods,
                        g,
                    ) {
                        preserves[f] = false;
                        p_changed = true;
                    }
                }
            }
            let all_preserve = preserves.iter().all(|&p| p);
            for f in 0..n_funcs {
                if preserves[f] {
                    fn_preserves[f].insert(g);
                    fn_net_writes[f].remove(g);
                }
            }
            if all_preserve
                && reg_globals.len() < MAX_REG_GLOBALS
                && !(target == Target::BlotCdylib && exported_mut_globals.contains(&(g as usize)))
            {
                reg_globals.push(g);
            }
        }
        reg_globals.sort_unstable();

        // Precompute per-call_indirect-signature aggregate effects.
        let n_ci = ci_sigs.len();
        let mut ci_reads = vec![BitSet::new(n_globals); n_ci];
        let mut ci_writes = vec![BitSet::new(n_globals); n_ci];
        let mut ci_net_writes = vec![BitSet::new(n_globals); n_ci];
        let mut ci_may_grow = vec![false; n_ci];
        let mut ci_reg_globals = vec![Vec::new(); n_ci];
        for ci_id in 0..n_ci {
            for &(_, f) in &ci_targets[ci_id] {
                ci_reads[ci_id].union_with(&fn_reads[f]);
                ci_writes[ci_id].union_with(&fn_writes[f]);
                ci_net_writes[ci_id].union_with(&fn_net_writes[f]);
                ci_may_grow[ci_id] |= fn_may_grow[f];
            }
            ci_reg_globals[ci_id] = reg_globals
                .iter()
                .copied()
                .filter(|&g| ci_reads[ci_id].contains(g) || ci_writes[ci_id].contains(g))
                .collect();
        }

        Ok(Module {
            target,
            module_types,
            types,
            imports,
            memory64,
            has_memory: !m.memories.is_empty(),
            min_bytes,
            cap32,
            cap64,
            globals,
            globals_mutable,
            global_consts,
            reg_globals,
            static_func_tables,
            mut_extern_tables,
            ci_sigs,
            ci_index,
            ci_targets,
            ci_reads,
            ci_writes,
            ci_net_writes,
            ci_may_grow,
            ci_reg_globals,
            noreturn_imports,
            reserve_imports,
            fn_reads,
            fn_writes,
            fn_net_writes,
            fn_preserves,
            fn_may_grow,
            fn_no_stack_check,
            fn_in_cycle,
            fn_has_calls,
            fn_has_loops,
            fn_callers,
            fn_direct_callers,
            fn_insns,
            start,
            entry,
            reserve,
            exported_funcs,
            exported_globals,
            // The host uses the memory exported as `memory`, as wtime-goose does.
            memory_exported: m
                .exports
                .iter()
                .any(|e| e.kind == ExternalKind::Memory && e.name == "memory"),
            flat64,
            fixed32,
            fixed64,
        })
    }

    /// The Rust type of the disjoint memory slice passed to every function.
    fn mb_type(&self) -> &'static str {
        if !self.memory64 {
            if self.target == Target::BlotCdylib {
                "&mut [u8]"
            } else {
                "&mut [u8; CAP32]"
            }
        } else if self.cap64.is_some() {
            "&mut [u8; CAP64]"
        } else {
            "&mut [u8]"
        }
    }

    /// Whether defined function `f` is a tiny non-recursive leaf that should
    /// always inline (including through its `Fns<Marker>` trait bridge).
    /// Excludes functions with net global writes (such as bump allocators).
    pub(crate) fn fn_inline_always(&self, f: usize) -> bool {
        let is_root = self.start == Some(f as u32)
            || self.entry == Some(f as u32)
            || self.reserve == Some(f as u32)
            || (self.target == Target::BlotCdylib && self.exported_funcs.contains(&f));
        f >= self.imports.len()
            && !is_root
            && !self.fn_in_cycle[f]
            && !self.fn_has_calls[f]
            && self.fn_net_writes[f].is_empty()
            && (self.fn_insns[f] <= INLINE_ALWAYS_LEAF_INSTRUCTIONS
                || (!self.fn_has_loops[f]
                    && self.fn_insns[f] <= INLINE_ALWAYS_SMALL_LEAF_INSTRUCTIONS
                    && self.fn_callers[f] <= INLINE_ALWAYS_SMALL_LEAF_MAX_CALLERS))
    }

    /// Signature of `call_indirect` dispatcher `ci_{ci_id}` on `Fns<M>`.
    fn ci_signature(&self, ci_id: usize) -> Res<String> {
        let (_, ti) = self.ci_sigs[ci_id];
        let ty = &self.module_types[ti as usize];
        let mut params = vec!["&mut self".to_string(), format!("mb: {}", self.mb_type())];
        for &g in &self.ci_reg_globals[ci_id] {
            params.push(format!("gv{g}: {}", rust_type(self.globals[g as usize].0)?));
        }
        params.push("idx: i32".to_string());
        for (i, t) in ty.params.iter().enumerate() {
            params.push(format!("p{i}: {}", rust_type(*t)?));
        }
        Ok(format!(
            "({}) -> {}",
            params.join(", "),
            fn_return_type(&ty.results)?
        ))
    }

    /// Preserved register-passed globals needed by defined function `f`.
    pub(crate) fn fn_reg_globals(&self, f: usize) -> Vec<u32> {
        if f < self.imports.len() {
            return Vec::new();
        }
        self.reg_globals
            .iter()
            .copied()
            .filter(|&g| self.fn_reads[f].contains(g) || self.fn_writes[f].contains(g))
            .collect()
    }

    /// The signature of defined function `f` after its name:
    /// `(ctx, mb, [reg_globals...], params...) -> <RetAbi>`.
    fn fn_signature(&self, f: usize, ctx_param: &str) -> Res<String> {
        let ty = &self.types[f];
        let mut params = vec![ctx_param.to_string(), format!("mb: {}", self.mb_type())];
        for g in self.fn_reg_globals(f) {
            params.push(format!("gv{g}: {}", rust_type(self.globals[g as usize].0)?));
        }
        for (i, t) in ty.params.iter().enumerate() {
            params.push(format!("p{i}: {}", rust_type(*t)?));
        }
        Ok(format!(
            "({}) -> {}",
            params.join(", "),
            fn_return_type(&ty.results)?
        ))
    }

    /// The signature of host import `k` on `Ctx`.
    fn import_signature(&self, k: usize) -> Res<String> {
        let ty = &self.types[k];
        let mut params = vec!["&mut self".to_string(), format!("mb: {}", self.mb_type())];
        for (i, t) in ty.params.iter().enumerate() {
            params.push(format!("p{i}: {}", rust_type(*t)?));
        }
        if self.noreturn_imports[k] {
            Ok(format!("({}) -> Trap", params.join(", ")))
        } else {
            Ok(format!(
                "({}) -> R<{}>",
                params.join(", "),
                result_type(&ty.results)?
            ))
        }
    }

    /// The core crate: `Ctx` (hot fields first, disjoint from linear memory),
    /// `Instance`, `HostView`, the host imports, and `Fns<M>`.
    fn core(&self, m: &DecodedModule) -> Res<String> {
        let mut s = String::new();
        s.push_str("// Generated by wtime-aot. Do not edit.\n#![forbid(unsafe_code)]\n#![allow(unused, non_snake_case, unreachable_code, unused_parens, clippy::all)]\n\n");
        s.push_str("pub use wtime_aot_rt::*;\n\n");
        if !self.memory64 {
            writeln!(s, "pub const CAP32: usize = {};", self.cap32).unwrap();
        } else if let Some(cap64) = self.cap64 {
            writeln!(s, "pub const CAP64: usize = {cap64};").unwrap();
        }
        writeln!(s, "pub const MIN_BYTES: usize = {};\n", self.min_bytes).unwrap();
        // `#[repr(C)]` keeps stack_floor, memory length and mutable globals in
        // the first cache line with 1-byte displacements (instead of Rust
        // reordering `goose::State` ahead of them).
        s.push_str("#[repr(C)]\npub struct Ctx {\n");
        s.push_str("    /// The lowest stack address guest code may use (`goose::stack_floor`).\n    pub stack_floor: usize,\n");
        if self.memory64 {
            s.push_str("    pub outer: Outer,\n");
        } else {
            s.push_str("    pub mem: Mem32Meta,\n");
        }
        for (k, (t, _)) in self.globals.iter().enumerate() {
            if self.global_consts[k].is_none() {
                writeln!(s, "    pub g{k}: {},", rust_type(*t)?).unwrap();
            }
        }
        for &tbl in self.mut_extern_tables.keys() {
            writeln!(s, "    pub t{tbl}: Box<[u64]>,").unwrap();
        }
        if self.target == Target::BlotCdylib {
            s.push_str("    pub host_ctx: usize,\n");
            s.push_str("    pub host_mxcsr: u32,\n");
            writeln!(
                s,
                "    pub host_call: fn(&mut Ctx, {}, u32, u64, u64) -> R<u64>,\n}}\n",
                self.mb_type()
            )
            .unwrap();
            s.push_str("impl Ctx {\n");
            for (k, (module, name)) in self.imports.iter().enumerate() {
                let ty = &self.types[k];
                let (tok_expr, arg_expr) = match ty.params.as_slice() {
                    [] => ("0u64".to_string(), "0u64".to_string()),
                    [ValType::Ref(_)] => ("p0".to_string(), "0u64".to_string()),
                    [ValType::I32] => ("0u64".to_string(), "(p0 as u32 as u64)".to_string()),
                    [ValType::F32] => ("0u64".to_string(), "(p0.to_bits() as u64)".to_string()),
                    [ValType::Ref(_), ValType::I32] => {
                        ("p0".to_string(), "(p1 as u32 as u64)".to_string())
                    }
                    [ValType::Ref(_), ValType::F32] => {
                        ("p0".to_string(), "(p1.to_bits() as u64)".to_string())
                    }
                    [ValType::Ref(_), ValType::I64] => {
                        ("p0".to_string(), "(p1 as u64)".to_string())
                    }
                    [ValType::Ref(_), ValType::F64] => {
                        ("p0".to_string(), "p1.to_bits()".to_string())
                    }
                    [ValType::Ref(_), ValType::Ref(_)] => ("p0".to_string(), "p1".to_string()),
                    _ => {
                        return Err(format!(
                            "Blot import {module:?}.{name:?} has unsupported params"
                        ));
                    }
                };
                let ret_expr = match ty.results.as_slice() {
                    [] => "let _ = res; Ok(())".to_string(),
                    [ValType::I32] => "Ok(res as u32 as i32)".to_string(),
                    [ValType::I64] => "Ok(res as i64)".to_string(),
                    [ValType::F32] => "Ok(f32::from_bits(res as u32))".to_string(),
                    [ValType::F64] => "Ok(f64::from_bits(res))".to_string(),
                    [ValType::Ref(_)] => "Ok(res)".to_string(),
                    _ => {
                        return Err(format!(
                            "Blot import {module:?}.{name:?} has unsupported results"
                        ));
                    }
                };
                writeln!(
                    s,
                    "    // {module:?}.{name:?}\n    #[inline(never)]\n    pub fn import{k}{} {{\n        let f = self.host_call;\n        let res = f(self, mb, {k}, {tok_expr}, {arg_expr})?;\n        {ret_expr}\n    }}",
                    self.import_signature(k)?
                )
                .unwrap();
            }
            s.push_str("}\n\n");
        } else {
            s.push_str("    /// Instantiation is complete: the host may use the memory and goose.reserve.\n    pub ready: bool,\n");
            writeln!(s, "    /// Runs `_start` (or reports why it cannot).\n    pub start: fn(&mut Ctx, {}) -> R<()>,", self.mb_type()).unwrap();
            writeln!(
                s,
                "    /// The exported `goose.reserve`, if any.\n    pub reserve: Option<fn(&mut Ctx, {}, i64, i64) -> R<i64>>,",
                self.mb_type()
            )
            .unwrap();
            writeln!(
                s,
                "    pub imports: [goose::Handler; {}],",
                self.imports.len()
            )
            .unwrap();
            s.push_str("    pub state: goose::State,\n}\n\n");

            // Top-level Instance owning both Ctx and the disjoint memory buffer.
            writeln!(
                s,
                "pub struct Instance {{\n    pub ctx: Ctx,\n    pub mem: {},\n}}\n",
                if !self.memory64 {
                    "Box<[u8; CAP32]>"
                } else if self.cap64.is_some() {
                    "Box<[u8; CAP64]>"
                } else {
                    "Vec<u8>"
                }
            )
            .unwrap();
            s.push_str("impl goose::Guest for Instance {\n");
            s.push_str("    fn memory(&mut self) -> Option<&mut dyn LinearMemory> { None }\n");
            s.push_str("    fn state(&mut self) -> &mut goose::State { &mut self.ctx.state }\n");
            s.push_str("    fn reserve(&mut self, _: i64, _: i64) -> Option<R<i64>> { None }\n");
            s.push_str("    fn start(&mut self) -> R<()> { (self.ctx.start)(&mut self.ctx, &mut self.mem) }\n}\n\n");

            // Temporary zero-allocation host view pairing `Ctx` and `mb` for host calls.
            writeln!(
                s,
                "struct HostView<'a> {{\n    mem: {},\n    ctx: &'a mut Ctx,\n}}\n",
                if !self.memory64 {
                    "Mem32Host<'a, CAP32>"
                } else if self.cap64.is_some() {
                    "Mem64FixedHost<'a, CAP64>"
                } else {
                    "Mem64Host<'a>"
                }
            )
            .unwrap();
            s.push_str("impl goose::Guest for HostView<'_> {\n");
            s.push_str(if self.memory_exported && self.has_memory {
                "    fn memory(&mut self) -> Option<&mut dyn LinearMemory> { if self.ctx.ready { Some(&mut self.mem) } else { None } }\n"
            } else {
                "    fn memory(&mut self) -> Option<&mut dyn LinearMemory> { None }\n"
            });
            s.push_str("    fn state(&mut self) -> &mut goose::State { &mut self.ctx.state }\n");
            if self.memory64 {
                s.push_str(
                    "    fn reserve(&mut self, control: i64, count: i64) -> Option<R<i64>> {\n\
                     \x20       match self.ctx.reserve {\n\
                     \x20           Some(reserve) if self.ctx.ready => {\n\
                     \x20               self.ctx.outer = std::mem::replace(&mut self.mem.outer, Outer::empty());\n\
                     \x20               let res = reserve(self.ctx, self.mem.base, control, count);\n\
                     \x20               self.mem.outer = std::mem::replace(&mut self.ctx.outer, Outer::empty());\n\
                     \x20               Some(res)\n\
                     \x20           }\n\
                     \x20           _ => None,\n\
                     \x20       }\n\
                     \x20   }\n",
                );
            } else {
                s.push_str(
                    "    fn reserve(&mut self, control: i64, count: i64) -> Option<R<i64>> {\n\
                     \x20       match self.ctx.reserve {\n\
                     \x20           Some(reserve) if self.ctx.ready => {\n\
                     \x20               self.ctx.mem = self.mem.meta;\n\
                     \x20               let res = reserve(self.ctx, self.mem.bytes, control, count);\n\
                     \x20               self.mem.meta = self.ctx.mem;\n\
                     \x20               Some(res)\n\
                     \x20           }\n\
                     \x20           _ => None,\n\
                     \x20       }\n\
                     \x20   }\n",
                );
            }
            s.push_str("    fn start(&mut self) -> R<()> { trap(Trap::NoStart) }\n}\n\n");

            // Host imports on Ctx.
            s.push_str("impl Ctx {\n");
            for (k, (module, name)) in self.imports.iter().enumerate() {
                let ty = &self.types[k];
                let vals: Vec<String> = ty
                    .params
                    .iter()
                    .enumerate()
                    .map(|(i, t)| {
                        Ok(match t {
                            ValType::I32 => format!("Val::I32(p{i})"),
                            ValType::I64 => format!("Val::I64(p{i})"),
                            ValType::F32 => format!("Val::F32(p{i})"),
                            ValType::F64 => format!("Val::F64(p{i})"),
                            _ => {
                                return Err(format!(
                                    "import {module:?}.{name:?} takes an unsupported type"
                                ));
                            }
                        })
                    })
                    .collect::<Res<_>>()?;
                let fit = match ty.results.as_slice() {
                    [] => "none",
                    [ValType::I32] => "i32",
                    [ValType::I64] => "i64",
                    [ValType::F32] => "f32",
                    [ValType::F64] => "f64",
                    _ => {
                        return Err(format!(
                            "import {module:?}.{name:?} has unsupported results"
                        ));
                    }
                };
                let cold = if self.noreturn_imports[k] {
                    "    #[cold]\n"
                } else {
                    ""
                };
                let (mk_view, restore) = if !self.memory64 {
                    (
                        "let mut view = HostView { mem: Mem32Host { bytes: mb, meta: self.mem }, ctx: self };",
                        "view.ctx.mem = view.mem.meta;",
                    )
                } else if self.cap64.is_some() {
                    (
                        "let mut view = HostView { mem: Mem64FixedHost { base: mb, outer: std::mem::replace(&mut self.outer, Outer::empty()) }, ctx: self };",
                        "view.ctx.outer = view.mem.outer;",
                    )
                } else {
                    (
                        "let mut view = HostView { mem: Mem64Host { base: mb, outer: std::mem::replace(&mut self.outer, Outer::empty()) }, ctx: self };",
                        "view.ctx.outer = view.mem.outer;",
                    )
                };
                let ret = if self.noreturn_imports[k] {
                    "match res { Err(error) => error, Ok(_) => Trap::Host }".to_string()
                } else {
                    format!("res?.{fit}()")
                };
                writeln!(
                    s,
                    "    // {module:?}.{name:?}\n{cold}    #[inline(never)]\n    pub fn import{k}{} {{\n        let handler = self.imports[{k}];\n        {mk_view}\n        let res = handler(&mut view, &[{}]);\n        {restore}\n        {ret}\n    }}",
                    self.import_signature(k)?,
                    vals.join(", ")
                )
                .unwrap();
            }
            s.push_str("}\n\n");
        }

        // Fns trait for defined functions and indirect-call dispatchers.
        s.push_str("/// Every defined function of the module. The part crates define the functions\n/// generically over `M` and call each other through this trait; the\n/// program crate implements it for its marker, which monomorphizes them all\n/// there, with direct calls and disjoint `noalias` memory.\npub trait Fns<M> {\n");
        for f in self.imports.len()..self.types.len() {
            writeln!(s, "    fn f{f}{};", self.fn_signature(f, "&mut self")?).unwrap();
        }
        for ci_id in 0..self.ci_sigs.len() {
            writeln!(s, "    fn ci{ci_id}{};", self.ci_signature(ci_id)?).unwrap();
        }
        s.push_str("}\n");
        let _ = m;
        Ok(s)
    }

    /// Emits `pub struct Marker; impl Fns<Marker> for Ctx { ... }` (all defined
    /// functions and `call_indirect` dispatchers).
    fn emit_fns_impl(&self, s: &mut String, paths: &[String]) -> Res<()> {
        s.push_str("pub struct Marker;\n\n");

        // Emit O(1) static function-pointer tables for polymorphic `call_indirect`
        // dispatchers (`>= CI_TABLE_MIN_TARGETS`), keeping small dispatchers as
        // inlined `match` statements for monomorphic/bimorphic devirtualization.
        let mut uses_table = vec![false; self.ci_sigs.len()];
        let mut table_min = vec![0usize; self.ci_sigs.len()];
        for ci_id in 0..self.ci_sigs.len() {
            let targets = &self.ci_targets[ci_id];
            if targets.len() < CI_TABLE_MIN_TARGETS {
                continue;
            }
            let min_slot = targets[0].0 as usize;
            let max_slot = targets.last().unwrap().0 as usize;
            let span = max_slot - min_slot + 1;
            if span > CI_TABLE_MAX_SPAN {
                continue;
            }
            uses_table[ci_id] = true;
            table_min[ci_id] = min_slot;
            let (_, ti) = self.ci_sigs[ci_id];
            let ty = &self.module_types[ti as usize];
            let ret_ty = fn_return_type(&ty.results)?;
            let mut fn_param_tys = vec!["&mut Ctx".to_string(), self.mb_type().to_string()];
            let mut fn_params_named = vec![
                "ctx: &mut Ctx".to_string(),
                format!("mb: {}", self.mb_type()),
            ];
            for &g in &self.ci_reg_globals[ci_id] {
                let rt = rust_type(self.globals[g as usize].0)?;
                fn_param_tys.push(rt.to_string());
                fn_params_named.push(format!("gv{g}: {rt}"));
            }
            for (i, t) in ty.params.iter().enumerate() {
                let rt = rust_type(*t)?;
                fn_param_tys.push(rt.to_string());
                fn_params_named.push(format!("p{i}: {rt}"));
            }
            writeln!(
                s,
                "type Ci{ci_id}Fn = fn({}) -> {ret_ty};",
                fn_param_tys.join(", ")
            )
            .unwrap();
            let mut entries: Vec<(usize, String)> = Vec::with_capacity(targets.len());
            let mut emitted_wrappers: BTreeMap<usize, String> = BTreeMap::new();
            for &(slot, f) in targets {
                let fn_expr = if f >= self.imports.len()
                    && self.fn_reg_globals(f) == self.ci_reg_globals[ci_id]
                {
                    format!("{}::<Marker>", paths[f])
                } else if let Some(w) = emitted_wrappers.get(&f) {
                    w.clone()
                } else {
                    let wname = format!("ci{ci_id}_w{f}");
                    let call_body = if f < self.imports.len() {
                        let mut a = vec!["mb".to_string()];
                        for i in 0..ty.params.len() {
                            a.push(format!("p{i}"));
                        }
                        if self.noreturn_imports[f] {
                            ret_err_expr(
                                ret_abi(&ty.results),
                                &format!("ctx.import{f}({})", a.join(", ")),
                            )?
                        } else {
                            result_to_ret_abi(
                                &ty.results,
                                &format!("ctx.import{f}({})", a.join(", ")),
                            )?
                        }
                    } else {
                        let mut a = vec!["ctx".to_string(), "mb".to_string()];
                        for g in self.fn_reg_globals(f) {
                            a.push(format!("gv{g}"));
                        }
                        for i in 0..ty.params.len() {
                            a.push(format!("p{i}"));
                        }
                        format!("{}::<Marker>({})", paths[f], a.join(", "))
                    };
                    writeln!(
                        s,
                        "fn {wname}({}) -> {ret_ty} {{ {call_body} }}",
                        fn_params_named.join(", ")
                    )
                    .unwrap();
                    emitted_wrappers.insert(f, wname.clone());
                    wname
                };
                entries.push((slot as usize - min_slot, fn_expr));
            }
            writeln!(
                s,
                "static CI{ci_id}_TBL: [Option<Ci{ci_id}Fn>; {span}] = {{"
            )
            .unwrap();
            writeln!(
                s,
                "    let mut t: [Option<Ci{ci_id}Fn>; {span}] = [None; {span}];"
            )
            .unwrap();
            for (rel_slot, fn_expr) in entries {
                writeln!(s, "    t[{rel_slot}] = Some({fn_expr});").unwrap();
            }
            s.push_str("    t\n};\n\n");
        }

        s.push_str("impl Fns<Marker> for Ctx {\n");
        for (f, path) in paths.iter().enumerate().skip(self.imports.len()) {
            let mut all = vec!["self".to_string(), "mb".to_string()];
            for g in self.fn_reg_globals(f) {
                all.push(format!("gv{g}"));
            }
            for i in 0..self.types[f].params.len() {
                all.push(format!("p{i}"));
            }
            let attr = if self.fn_inline_always(f) {
                "#[inline(always)]"
            } else {
                "#[inline]"
            };
            writeln!(
                s,
                "    {attr}\n    fn f{f}{} {{ {path}::<Marker>({}) }}",
                self.fn_signature(f, "&mut self")?,
                all.join(", ")
            )
            .unwrap();
        }
        for ci_id in 0..self.ci_sigs.len() {
            let (tbl, ti) = self.ci_sigs[ci_id];
            let ty = &self.module_types[ti as usize];
            let abi = ret_abi(&ty.results);
            let targets = &self.ci_targets[ci_id];
            let tlen = self.static_func_tables[&tbl];
            let oob_err = ret_err_expr(abi, "Trap::TableOutOfBounds")?;
            let bad_sig_err = ret_err_expr(abi, "Trap::IndirectCall")?;
            if uses_table[ci_id] {
                let min_slot = table_min[ci_id];
                let mut call_args = vec!["self".to_string(), "mb".to_string()];
                for &g in &self.ci_reg_globals[ci_id] {
                    call_args.push(format!("gv{g}"));
                }
                for i in 0..ty.params.len() {
                    call_args.push(format!("p{i}"));
                }
                let lookup = if min_slot == 0 {
                    format!("CI{ci_id}_TBL.get(u).copied()")
                } else {
                    format!("u.checked_sub({min_slot}).and_then(|k| CI{ci_id}_TBL.get(k).copied())")
                };
                writeln!(
                    s,
                    "    #[inline]\n    fn ci{ci_id}{} {{\n        let u = idx as u32 as usize;\n        if u >= {tlen} {{ return {oob_err}; }}\n        if let Some(Some(f)) = {lookup} {{ return f({}); }}\n        {bad_sig_err}\n    }}",
                    self.ci_signature(ci_id)?,
                    call_args.join(", ")
                )
                .unwrap();
                continue;
            }
            let attr = if targets.len() <= 2 {
                "#[inline(always)]\n    "
            } else {
                "#[inline]\n    "
            };
            writeln!(s, "    {attr}fn ci{ci_id}{} {{", self.ci_signature(ci_id)?).unwrap();
            if targets.is_empty() {
                writeln!(s, "        if (idx as u32 as usize) >= {tlen} {{ {oob_err} }} else {{ {bad_sig_err} }}\n    }}").unwrap();
            } else {
                s.push_str("        match idx as u32 {\n");
                for &(slot, f) in targets {
                    let call = if f < self.imports.len() {
                        let mut a = vec!["mb".to_string()];
                        for i in 0..ty.params.len() {
                            a.push(format!("p{i}"));
                        }
                        if self.noreturn_imports[f] {
                            ret_err_expr(abi, &format!("self.import{f}({})", a.join(", ")))?
                        } else {
                            result_to_ret_abi(
                                &ty.results,
                                &format!("self.import{f}({})", a.join(", ")),
                            )?
                        }
                    } else {
                        let mut a = vec!["self".to_string(), "mb".to_string()];
                        for g in self.fn_reg_globals(f) {
                            a.push(format!("gv{g}"));
                        }
                        for i in 0..ty.params.len() {
                            a.push(format!("p{i}"));
                        }
                        format!("{}::<Marker>({})", paths[f], a.join(", "))
                    };
                    writeln!(s, "            {slot} => {call},").unwrap();
                }
                writeln!(
                    s,
                    "            _ => if (idx as u32 as usize) >= {tlen} {{ {oob_err} }} else {{ {bad_sig_err} }},\n        }}\n    }}"
                )
                .unwrap();
            }
        }
        s.push_str("}\n\n");
        Ok(())
    }

    /// Helper for calling function `f` from top-level `initialize`/exports,
    /// converting its `RetAbi` back to `R<T>`.
    fn top_call(&self, f: usize, extra_args: &[String]) -> String {
        if f < self.imports.len() {
            let mut a = vec!["mb".to_string()];
            a.extend_from_slice(extra_args);
            if self.noreturn_imports[f] {
                format!("Err(ctx.import{f}({}))", a.join(", "))
            } else {
                format!("ctx.import{f}({})", a.join(", "))
            }
        } else {
            let mut a = vec!["mb".to_string()];
            for g in self.fn_reg_globals(f) {
                a.push(format!("ctx.g{g}"));
            }
            a.extend_from_slice(extra_args);
            let raw = format!("ctx.f{f}({})", a.join(", "));
            match ret_abi(&self.types[f].results) {
                RetAbi::Void => format!("match {raw} {{ None => Ok(()), Some(t) => Err(t) }}"),
                RetAbi::Scalar(_) => {
                    format!("match {raw} {{ (v, None) => Ok(v), (_, Some(t)) => Err(t) }}")
                }
                RetAbi::Pair32(t0, t1) => {
                    let v0 = unpack_u32_lane(t0, "p as u32");
                    let v1 = unpack_u32_lane(t1, "(p >> 32) as u32");
                    format!(
                        "match {raw} {{ (p, None) => Ok(({v0}, {v1})), (_, Some(t)) => Err(t) }}"
                    )
                }
                RetAbi::Result => raw,
            }
        }
    }

    /// Emits `fn initialize(ctx, mb) -> R<()>` (active elements, active data, start).
    fn emit_initialize(&self, m: &DecodedModule, s: &mut String, data: &mut Vec<u8>) -> Res<()> {
        writeln!(
            s,
            "fn initialize(ctx: &mut Ctx, mb: {}) -> R<()> {{",
            self.mb_type()
        )
        .unwrap();
        for element in &m.elements {
            if let SegmentMode::Active { index, offset } = &element.mode {
                let table = m
                    .tables
                    .get(*index as usize)
                    .ok_or("element segment for an imported table")?;
                let offset = constant(offset, self.globals.len(), "ctx.", &self.global_consts)?;
                let offset = if table.index_type == wtime_types::IndexType::I64 {
                    format!("({offset}) as u64")
                } else {
                    format!("({offset}) as u32 as u64")
                };
                writeln!(
                    s,
                    "    if ({offset}).checked_add({}).is_none_or(|end| end > {}) {{ return trap(Trap::TableOutOfBounds); }}",
                    element.init.len(),
                    table.limits.min
                )
                .unwrap();
            }
        }
        for segment in &m.data {
            let bytes = &m.bytes[segment.bytes.start..segment.bytes.end];
            let at = data.len();
            data.extend_from_slice(bytes);
            match &segment.mode {
                SegmentMode::Active { index: 0, offset } => {
                    let offset = constant(offset, self.globals.len(), "ctx.", &self.global_consts)?;
                    let address = if self.memory64 {
                        format!("({offset}) as u64")
                    } else {
                        format!("({offset}) as u32 as u64")
                    };
                    if self.memory64 {
                        writeln!(s, "    init_parts(mb, &mut ctx.outer, {address}, &DATA[{at}..{}], 0, {})?;", at + bytes.len(), bytes.len()).unwrap();
                    } else {
                        writeln!(
                            s,
                            "    ctx.mem.init(mb, {address}, &DATA[{at}..{}], 0, {})?;",
                            at + bytes.len(),
                            bytes.len()
                        )
                        .unwrap();
                    }
                }
                SegmentMode::Passive => {}
                _ => return Err("data segments for other memories are not supported".into()),
            }
        }
        if let Some(start) = self.start {
            writeln!(s, "    {}?;", self.top_call(start as usize, &[])).unwrap();
        }
        s.push_str("    Ok(())\n}\n\n");
        Ok(())
    }

    /// The program crate's main.rs: `Fns` for its marker, instantiation and
    /// `main`. `paths[f]` is the path of defined function `f`.
    fn main(
        &self,
        m: &DecodedModule,
        data: &mut Vec<u8>,
        module_path: &str,
        core: &str,
        paths: &[String],
    ) -> Res<String> {
        let mut s = String::new();
        s.push_str("// Generated by wtime-aot. Do not edit.\n#![forbid(unsafe_code)]\n#![allow(unused, non_snake_case, unreachable_code, unused_parens, clippy::all)]\n\n");
        writeln!(s, "use {core}::*;\n").unwrap();
        s.push_str("static DATA: &[u8] = include_bytes!(\"data.bin\");\n\n");
        self.emit_fns_impl(&mut s, paths)?;

        // Instantiation.
        s.push_str("fn instantiate(state: &mut goose::State) -> R<Instance> {\n");
        match m.memories.first() {
            Some(mt) if self.memory64 && self.cap64.is_some() => {
                writeln!(
                    s,
                    "    let outer = Outer::new_flat_meta({}, {:?}, CAP64)?;",
                    mt.limits.min, mt.limits.max
                )
                .unwrap();
                s.push_str("    let mem = alloc64::<CAP64>()?;\n");
            }
            Some(mt) if self.memory64 => {
                writeln!(
                    s,
                    "    let (mem, outer) = Memory::new_parts({}, {:?}, true, {})?;",
                    mt.limits.min, mt.limits.max, self.flat64
                )
                .unwrap();
            }
            Some(mt) => {
                writeln!(
                    s,
                    "    let mem_meta = Mem32Meta::new({}, {:?})?;",
                    mt.limits.min, mt.limits.max
                )
                .unwrap();
                s.push_str("    let mem = alloc32::<CAP32>()?;\n");
            }
            None => {
                s.push_str("    let mem_meta = Mem32Meta::new(0, Some(0))?;\n    let mem = alloc32::<CAP32>()?;\n");
            }
        }
        s.push_str("    let mut imports: Vec<goose::Handler> = Vec::new();\n");
        for (module, name) in &self.imports {
            let unsupported = format!("unsupported direct host import: {module}.{name}");
            writeln!(
                s,
                "    imports.push(match goose::handler({module:?}, {name:?}) {{\n        Some(handler) => handler,\n        None => {{\n            state.failure = Some({unsupported:?}.into());\n            return trap(Trap::Host);\n        }}\n    }});"
            )
            .unwrap();
        }
        s.push_str("    let imports = imports.try_into().map_err(|_| Trap::Host)?;\n");
        s.push_str("    let owned = std::mem::replace(state, goose::State::new(Vec::new(), Vec::new()));\n");
        // Non-constant globals in order.
        for (k, (t, init)) in self.globals.iter().enumerate() {
            if self.global_consts[k].is_none() {
                writeln!(s, "    let g{k}: {} = {init};", rust_type(*t)?).unwrap();
            }
        }
        writeln!(
            s,
            "    let ctx = Ctx {{\n        stack_floor: goose::stack_floor(),\n        {},",
            if self.memory64 {
                "outer"
            } else {
                "mem: mem_meta"
            }
        )
        .unwrap();
        for k in 0..self.globals.len() {
            if self.global_consts[k].is_none() {
                writeln!(s, "        g{k},").unwrap();
            }
        }
        for (&tbl, &len) in &self.mut_extern_tables {
            writeln!(s, "        t{tbl}: wtime_aot_rt::zeroed_u64({len})?,").unwrap();
        }
        writeln!(
            s,
            "        ready: false,\n        start,\n        reserve: {},\n        imports,\n        state: owned,\n    }};",
            if self.reserve.is_some() { "Some(reserve)" } else { "None" }
        )
        .unwrap();
        let refused = m.exports.iter().any(|e| e.name == "__set_stack_limits");
        s.push_str("    let mut instance = Instance { ctx, mem };\n    match initialize(&mut instance.ctx, &mut instance.mem) {\n");
        if refused {
            s.push_str("        Ok(()) => {\n            instance.ctx.state.failure = Some(\"emscripten standalone modules are not supported by this host\".into());\n            *state = std::mem::replace(&mut instance.ctx.state, goose::State::new(Vec::new(), Vec::new()));\n            trap(Trap::Host)\n        }\n");
        } else {
            s.push_str("        Ok(()) => {\n            instance.ctx.ready = true;\n            Ok(instance)\n        }\n");
        }
        s.push_str("        Err(error) => {\n            *state = std::mem::replace(&mut instance.ctx.state, goose::State::new(Vec::new(), Vec::new()));\n            Err(error)\n        }\n    }\n}\n\n");

        self.emit_initialize(m, &mut s, data)?;
        let mb_ty = self.mb_type();
        match self.entry {
            Some(f) if self.types[f as usize].params.is_empty() => {
                writeln!(s, "fn start(ctx: &mut Ctx, mb: {mb_ty}) -> R<()> {{ {}.map(|_| ()) }}\n", self.top_call(f as usize, &[])).unwrap()
            }
            Some(f) if (f as usize) < self.imports.len() => {
                writeln!(s, "fn start(_: &mut Ctx, _: {mb_ty}) -> R<()> {{ trap(Trap::StartImportArguments) }}\n").unwrap()
            }
            Some(_) => writeln!(s, "fn start(_: &mut Ctx, _: {mb_ty}) -> R<()> {{ trap(Trap::StartArguments) }}\n").unwrap(),
            None if m.exports.iter().any(|e| e.name == "_start") => {
                writeln!(s, "fn start(_: &mut Ctx, _: {mb_ty}) -> R<()> {{ trap(Trap::StartNotFunction) }}\n").unwrap()
            }
            None => writeln!(s, "fn start(_: &mut Ctx, _: {mb_ty}) -> R<()> {{ trap(Trap::NoStart) }}\n").unwrap(),
        }
        if let Some(f) = self.reserve {
            let ty = &self.types[f as usize];
            let arg = |t: &ValType, v: &str| match t {
                ValType::I32 => Ok(format!("{v} as i32")),
                ValType::I64 => Ok(v.to_string()),
                _ => Err("goose.reserve is not (address, address) -> address".to_string()),
            };
            let (call, fit) = match (ty.params.as_slice(), ty.results.as_slice()) {
                ([a, b], [r]) => (
                    self.top_call(f as usize, &[arg(a, "control")?, arg(b, "count")?]),
                    match r {
                        ValType::I32 => ".map(|v| v as i64)",
                        ValType::I64 => "",
                        _ => {
                            return Err("goose.reserve is not (address, address) -> address".into());
                        }
                    },
                ),
                _ => return Err("goose.reserve is not (address, address) -> address".into()),
            };
            writeln!(s, "fn reserve(ctx: &mut Ctx, mb: {mb_ty}, control: i64, count: i64) -> R<i64> {{ {call}{fit} }}\n").unwrap();
        }
        writeln!(
            s,
            "fn main() {{\n    goose::main({module_path:?}, {STACK_BYTES}, instantiate);\n}}"
        )
        .unwrap();
        Ok(s)
    }

    /// Generates `src/lib.rs` for a Blot ABI 8 `cdylib` loaded by `pen/blot-host`.
    /// All generated Wasm code (`core` and `p0..pN`) stays 100% `#![forbid(unsafe_code)]`;
    /// only this thin C-ABI boundary exposes opaque instance & active-call pointers.
    fn blot_lib(
        &self,
        m: &DecodedModule,
        data: &mut Vec<u8>,
        core: &str,
        paths: &[String],
    ) -> Res<String> {
        let mut s = String::new();
        s.push_str("// Generated by wtime-aot (--blot-cdylib). Do not edit.\n#![allow(unused, non_snake_case, unreachable_code, unused_parens, clippy::all)]\n\n");
        s.push_str("use std::ffi::c_void;\n");
        writeln!(s, "use {core}::*;\n").unwrap();
        s.push_str("static DATA: &[u8] = include_bytes!(\"data.bin\");\n\n");
        self.emit_fns_impl(&mut s, paths)?;
        self.emit_initialize(m, &mut s, data)?;

        let max_ffi_name_len = m
            .exports
            .iter()
            .map(|e| e.name.len())
            .chain(self.exported_globals.iter().map(|(n, ..)| n.len()))
            .max()
            .unwrap_or(0);
        // Host trampoline, MXCSR/stack-floor isolation, & opaque C-ABI handle structs.
        // `BlotHandle::in_call` lives in a disjoint `Cell<bool>` checked via raw
        // `addr_of!` before any reference to `ctx` or `mem` is formed, eliminating
        // aliasing `&mut` borrows if a host callback re-enters on the outer handle.
        writeln!(
            s,
            "pub type HostFn = unsafe extern \"C\" fn(*mut c_void, *mut ActiveCall, u32, u64, u64, *mut u64) -> i32;\n\n\
             const MAX_FFI_NAME_LEN: usize = {max_ffi_name_len};\n"
        )
        .unwrap();
        s.push_str(
            "#[repr(C)]\n\
             pub struct ActiveCall {\n\
                 ctx: *mut Ctx,\n\
                 mem_ptr: *mut u8,\n\
                 mem_len: usize,\n\
             }\n\n\
             struct HostBridge {\n\
                 user_ctx: *mut c_void,\n\
                 host_fn: Option<HostFn>,\n\
             }\n\n\
             struct MemBuf {\n\
                 ptr: *mut u8,\n\
                 len: usize,\n\
             }\n\n\
             impl MemBuf {\n\
                 fn alloc_zeroed(len: usize) -> R<Self> {\n\
                     if len == 0 {\n\
                         return Ok(Self { ptr: std::ptr::NonNull::dangling().as_ptr(), len: 0 });\n\
                     }\n\
                     let Ok(layout) = std::alloc::Layout::from_size_align(len, 16) else {\n\
                         return trap(Trap::MemoryTooLarge);\n\
                     };\n\
                     let ptr = unsafe { std::alloc::alloc_zeroed(layout) };\n\
                     if ptr.is_null() {\n\
                         return trap(Trap::AllocationFailed);\n\
                     }\n\
                     Ok(Self { ptr, len })\n\
                 }\n\
                 #[inline(always)]\n\
                 fn as_mut_slice(&mut self) -> &mut [u8] {\n\
                     unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }\n\
                 }\n\
                 #[inline(always)]\n\
                 fn as_mut_ptr(&self) -> *mut u8 {\n\
                     if self.len == 0 { std::ptr::null_mut() } else { self.ptr }\n\
                 }\n\
             }\n\n\
             impl Drop for MemBuf {\n\
                 fn drop(&mut self) {\n\
                     if self.len > 0 && !self.ptr.is_null() {\n\
                         if let Ok(layout) = std::alloc::Layout::from_size_align(self.len, 16) {\n\
                             unsafe { std::alloc::dealloc(self.ptr, layout); }\n\
                         }\n\
                         self.ptr = std::ptr::null_mut();\n\
                         self.len = 0;\n\
                     }\n\
                 }\n\
             }\n\n\
             pub struct BlotHandle {\n\
                 in_call: std::cell::Cell<bool>,\n\
                 ctx: std::cell::UnsafeCell<Box<Ctx>>,\n\
                 mem: std::cell::UnsafeCell<MemBuf>,\n\
                 bridge: *mut HostBridge,\n\
             }\n\n",
        );
        s.push_str(
            "impl Drop for BlotHandle {\n\
                 fn drop(&mut self) {\n\
                     if !self.bridge.is_null() {\n\
                         unsafe { drop(Box::from_raw(self.bridge)); }\n\
                         self.bridge = std::ptr::null_mut();\n\
                     }\n\
                 }\n\
             }\n\n\
             #[inline(always)]\n\
             unsafe fn handle_in_call(handle: *mut BlotHandle) -> bool {\n\
                 unsafe { (*std::ptr::addr_of!((*handle).in_call)).get() }\n\
             }\n\n\
             struct InCallGuard(*const std::cell::Cell<bool>);\n\
             impl InCallGuard {\n\
                 #[inline(always)]\n\
                 unsafe fn enter(handle: *mut BlotHandle) -> Self {\n\
                     let flag = unsafe { std::ptr::addr_of!((*handle).in_call) };\n\
                     unsafe { (*flag).set(true); }\n\
                     Self(flag)\n\
                 }\n\
             }\n\
             impl Drop for InCallGuard {\n\
                 #[inline(always)]\n\
                 fn drop(&mut self) {\n\
                     unsafe { (*self.0).set(false); }\n\
                 }\n\
             }\n\n\
             #[inline(always)]\n\
             fn write_trap(out: *mut u8, t: Trap) {\n\
                 if !out.is_null() { unsafe { *out = t.to_code(); } }\n\
             }\n\n\
             fn read_ffi_name<'a>(ptr: *const u8, len: usize) -> Option<&'a str> {\n\
                 if len == 0 { return Some(\"\"); }\n\
                 if ptr.is_null() || len > MAX_FFI_NAME_LEN { return None; }\n\
                 let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };\n\
                 std::str::from_utf8(bytes).ok()\n\
             }\n\n\
             #[cfg(target_arch = \"x86_64\")]\n\
             #[inline(never)]\n\
             fn get_mxcsr() -> u32 {\n\
                 let mut val: u32 = 0;\n\
                 unsafe { std::arch::asm!(\"stmxcsr [{}]\", in(reg) &mut val, options(nostack, preserves_flags)); }\n\
                 val\n\
             }\n\n\
             #[cfg(target_arch = \"x86_64\")]\n\
             #[inline(never)]\n\
             fn set_mxcsr(val: u32) {\n\
                 unsafe { std::arch::asm!(\"ldmxcsr [{}]\", in(reg) &val, options(nostack, preserves_flags)); }\n\
             }\n\n\
             #[cfg(not(target_arch = \"x86_64\"))]\n\
             #[inline(never)]\n\
             fn get_mxcsr() -> u32 { 0x1f80 }\n\n\
             #[cfg(not(target_arch = \"x86_64\"))]\n\
             #[inline(never)]\n\
             fn set_mxcsr(_val: u32) {}\n\n\
             #[inline(never)]\n\
             fn run_with_wasm_mxcsr<A, O>(ctx: &mut Ctx, mb: &mut [u8], arg: A, f: fn(&mut Ctx, &mut [u8], A) -> O, save_host_mxcsr: bool) -> O {\n\
                 let saved = get_mxcsr();\n\
                 if save_host_mxcsr {\n\
                     ctx.host_mxcsr = saved;\n\
                 }\n\
                 set_mxcsr(0x1f80);\n\
                 let callee = std::hint::black_box(f);\n\
                 let out = callee(ctx, mb, arg);\n\
                 set_mxcsr(saved);\n\
                 out\n\
             }\n\n\
             #[inline(never)]\n\
             fn sp_here() -> usize {\n\
                 let probe = 0u8;\n\
                 (&probe as *const u8) as usize\n\
             }\n\n\
             #[cfg(target_os = \"linux\")]\n\
             #[inline(never)]\n\
             fn blot_stack_floor() -> usize {\n\
                 use std::cell::Cell;\n\
                 #[repr(C, align(8))]\n\
                 struct PthreadAttr([u8; 128]);\n\
                 unsafe extern \"C\" {\n\
                     fn pthread_self() -> usize;\n\
                     fn pthread_getattr_np(thread: usize, attr: *mut PthreadAttr) -> i32;\n\
                     fn pthread_attr_getstack(attr: *const PthreadAttr, stackaddr: *mut *mut c_void, stacksize: *mut usize) -> i32;\n\
                     fn pthread_attr_destroy(attr: *mut PthreadAttr) -> i32;\n\
                 }\n\
                 thread_local! {\n\
                     static TLS_STACK: Cell<(usize, usize, usize)> = const { Cell::new((0, 0, 0)) };\n\
                 }\n\
                 let probe = 0u8;\n\
                 let sp = (&probe as *const u8) as usize;\n\
                 let budget_floor = sp.saturating_sub(7 << 20);\n\
                 let cached = TLS_STACK.with(|c| c.get());\n\
                 if sp >= cached.0 && sp < cached.1 {\n\
                     return cached.2.max(budget_floor);\n\
                 }\n\
                 let mut attr = PthreadAttr([0u8; 128]);\n\
                 if unsafe { pthread_getattr_np(pthread_self(), &mut attr) } == 0 {\n\
                     let mut addr: *mut c_void = std::ptr::null_mut();\n\
                     let mut size: usize = 0;\n\
                     let ok = unsafe { pthread_attr_getstack(&attr, &mut addr, &mut size) };\n\
                     unsafe { pthread_attr_destroy(&mut attr); }\n\
                     if ok == 0 && !addr.is_null() && size > 0 {\n\
                         let low = addr as usize;\n\
                         let high = low.saturating_add(size);\n\
                         if high > low && sp >= low && sp < high {\n\
                             let headroom = if size >= (256 << 10) { 128 << 10 } else { size / 2 };\n\
                             let real_floor = low.saturating_add(headroom);\n\
                             TLS_STACK.with(|c| c.set((low, high, real_floor)));\n\
                             return real_floor.max(budget_floor);\n\
                         }\n\
                     }\n\
                 }\n\
                 usize::MAX\n\
             }\n\n\
             #[cfg(not(target_os = \"linux\"))]\n\
             #[inline(never)]\n\
             fn blot_stack_floor() -> usize {\n\
                 stack_floor_for(7 << 20)\n\
             }\n\n\
             fn host_trampoline(ctx: &mut Ctx, mb: &mut [u8], import_idx: u32, token: u64, arg_bits: u64) -> R<u64> {\n\
                 if ctx.host_ctx == 0 { return trap(Trap::Host); }\n\
                 let (user_ctx, host_fn) = unsafe {\n\
                     let bridge = &*(ctx.host_ctx as *const HostBridge);\n\
                     (bridge.user_ctx, bridge.host_fn)\n\
                 };\n\
                 let Some(host_fn) = host_fn else { return trap(Trap::Host); };\n\
                 let mut active = ActiveCall {\n\
                     ctx: ctx as *mut Ctx,\n\
                     mem_ptr: mb.as_mut_ptr(),\n\
                     mem_len: mb.len(),\n\
                 };\n\
                 let mut res: u64 = 0;\n\
                 let saved_host_mxcsr = ctx.host_mxcsr;\n\
                 set_mxcsr(saved_host_mxcsr);\n\
                 let status = unsafe { host_fn(user_ctx, &mut active as *mut ActiveCall, import_idx, token, arg_bits, &mut res as *mut u64) };\n\
                 set_mxcsr(0x1f80);\n\
                 if status == 0 { Ok(res) } else { trap(Trap::Host) }\n\
             }\n\n\
             #[unsafe(no_mangle)]\n\
             pub extern \"C\" fn blot_aot_abi_version() -> u32 { 1 }\n\n\
             #[unsafe(no_mangle)]\n\
             pub extern \"C\" fn blot_aot_trap_message(code: u8, ptr_out: *mut *const u8, len_out: *mut usize) -> i32 {\n\
                 match Trap::from_code(code) {\n\
                     Some(t) => {\n\
                         let msg = t.message();\n\
                         unsafe {\n\
                             if !ptr_out.is_null() { *ptr_out = msg.as_ptr(); }\n\
                             if !len_out.is_null() { *len_out = msg.len(); }\n\
                         }\n\
                         0\n\
                     }\n\
                     None => 1,\n\
                 }\n\
             }\n\n",
        );

        // Import metadata table.
        writeln!(s, "static IMPORTS: &[(&str, &str)] = &[").unwrap();
        for (module, name) in &self.imports {
            writeln!(s, "    ({module:?}, {name:?}),").unwrap();
        }
        s.push_str(
            "];\n\n\
             #[unsafe(no_mangle)]\n\
             pub extern \"C\" fn blot_aot_import_count() -> u32 { IMPORTS.len() as u32 }\n\n\
             #[unsafe(no_mangle)]\n\
             pub extern \"C\" fn blot_aot_import_name(idx: u32, mod_ptr: *mut *const u8, mod_len: *mut usize, name_ptr: *mut *const u8, name_len: *mut usize) -> i32 {\n\
                 let Some(&(m, n)) = IMPORTS.get(idx as usize) else { return 1; };\n\
                 unsafe {\n\
                     if !mod_ptr.is_null() { *mod_ptr = m.as_ptr(); }\n\
                     if !mod_len.is_null() { *mod_len = m.len(); }\n\
                     if !name_ptr.is_null() { *name_ptr = n.as_ptr(); }\n\
                     if !name_len.is_null() { *name_len = n.len(); }\n\
                 }\n\
                 0\n\
             }\n\n",
        );

        // blot_aot_new & blot_aot_drop.
        let (mem_min, mem_max) = match m.memories.first() {
            Some(mt) => (mt.limits.min, mt.limits.max),
            None => (0, Some(0)),
        };
        writeln!(
            s,
            "#[unsafe(no_mangle)]\n\
             pub extern \"C\" fn blot_aot_new(user_ctx: *mut c_void, host_fn: Option<HostFn>, mem_limit_pages: u64, trap_out: *mut u8) -> *mut BlotHandle {{\n\
                 let limit = if mem_limit_pages == 0 {{ MEMORY_LIMIT_PAGES }} else {{ mem_limit_pages }};\n\
                 let mem_meta = match Mem32Meta::new_with_limit({mem_min}, {mem_max:?}, limit) {{\n\
                     Ok(m) => m,\n\
                     Err(t) => {{ write_trap(trap_out, t); return std::ptr::null_mut(); }}\n\
                 }};\n\
                 let Some(cap_bytes) = mem_meta.max_pages.checked_mul(65536).and_then(|n| usize::try_from(n).ok()) else {{\n\
                     write_trap(trap_out, Trap::MemoryTooLarge);\n\
                     return std::ptr::null_mut();\n\
                 }};\n\
                 let mut mem = match MemBuf::alloc_zeroed(cap_bytes) {{\n\
                     Ok(m) => m,\n\
                     Err(t) => {{ write_trap(trap_out, t); return std::ptr::null_mut(); }}\n\
                 }};\n\
                 let stack_floor = blot_stack_floor();\n\
                 if sp_here() <= stack_floor {{\n\
                     write_trap(trap_out, Trap::CallStackExhausted);\n\
                     return std::ptr::null_mut();\n\
                 }}\n\
                 let bridge = Box::into_raw(Box::new(HostBridge {{ user_ctx, host_fn }}));\n\
                 let host_ctx = bridge as usize;"
        )
        .unwrap();
        for (k, (t, init)) in self.globals.iter().enumerate() {
            if self.global_consts[k].is_none() {
                writeln!(s, "    let g{k}: {} = {init};", rust_type(*t)?).unwrap();
            }
        }
        for (&tbl, &len) in &self.mut_extern_tables {
            writeln!(
                s,
                "    let t{tbl} = match wtime_aot_rt::zeroed_u64({len}) {{\n\
                 \x20       Ok(v) => v,\n\
                 \x20       Err(t) => {{ unsafe {{ drop(Box::from_raw(bridge)); }} write_trap(trap_out, t); return std::ptr::null_mut(); }}\n\
                 \x20   }};"
            )
            .unwrap();
        }
        // Allocate `Ctx` directly on the heap via fallible `alloc_zeroed` so
        // modules with large contexts never build `Ctx` on the host thread stack.
        s.push_str(
            "    let ctx_layout = std::alloc::Layout::new::<Ctx>();\n\
                 let ctx_raw = unsafe { std::alloc::alloc_zeroed(ctx_layout) as *mut Ctx };\n\
                 if ctx_raw.is_null() {\n\
                     unsafe { drop(Box::from_raw(bridge)); }\n\
                     write_trap(trap_out, Trap::AllocationFailed);\n\
                     return std::ptr::null_mut();\n\
                 }\n\
                 unsafe {\n\
                     std::ptr::addr_of_mut!((*ctx_raw).stack_floor).write(stack_floor);\n\
                     std::ptr::addr_of_mut!((*ctx_raw).mem).write(mem_meta);\n",
        );
        for k in 0..self.globals.len() {
            if self.global_consts[k].is_none() {
                writeln!(
                    s,
                    "        std::ptr::addr_of_mut!((*ctx_raw).g{k}).write(g{k});"
                )
                .unwrap();
            }
        }
        for &tbl in self.mut_extern_tables.keys() {
            writeln!(
                s,
                "        std::ptr::addr_of_mut!((*ctx_raw).t{tbl}).write(t{tbl});"
            )
            .unwrap();
        }
        s.push_str(
            "        std::ptr::addr_of_mut!((*ctx_raw).host_ctx).write(host_ctx);\n\
                     std::ptr::addr_of_mut!((*ctx_raw).host_mxcsr).write(0x1f80);\n\
                     std::ptr::addr_of_mut!((*ctx_raw).host_call).write(host_trampoline);\n\
                 }\n\
                 let mut ctx: Box<Ctx> = unsafe { Box::from_raw(ctx_raw) };\n\
                 let init_res = run_with_wasm_mxcsr::<(), R<()>>(&mut ctx, mem.as_mut_slice(), (), |ctx, mb, ()| initialize(ctx, mb), true);\n\
                 if let Err(t) = init_res {\n\
                     unsafe { drop(Box::from_raw(bridge)); }\n\
                     write_trap(trap_out, t);\n\
                     return std::ptr::null_mut();\n\
                 }\n\
                 Box::into_raw(Box::new(BlotHandle {\n\
                     in_call: std::cell::Cell::new(false),\n\
                     ctx: std::cell::UnsafeCell::new(ctx),\n\
                     mem: std::cell::UnsafeCell::new(mem),\n\
                     bridge,\n\
                 }))\n\
                 }\n\n\
                 #[unsafe(no_mangle)]\n\
                 pub extern \"C\" fn blot_aot_drop(handle: *mut BlotHandle) {\n\
                     if handle.is_null() { return; }\n\
                     if unsafe { handle_in_call(handle) } { return; }\n\
                     unsafe { drop(Box::from_raw(handle)); }\n\
                 }\n\n",
        );

        // Helper for calling a single-param/single-result Blot export.
        let export_idx = |name: &str| {
            m.exports
                .iter()
                .find(|e| e.name == name && e.kind == ExternalKind::Func)
                .map(|e| e.index as usize)
        };
        let emit_fast_i32_i32 = |s: &mut String, fn_name: &str, export_name: &str, active: bool| {
            let param_str = if active {
                "active: *mut ActiveCall"
            } else {
                "handle: *mut BlotHandle"
            };
            writeln!(s, "#[unsafe(no_mangle)]\npub extern \"C\" fn {fn_name}({param_str}, arg: i32, res_out: *mut i32, trap_out: *mut u8) -> i32 {{").unwrap();
            if let Some(f) = export_idx(export_name)
                && self.types[f].params.as_slice() == [ValType::I32]
                && self.types[f].results.as_slice() == [ValType::I32]
            {
                let call = self.top_call(f, &["arg".to_string()]);
                if active {
                    writeln!(
                        s,
                        "    if active.is_null() {{ write_trap(trap_out, Trap::Host); return 2; }}\n\
                             let (ctx_ptr, mem_ptr, mem_len) = unsafe {{ ((*active).ctx, (*active).mem_ptr, (*active).mem_len) }};\n\
                             if ctx_ptr.is_null() || (mem_len > 0 && mem_ptr.is_null()) {{ write_trap(trap_out, Trap::Host); return 2; }}\n\
                             let ctx = unsafe {{ &mut *ctx_ptr }};\n\
                             let mb = unsafe {{ std::slice::from_raw_parts_mut(if mem_len == 0 {{ std::ptr::NonNull::dangling().as_ptr() }} else {{ mem_ptr }}, mem_len) }};\n\
                             if sp_here() <= ctx.stack_floor {{ write_trap(trap_out, Trap::CallStackExhausted); return 1; }}\n\
                             match run_with_wasm_mxcsr::<i32, R<i32>>(ctx, mb, arg, |ctx, mb, arg| {call}, false) {{\n\
                                 Ok(v) => {{ if !res_out.is_null() {{ unsafe {{ *res_out = v; }} }} 0 }}\n\
                                 Err(Trap::Host) => {{ write_trap(trap_out, Trap::Host); 2 }}\n\
                                 Err(t) => {{ write_trap(trap_out, t); 1 }}\n\
                             }}\n}}\n"
                    )
                    .unwrap();
                } else {
                    writeln!(
                        s,
                        "    if handle.is_null() || unsafe {{ handle_in_call(handle) }} {{ write_trap(trap_out, Trap::Host); return 2; }}\n\
                             let _guard = unsafe {{ InCallGuard::enter(handle) }};\n\
                             let ctx: &mut Ctx = unsafe {{ &mut **(*std::ptr::addr_of!((*handle).ctx)).get() }};\n\
                             let mem = unsafe {{ &mut *(*std::ptr::addr_of!((*handle).mem)).get() }};\n\
                             ctx.stack_floor = blot_stack_floor();\n\
                             if sp_here() <= ctx.stack_floor {{ write_trap(trap_out, Trap::CallStackExhausted); return 1; }}\n\
                             let res = run_with_wasm_mxcsr::<i32, R<i32>>(ctx, mem.as_mut_slice(), arg, |ctx, mb, arg| {call}, true);\n\
                             match res {{\n\
                                 Ok(v) => {{ if !res_out.is_null() {{ unsafe {{ *res_out = v; }} }} 0 }}\n\
                                 Err(Trap::Host) => {{ write_trap(trap_out, Trap::Host); 2 }}\n\
                                 Err(t) => {{ write_trap(trap_out, t); 1 }}\n\
                             }}\n}}\n"
                    )
                    .unwrap();
                }
            } else {
                let param_name = if active { "active" } else { "handle" };
                writeln!(
                    s,
                    "    let _ = ({param_name}, arg, res_out, trap_out); 3\n}}\n"
                )
                .unwrap();
            }
        };
        emit_fast_i32_i32(&mut s, "blot_aot_call_allocate", "blot:allocate", false);
        emit_fast_i32_i32(&mut s, "blot_aot_call_reset", "blot:reset", false);
        emit_fast_i32_i32(&mut s, "blot_aot_active_allocate", "blot:allocate", true);
        emit_fast_i32_i32(&mut s, "blot_aot_active_reset", "blot:reset", true);

        // blot_aot_call_finish_invocation
        s.push_str("#[unsafe(no_mangle)]\npub extern \"C\" fn blot_aot_call_finish_invocation(handle: *mut BlotHandle, res_out: *mut i32, trap_out: *mut u8) -> i32 {\n");
        if let Some(f) = export_idx("blot:finish_invocation")
            && self.types[f].params.is_empty()
            && self.types[f].results.as_slice() == [ValType::I32]
        {
            let call = self.top_call(f, &[]);
            writeln!(
                s,
                "    if handle.is_null() || unsafe {{ handle_in_call(handle) }} {{ write_trap(trap_out, Trap::Host); return 2; }}\n\
                     let _guard = unsafe {{ InCallGuard::enter(handle) }};\n\
                     let ctx: &mut Ctx = unsafe {{ &mut **(*std::ptr::addr_of!((*handle).ctx)).get() }};\n\
                     let mem = unsafe {{ &mut *(*std::ptr::addr_of!((*handle).mem)).get() }};\n\
                     ctx.stack_floor = blot_stack_floor();\n\
                     if sp_here() <= ctx.stack_floor {{ write_trap(trap_out, Trap::CallStackExhausted); return 1; }}\n\
                     let res = run_with_wasm_mxcsr::<(), R<i32>>(ctx, mem.as_mut_slice(), (), |ctx, mb, ()| {call}, true);\n\
                     match res {{\n\
                         Ok(v) => {{ if !res_out.is_null() {{ unsafe {{ *res_out = v; }} }} 0 }}\n\
                         Err(Trap::Host) => {{ write_trap(trap_out, Trap::Host); 2 }}\n\
                         Err(t) => {{ write_trap(trap_out, t); 1 }}\n\
                     }}\n}}\n"
            )
            .unwrap();
        } else {
            s.push_str("    let _ = (handle, res_out, trap_out); 3\n}\n\n");
        }

        // General blot_aot_call_export (handles main, main_trace, ownership_enter/leave, all Blot entrypoints).
        s.push_str(
            "#[unsafe(no_mangle)]\n\
             pub extern \"C\" fn blot_aot_call_export(handle: *mut BlotHandle, name_ptr: *const u8, name_len: usize, arg_bits: u64, res_out: *mut u64, trap_out: *mut u8) -> i32 {\n\
                 if handle.is_null() { return 3; }\n\
                 if unsafe { handle_in_call(handle) } { write_trap(trap_out, Trap::Host); return 2; }\n\
                 let Some(name) = read_ffi_name(name_ptr, name_len) else { return 3; };\n\
                 let _guard = unsafe { InCallGuard::enter(handle) } ;\n\
                 let ctx: &mut Ctx = unsafe { &mut **(*std::ptr::addr_of!((*handle).ctx)).get() };\n\
                 let mem = unsafe { &mut *(*std::ptr::addr_of!((*handle).mem)).get() };\n\
                 ctx.stack_floor = blot_stack_floor();\n\
                 let res: R<u64> = match name {\n",
        );
        for exp in &m.exports {
            if exp.kind != ExternalKind::Func {
                continue;
            }
            let f = exp.index as usize;
            let ty = &self.types[f];
            let arg_expr: Option<Vec<String>> = match ty.params.as_slice() {
                [] => Some(vec![]),
                [ValType::I32] => Some(vec!["(arg_bits as u32 as i32)".into()]),
                [ValType::I64] => Some(vec!["(arg_bits as i64)".into()]),
                [ValType::F32] => Some(vec!["f32::from_bits(arg_bits as u32)".into()]),
                [ValType::F64] => Some(vec!["f64::from_bits(arg_bits)".into()]),
                [ValType::Ref(_)] => Some(vec!["arg_bits".into()]),
                _ => None,
            };
            let ret_map: Option<&'static str> = match ty.results.as_slice() {
                [] => Some(".map(|()| 0u64)"),
                [ValType::I32] => Some(".map(|v| v as u32 as u64)"),
                [ValType::I64] => Some(".map(|v| v as u64)"),
                [ValType::F32] => Some(".map(|v| v.to_bits() as u64)"),
                [ValType::F64] => Some(".map(|v| v.to_bits())"),
                [ValType::Ref(_)] => Some(""),
                _ => None,
            };
            if let (Some(args), Some(rmap)) = (arg_expr, ret_map) {
                let call = self.top_call(f, &args);
                writeln!(
                    s,
                    "        {:?} => {{\n            if sp_here() <= ctx.stack_floor {{ Err(Trap::CallStackExhausted) }} else {{\n                run_with_wasm_mxcsr::<u64, R<u64>>(ctx, mem.as_mut_slice(), arg_bits, |ctx, mb, arg_bits| {{ let _ = arg_bits; {call}{rmap} }}, true)\n            }}\n        }},",
                    exp.name
                )
                .unwrap();
            }
        }
        s.push_str(
            "        _ => return 3,\n\
                 };\n\
                 match res {\n\
                     Ok(v) => { if !res_out.is_null() { unsafe { *res_out = v; } } 0 }\n\
                     Err(Trap::Host) => { write_trap(trap_out, Trap::Host); 2 }\n\
                     Err(t) => { write_trap(trap_out, t); 1 }\n\
                 }\n\
             }\n\n",
        );

        // Memory access & grow.
        s.push_str(
            "#[unsafe(no_mangle)]\n\
             pub extern \"C\" fn blot_aot_mem_data(handle: *mut BlotHandle, ptr_out: *mut *mut u8, len_out: *mut usize) {\n\
                 if handle.is_null() || unsafe { handle_in_call(handle) } {\n\
                     unsafe {\n\
                         if !ptr_out.is_null() { *ptr_out = std::ptr::null_mut(); }\n\
                         if !len_out.is_null() { *len_out = 0; }\n\
                     }\n\
                     return;\n\
                 }\n\
                 let ctx: &Ctx = unsafe { &**(*std::ptr::addr_of!((*handle).ctx)).get() };\n\
                 let mem = unsafe { &*(*std::ptr::addr_of!((*handle).mem)).get() };\n\
                 unsafe {\n\
                     if !ptr_out.is_null() { *ptr_out = mem.as_mut_ptr(); }\n\
                     if !len_out.is_null() { *len_out = ctx.mem.len; }\n\
                 }\n\
             }\n\n\
             #[unsafe(no_mangle)]\n\
             pub extern \"C\" fn blot_aot_active_mem_data(active: *mut ActiveCall, ptr_out: *mut *mut u8, len_out: *mut usize) {\n\
                 if active.is_null() {\n\
                     unsafe {\n\
                         if !ptr_out.is_null() { *ptr_out = std::ptr::null_mut(); }\n\
                         if !len_out.is_null() { *len_out = 0; }\n\
                     }\n\
                     return;\n\
                 }\n\
                 let (ctx_ptr, mem_ptr, mem_len) = unsafe { ((*active).ctx, (*active).mem_ptr, (*active).mem_len) };\n\
                 if ctx_ptr.is_null() || (mem_len > 0 && mem_ptr.is_null()) {\n\
                     unsafe {\n\
                         if !ptr_out.is_null() { *ptr_out = std::ptr::null_mut(); }\n\
                         if !len_out.is_null() { *len_out = 0; }\n\
                     }\n\
                     return;\n\
                 }\n\
                 let ctx = unsafe { &*ctx_ptr };\n\
                 unsafe {\n\
                     if !ptr_out.is_null() { *ptr_out = if mem_len == 0 { std::ptr::null_mut() } else { mem_ptr }; }\n\
                     if !len_out.is_null() { *len_out = ctx.mem.len; }\n\
                 }\n\
             }\n\n\
             #[unsafe(no_mangle)]\n\
             pub extern \"C\" fn blot_aot_mem_grow(handle: *mut BlotHandle, delta_pages: u64) -> i64 {\n\
                 if handle.is_null() || unsafe { handle_in_call(handle) } { return -1; }\n\
                 let ctx: &mut Ctx = unsafe { &mut **(*std::ptr::addr_of!((*handle).ctx)).get() };\n\
                 ctx.mem.grow(delta_pages)\n\
             }\n\n",
        );

        // Exported globals read/write.
        s.push_str(
            "fn read_global(ctx: &Ctx, name: &str) -> Option<(u64, u8)> {\n    match name {\n",
        );
        for (name, gi, vt, _) in &self.exported_globals {
            let (kind, expr) = match (vt, self.global_consts[*gi]) {
                (ValType::I32, Some(ConstVal::I32(v))) => (0u8, format!("({v}i32 as u32 as u64)")),
                (ValType::I64, Some(ConstVal::I64(v))) => (1u8, format!("({v}i64 as u64)")),
                (ValType::F32, Some(ConstVal::F32(bits))) => (2u8, format!("({bits}u32 as u64)")),
                (ValType::F64, Some(ConstVal::F64(bits))) => (3u8, format!("{bits}u64")),
                (ValType::I32, None) => (0u8, format!("(ctx.g{gi} as u32 as u64)")),
                (ValType::I64, None) => (1u8, format!("(ctx.g{gi} as u64)")),
                (ValType::F32, None) => (2u8, format!("(ctx.g{gi}.to_bits() as u64)")),
                (ValType::F64, None) => (3u8, format!("ctx.g{gi}.to_bits()")),
                (ValType::Ref(_), None) => (4u8, format!("ctx.g{gi}")),
                _ => continue,
            };
            writeln!(s, "        {name:?} => Some(({expr}, {kind})),").unwrap();
        }
        s.push_str(
            "        _ => None,\n\
                 }\n\
             }\n\n\
             #[unsafe(no_mangle)]\n\
             pub extern \"C\" fn blot_aot_get_global(handle: *mut BlotHandle, name_ptr: *const u8, name_len: usize, val_out: *mut u64, kind_out: *mut u8) -> i32 {\n\
                 if handle.is_null() || unsafe { handle_in_call(handle) } { return 1; }\n\
                 let Some(name) = read_ffi_name(name_ptr, name_len) else { return 1; };\n\
                 let ctx: &Ctx = unsafe { &**(*std::ptr::addr_of!((*handle).ctx)).get() };\n\
                 let Some((v, k)) = read_global(ctx, name) else { return 1; };\n\
                 unsafe {\n\
                     if !val_out.is_null() { *val_out = v; }\n\
                     if !kind_out.is_null() { *kind_out = k; }\n\
                 }\n\
                 0\n\
             }\n\n\
             #[unsafe(no_mangle)]\n\
             pub extern \"C\" fn blot_aot_active_get_global(active: *mut ActiveCall, name_ptr: *const u8, name_len: usize, val_out: *mut u64, kind_out: *mut u8) -> i32 {\n\
                 if active.is_null() { return 1; }\n\
                 let ctx_ptr = unsafe { (*active).ctx };\n\
                 if ctx_ptr.is_null() { return 1; }\n\
                 let Some(name) = read_ffi_name(name_ptr, name_len) else { return 1; };\n\
                 let ctx = unsafe { &*ctx_ptr };\n\
                 let Some((v, k)) = read_global(ctx, name) else { return 1; };\n\
                 unsafe {\n\
                     if !val_out.is_null() { *val_out = v; }\n\
                     if !kind_out.is_null() { *kind_out = k; }\n\
                 }\n\
                 0\n\
             }\n\n\
             #[unsafe(no_mangle)]\n\
             pub extern \"C\" fn blot_aot_set_global(handle: *mut BlotHandle, name_ptr: *const u8, name_len: usize, val: u64) -> i32 {\n\
                 if handle.is_null() || unsafe { handle_in_call(handle) } { return 1; }\n\
                 let Some(name) = read_ffi_name(name_ptr, name_len) else { return 1; };\n\
                 let ctx: &mut Ctx = unsafe { &mut **(*std::ptr::addr_of!((*handle).ctx)).get() };\n\
                 match name {\n",
        );
        for (name, gi, vt, mutable) in &self.exported_globals {
            if !*mutable || self.global_consts[*gi].is_some() {
                continue;
            }
            let assign = match vt {
                ValType::I32 => format!("ctx.g{gi} = val as u32 as i32;"),
                ValType::I64 => format!("ctx.g{gi} = val as i64;"),
                ValType::F32 => format!("ctx.g{gi} = f32::from_bits(val as u32);"),
                ValType::F64 => format!("ctx.g{gi} = f64::from_bits(val);"),
                ValType::Ref(_) => format!("ctx.g{gi} = val;"),
                _ => continue,
            };
            writeln!(s, "        {name:?} => {{ {assign} 0 }}").unwrap();
        }
        s.push_str("        _ => 1,\n    }\n}\n");
        Ok(s)
    }

    /// The defined functions: (index, source).
    fn functions(
        &self,
        m: &DecodedModule,
        deadline: Option<std::time::Instant>,
    ) -> Res<Vec<(usize, String)>> {
        let base = self.imports.len();
        let mut functions = Vec::new();
        let locals: usize = m.code.iter().map(|body| body.locals.len()).sum();
        if locals > MAX_LOCALS {
            return Err(format!(
                "the module declares {locals} locals, more than {MAX_LOCALS}"
            ));
        }
        let mut total = 0;
        for (k, body) in m.code.iter().enumerate() {
            if let Some(dl) = deadline
                && (k & 63) == 0
                && std::time::Instant::now() >= dl
            {
                return Err("rust codegen function emission: exceeded build time limit".into());
            }
            let index = base + k;
            let mut s = String::new();
            let f = Function::new(self, index, &body.locals)?;
            f.emit(&body.instructions, &mut s, MAX_SOURCE_BYTES - total)
                .map_err(|e| format!("function {index}: {e}"))?;
            total += s.len();
            if total > MAX_SOURCE_BYTES {
                return Err(format!(
                    "the generated source exceeds {MAX_SOURCE_BYTES} bytes"
                ));
            }
            functions.push((index, s));
        }
        Ok(functions)
    }
}

/// `line` without the parentheses that enclose a whole value: an assigned
/// or initializing value, an argument, a tuple element or an `if`
/// condition. Operands are parenthesized when they are pushed, which keeps
/// operator precedence right; where an operand stands alone the
/// parentheses are noise, and rustc's `unused_parens` lint spends time on
/// each one. Tuples and `()` keep theirs.
fn bare_operands(line: &str) -> String {
    let b = line.as_bytes();
    if !line.is_ascii() || line.trim_start().starts_with("//") {
        return line.to_string();
    }
    // The matching close of each open parenthesis, and whether the group
    // has a comma at its own level.
    let mut close = vec![usize::MAX; b.len()];
    let mut comma = vec![false; b.len()];
    let mut open = Vec::new();
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'(' => open.push(i),
            b')' => {
                let Some(o) = open.pop() else {
                    return line.to_string();
                };
                close[o] = i;
            }
            b',' => {
                if let Some(&o) = open.last() {
                    comma[o] = true;
                }
            }
            // Generated code has no char literals: `'` only starts labels.
            b'"' => return line.to_string(),
            _ => {}
        }
    }
    if !open.is_empty() {
        return line.to_string();
    }
    let mut drop = vec![false; b.len()];
    for o in 0..b.len() {
        let c = close[o];
        if c == usize::MAX || c == o + 1 || comma[o] {
            continue;
        }
        let before = &line[..o];
        let after = &line[c + 1..];
        // An assignment's `=`, not a comparison's.
        let assigned =
            before.ends_with("= ") && !before[..before.len() - 2].ends_with(['=', '!', '<', '>']);
        let condition = before.ends_with("if ")
            && !before[..before.len() - 3]
                .ends_with(|c: char| c.is_ascii_alphanumeric() || c == '_');
        let alone_before = assigned || before.ends_with('(') || before.ends_with(", ") || condition;
        let alone_after = after.starts_with(';')
            || after.starts_with(')')
            || after.starts_with(',')
            || after.starts_with(" {");
        // `= (a)` before ` {` would be a block, not a condition.
        if alone_before && alone_after && (condition || !after.starts_with(" {")) {
            drop[o] = true;
            drop[c] = true;
        }
    }
    line.bytes()
        .enumerate()
        .filter(|(i, _)| !drop[*i])
        .map(|(_, c)| c as char)
        .collect()
}

/// A constant expression as Rust: numeric constants, global.get and the
/// extended constant operators (wrapping integer add, sub and mul).
/// Earlier globals are referred to by name (`{prefix}g{k}`), or by their
/// evaluated constant literal when effectively immutable.
fn constant(
    expression: &[Instruction],
    globals: usize,
    prefix: &str,
    consts: &[Option<ConstVal>],
) -> Res<String> {
    let mut stack: Vec<String> = Vec::new();
    for i in expression {
        let value = match (i.opcode, &i.immediate) {
            (0x0b, _) => break,
            (0x41, Immediate::I32(v)) => format!("({v}i32)"),
            (0x42, Immediate::I64(v)) => format!("({v}i64)"),
            (0x43, Immediate::F32(v)) => format!("f32::from_bits({v:#x})"),
            (0x44, Immediate::F64(v)) => format!("f64::from_bits({v:#x})"),
            (0xd0, _) => "0u64".to_string(),
            (0x23, Immediate::Index(g)) if (*g as usize) < globals => {
                if let Some(c) = consts.get(*g as usize).and_then(|c| *c) {
                    c.rust_literal()
                } else {
                    format!("{prefix}g{g}")
                }
            }
            (0x23, _) => return Err("global.get of a later global".into()),
            (op @ (0x6a..=0x6c | 0x7c..=0x7e), _) => {
                let (Some(b), Some(a)) = (stack.pop(), stack.pop()) else {
                    return Err("constant expression operator without operands".into());
                };
                let method = match op {
                    0x6a | 0x7c => "wrapping_add",
                    0x6b | 0x7d => "wrapping_sub",
                    _ => "wrapping_mul",
                };
                format!("{a}.{method}({b})")
            }
            (op, _) => return Err(format!("unsupported constant instruction {op:#x}")),
        };
        stack.push(value);
    }
    match <[String; 1]>::try_from(stack) {
        Ok([value]) => Ok(value),
        Err(_) => Err("constant expression does not leave one value".into()),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Function,
    Block,
    Loop,
    If,
}

struct Frame {
    kind: Kind,
    label: usize,
    results: Vec<ValType>,
    /// Stack height below the frame's params.
    height: usize,
    unreachable: bool,
    /// Variables that hold the frame's results (block, if) or params (loop).
    result_vars: Vec<String>,
    loop_vars: Vec<String>,
    /// The params as the if found them, for its else.
    if_params: Vec<(String, ValType)>,
    has_else: bool,
    /// Conservative dirty-global sets for sound `ctx.g{x}` spill tracking.
    if_entry_dirty: BitSet,
    end_dirty: BitSet,
}

struct Function<'a> {
    module: &'a Module,
    index: usize,
    locals: Vec<ValType>,
    n_params: usize,
}

struct Emitter {
    s: String,
    next: usize,
    ret_abi: RetAbi,
    /// The operand stack: each value's Rust expression and type. An
    /// expression is a variable, a literal, or a pure expression (no trap,
    /// no side effect) whose reads are in `deps` at the same position.
    stack: Vec<(String, ValType)>,
    deps: Vec<Dep>,
    frames: Vec<Frame>,
    /// Nesting of blocks inside dead code being skipped.
    dead: usize,
    /// Stack positions that held pending expressions when pushed, in
    /// increasing order; some may since have been evaluated or popped.
    /// `materialize` visits these instead of the whole stack.
    pending: Vec<usize>,
    /// The globals the function sets (excluding register-passed preserved
    /// globals), which it stores back to `ctx` before calls that read/write
    /// them; `globals_return_store` omits globals proven preserved when no
    /// callee inside this function reads or writes them.
    globals_set: Vec<u32>,
    globals_return_store: Vec<u32>,
    globals_reloaded: Vec<u32>,
    /// The globals held in locals (including register-passed preserved globals).
    globals_cached: std::collections::BTreeSet<u32>,
    /// Cached `globals_set` entries whose local `gv{x}` may currently differ
    /// from `ctx.g{x}` along some reachable path.
    dirty_globals: BitSet,
    n_globals: usize,
    /// Preserved globals passed by value into this function.
    reg_globals: Vec<u32>,
    /// Whether this function keeps `ml: usize` (the active direct memory length)
    /// and (in chunked 64-bit mode) `mf: bool` (`ctx.outer.is_flat()`).
    uses_ml: bool,
    /// Inside an `if mf { ... }` non-growing region of a chunked 64-bit function:
    /// all memory ops emit flat `m64::fl*`/`fs*`/`fcopy_n`/`ffill_n`.
    in_flat_region: bool,
    /// The bytes this function's body may take, and whether it went past:
    /// checked as lines are written, so that no module makes the generator
    /// build an oversized string before the total is checked.
    budget: usize,
    over: bool,
}

/// What a pending pure expression reads, and so what must not change before
/// it is evaluated.
#[derive(Clone, Default)]
struct Dep {
    locals: Vec<u32>,
    globals: bool,
    /// Not a variable or a literal: evaluated where it is used.
    pending: bool,
}

impl Dep {
    fn local(x: u32) -> Dep {
        Dep {
            locals: vec![x],
            globals: false,
            pending: true,
        }
    }
    fn global() -> Dep {
        Dep {
            locals: vec![],
            globals: true,
            pending: true,
        }
    }
    fn union(mut self, other: &Dep) -> Dep {
        for x in &other.locals {
            if !self.locals.contains(x) {
                self.locals.push(*x);
            }
        }
        self.globals |= other.globals;
        self.pending = true;
        self
    }
}

/// Expressions longer than this are bound to a variable.
const MAX_EXPRESSION: usize = 300;

impl Emitter {
    /// Stores modified dirty cached globals back to `ctx` before returning,
    /// omitting globals proven preserved whose `ctx` slot was never touched.
    fn store_globals_return(&mut self) {
        for x in self.globals_return_store.clone() {
            if self.dirty_globals.contains(x) {
                self.line(&format!("ctx.g{x} = gv{x};"));
            }
        }
    }

    /// Stores only the dirty set cached globals that `callee` transitively
    /// reads or writes, marking them clean once flushed.
    fn store_globals_for_call(&mut self, module: &Module, callee: usize) {
        for x in self.globals_set.clone() {
            if self.dirty_globals.contains(x)
                && (module.fn_reads[callee].contains(x) || module.fn_writes[callee].contains(x))
            {
                self.line(&format!("ctx.g{x} = gv{x};"));
                self.dirty_globals.remove(x);
            }
        }
        // Host imports that can re-enter `goose.reserve` read preserved globals
        // from `ctx` inside `reserve(...)`.
        if module.reserve_imports[callee] {
            for g in self.reg_globals.clone() {
                if module.fn_reads[callee].contains(g) || module.fn_writes[callee].contains(g) {
                    self.line(&format!("ctx.g{g} = gv{g};"));
                }
            }
        }
    }

    /// Reloads only the cached mutable globals that `callee` may net-modify
    /// (skipping globals `callee` is proven to preserve).
    fn load_globals_after_call(&mut self, module: &Module, callee: usize) {
        for x in self.globals_reloaded.clone() {
            if module.fn_net_writes[callee].contains(x) {
                self.line(&format!("gv{x} = ctx.g{x};"));
                self.dirty_globals.remove(x);
            }
        }
    }

    /// Stores dirty cached globals that any target of `ci_id` may read or write.
    fn store_globals_for_ci(&mut self, module: &Module, ci_id: usize) {
        for x in self.globals_set.clone() {
            if self.dirty_globals.contains(x)
                && (module.ci_reads[ci_id].contains(x) || module.ci_writes[ci_id].contains(x))
            {
                self.line(&format!("ctx.g{x} = gv{x};"));
                self.dirty_globals.remove(x);
            }
        }
        if module.ci_targets[ci_id]
            .iter()
            .any(|&(_, f)| module.reserve_imports[f])
        {
            for g in self.reg_globals.clone() {
                if module.ci_reads[ci_id].contains(g) || module.ci_writes[ci_id].contains(g) {
                    self.line(&format!("ctx.g{g} = gv{g};"));
                }
            }
        }
    }

    /// Reloads cached globals that any target of `ci_id` may net-modify.
    fn load_globals_after_ci(&mut self, module: &Module, ci_id: usize) {
        for x in self.globals_reloaded.clone() {
            if module.ci_net_writes[ci_id].contains(x) {
                self.line(&format!("gv{x} = ctx.g{x};"));
                self.dirty_globals.remove(x);
            }
        }
    }

    fn fresh(&mut self, prefix: &str) -> String {
        self.next += 1;
        format!("{prefix}{}", self.next)
    }

    /// Refreshes `ml` (and `mf` in chunked 64-bit mode) after a call that may
    /// grow memory or after `memory.grow`, re-establishing LLVM's dominating
    /// `MIN_BYTES <= ml <= cap` range proof.
    fn refresh_ml(&mut self, module: &Module) -> Res<()> {
        if !self.uses_ml {
            return Ok(());
        }
        let oob = ret_err_expr(self.ret_abi, "Trap::OutOfBounds")?;
        if module.memory64 {
            if module.fixed64.is_none() {
                self.line("ml = ctx.outer.fast();");
                if !module.flat64 {
                    self.line("mf = ctx.outer.is_flat();");
                    self.line(&format!("if ml > mb.len() {{ return {oob}; }}"));
                } else if module.cap64.is_some() {
                    if module.min_bytes > 0 {
                        self.line(&format!(
                            "if ml < MIN_BYTES || ml > CAP64 {{ return {oob}; }}"
                        ));
                    } else {
                        self.line(&format!("if ml > CAP64 {{ return {oob}; }}"));
                    }
                } else if module.min_bytes > 0 {
                    self.line(&format!(
                        "if ml < MIN_BYTES || ml > mb.len() {{ return {oob}; }}"
                    ));
                } else {
                    self.line(&format!("if ml > mb.len() {{ return {oob}; }}"));
                }
            }
        } else if module.fixed32.is_none() {
            self.line("ml = ctx.mem.len;");
            let max_expr = if module.target == Target::BlotCdylib {
                "mb.len()"
            } else {
                "CAP32"
            };
            if module.min_bytes > 0 {
                self.line(&format!(
                    "if ml < MIN_BYTES || ml > {max_expr} {{ return {oob}; }}"
                ));
            } else {
                self.line(&format!("if ml > {max_expr} {{ return {oob}; }}"));
            }
        }
        Ok(())
    }

    fn line(&mut self, text: &str) {
        if self.over || self.s.len() + text.len() + 65 > self.budget {
            self.over = true;
            return;
        }
        // Indentation stops deepening at 16 levels: deeply nested blocks
        // would otherwise make the source grow with the square of the nesting.
        let depth = self.frames.len().min(16);
        for _ in 0..depth {
            self.s.push_str("    ");
        }
        self.s.push_str(text);
        self.s.push('\n');
    }

    fn pop(&mut self) -> Res<(String, ValType)> {
        self.pop_dep().map(|(v, t, _)| (v, t))
    }

    fn pop_dep(&mut self) -> Res<(String, ValType, Dep)> {
        let (v, t) = self
            .stack
            .pop()
            .ok_or_else(|| "operand stack underflow".to_string())?;
        let dep = self.deps.pop().unwrap_or_default();
        Ok((v, t, dep))
    }

    /// Pushes `expr` of type `t` as a new variable, evaluated here.
    fn push(&mut self, t: ValType, expr: &str) -> Res<()> {
        let v = self.fresh("v");
        let ty = rust_type(t)?;
        self.line(&format!("let {v}: {ty} = {expr};"));
        self.stack.push((v, t));
        self.deps.push(Dep::default());
        Ok(())
    }

    /// Pushes a variable or literal.
    fn push_plain(&mut self, t: ValType, value: String) {
        self.stack.push((value, t));
        self.deps.push(Dep::default());
    }

    /// Pushes a pure expression reading `dep`, evaluated where it is used.
    fn push_pure(&mut self, t: ValType, expr: &str, dep: Dep) -> Res<()> {
        if expr.len() > MAX_EXPRESSION {
            return self.push(t, expr);
        }
        // At most MAX_PENDING pending expressions: past that the oldest are
        // evaluated into variables now (they are pure, so evaluating them
        // early changes nothing), which keeps `materialize` cheap.
        let at = self.stack.len();
        while self.pending.last().is_some_and(|&k| k >= at) {
            self.pending.pop();
        }
        if self.pending.len() >= MAX_PENDING {
            let deps = &self.deps;
            self.pending.retain(|&k| deps[k].pending);
            let excess = self.pending.len().saturating_sub(MAX_PENDING / 2);
            let oldest: Vec<usize> = self.pending[..excess].to_vec();
            self.materialize(|_| true, Some(&oldest))?;
        }
        self.stack.push((format!("({expr})"), t));
        self.pending.push(at);
        self.deps.push(Dep {
            pending: true,
            ..dep
        });
        Ok(())
    }

    /// Evaluates the pending expressions that `reads` into variables, in
    /// stack order.
    fn materialize(&mut self, reads: impl Fn(&Dep) -> bool, only: Option<&[usize]>) -> Res<()> {
        let len = self.stack.len();
        self.pending.retain(|&k| k < len);
        let pending = std::mem::take(&mut self.pending);
        let mut still = Vec::with_capacity(pending.len());
        for k in pending {
            if !self.deps[k].pending {
                continue;
            }
            if !reads(&self.deps[k]) || only.is_some_and(|only| !only.contains(&k)) {
                still.push(k);
                continue;
            }
            {
                let (expr, t) = self.stack[k].clone();
                let v = self.fresh("v");
                self.line(&format!("let {v}: {} = {expr};", rust_type(t)?));
                self.stack[k].0 = v;
                self.deps[k] = Dep::default();
            }
        }
        self.pending = still;
        Ok(())
    }

    fn top(&self, n: usize) -> Res<Vec<String>> {
        if self.stack.len() < n {
            return Err("operand stack underflow".into());
        }
        Ok(self.stack[self.stack.len() - n..]
            .iter()
            .map(|(v, _)| v.clone())
            .collect())
    }

    /// Assigns `values` to `vars`, all at once.
    fn assign(&mut self, vars: &[String], values: &[String]) {
        match vars.len() {
            0 => {}
            1 => self.line(&format!("{} = {};", vars[0], values[0])),
            _ => {
                self.line(&format!("let t = ({});", values.join(", ")));
                for (k, v) in vars.iter().enumerate() {
                    self.line(&format!("{v} = t.{k};"));
                }
            }
        }
    }

    /// A branch to the frame `depth` levels out, taking its values from
    /// the top of the stack.
    fn branch(&mut self, depth: usize) -> Res<()> {
        let at = self
            .frames
            .len()
            .checked_sub(depth + 1)
            .ok_or("branch depth")?;
        let (kind, label) = (self.frames[at].kind, self.frames[at].label);
        match kind {
            Kind::Function => {
                let values = self.top(self.frames[at].results.len())?;
                self.store_globals_return();
                self.line(&format!(
                    "return {};",
                    ok_return_expr(self.ret_abi, &values)
                ));
            }
            Kind::Loop => {
                let vars = self.frames[at].loop_vars.clone();
                let values = self.top(vars.len())?;
                self.assign(&vars, &values);
                self.line(&format!("continue 'L{label};"));
            }
            Kind::Block | Kind::If => {
                let dirty = self.dirty_globals.clone();
                self.frames[at].end_dirty.union_with(&dirty);
                let vars = self.frames[at].result_vars.clone();
                let values = self.top(vars.len())?;
                self.assign(&vars, &values);
                self.line(&format!("break 'L{label};"));
            }
        }
        Ok(())
    }

    fn set_unreachable(&mut self) {
        if let Some(frame) = self.frames.last_mut() {
            frame.unreachable = true;
        }
    }
}

impl<'a> Function<'a> {
    fn new(module: &'a Module, index: usize, declared: &[ValType]) -> Res<Function<'a>> {
        let ty = &module.types[index];
        let mut locals = ty.params.clone();
        locals.extend_from_slice(declared);
        Ok(Function {
            module,
            index,
            locals,
            n_params: ty.params.len(),
        })
    }

    fn block_type(&self, immediate: &Immediate) -> Res<(Vec<ValType>, Vec<ValType>)> {
        Ok(match immediate {
            Immediate::Block(-64) => (vec![], vec![]),
            Immediate::Block(-1) => (vec![], vec![ValType::I32]),
            Immediate::Block(-2) => (vec![], vec![ValType::I64]),
            Immediate::Block(-3) => (vec![], vec![ValType::F32]),
            Immediate::Block(-4) => (vec![], vec![ValType::F64]),
            Immediate::Block(-5) => (vec![], vec![ValType::V128]),
            Immediate::Block(-16) => (vec![], vec![ValType::FUNCREF]),
            Immediate::Block(-17) => (vec![], vec![ValType::EXTERNREF]),
            Immediate::BlockType(t) => (vec![], vec![*t]),
            Immediate::Block(n) if *n >= 0 => {
                let t = self.module_type(*n as usize)?;
                (t.params.clone(), t.results.clone())
            }
            _ => return Err("unsupported block type".into()),
        })
    }

    fn module_type(&self, n: usize) -> Res<&FuncType> {
        self.module
            .module_types
            .get(n)
            .ok_or_else(|| "block type index out of range".into())
    }

    /// The length a memory access checks against: the constant of a fixed
    /// memory, otherwise the local `ml` kept by the prologue and `refresh_ml`.
    fn ml(&self) -> String {
        if self.module.memory64 {
            match self.module.fixed64 {
                Some(bytes) => format!("{bytes}usize"),
                None => "ml".into(),
            }
        } else {
            match self.module.fixed32 {
                Some(bytes) => format!("{bytes}usize"),
                None => "ml".into(),
            }
        }
    }

    fn address(&self, v: &str) -> String {
        if self.module.memory64 {
            format!("({v} as u64)")
        } else {
            format!("({v} as u32 as u64)")
        }
    }

    fn emit(&self, code: &[Instruction], out: &mut String, budget: usize) -> Res<()> {
        let ty = &self.module.types[self.index];
        let fn_abi = ret_abi(&ty.results);
        let fn_reg_globals = self.module.fn_reg_globals(self.index);
        let reg_set: std::collections::BTreeSet<u32> = fn_reg_globals.iter().copied().collect();
        let n_globals = self.module.globals.len();

        // Collect non-constant, non-register-passed globals used in live code,
        // per-global spill/reload call-site counts (for ROI filtering), and
        // globals touched by any reachable non-noreturn callee.
        let mut uses: std::collections::BTreeMap<u32, (usize, usize)> =
            std::collections::BTreeMap::new();
        let mut callee_touches = BitSet::new(n_globals);
        let mut call_targets: Vec<usize> = Vec::new();
        let mut ci_calls: Vec<usize> = Vec::new();
        let mut sites = 1usize;
        let mut uses_ml = false;
        let is_fixed = if self.module.memory64 {
            self.module.fixed64.is_some()
        } else {
            self.module.fixed32.is_some()
        };
        visit_live(code, &self.module.noreturn_imports, |i| {
            match (i.opcode, &i.immediate) {
                (0x23 | 0x24, Immediate::Index(x))
                    if (*x as usize) < n_globals
                        && self.module.global_consts[*x as usize].is_none()
                        && !reg_set.contains(x) =>
                {
                    let entry = uses.entry(*x).or_default();
                    if i.opcode == 0x23 {
                        entry.0 += 1;
                    } else {
                        entry.1 += 1;
                    }
                }
                (0x0c..=0x0f, _) => sites += 1,
                (0x10, Immediate::Index(c))
                    if !self
                        .module
                        .noreturn_imports
                        .get(*c as usize)
                        .copied()
                        .unwrap_or(false) =>
                {
                    sites += 1;
                    let c = *c as usize;
                    call_targets.push(c);
                    if let Some(r) = self.module.fn_reads.get(c) {
                        callee_touches.union_with(r);
                    }
                    if let Some(w) = self.module.fn_writes.get(c) {
                        callee_touches.union_with(w);
                    }
                }
                (0x11, Immediate::Pair(ti, tbl)) => {
                    sites += 1;
                    if let Some(&ci_id) = self.module.ci_index.get(&(*tbl, *ti)) {
                        ci_calls.push(ci_id);
                        callee_touches.union_with(&self.module.ci_reads[ci_id]);
                        callee_touches.union_with(&self.module.ci_writes[ci_id]);
                    }
                }
                (0x12..=0x15, _) => sites += 1,
                (0x28..=0x3e | 0x3f | 0x40 | 0xfc0a | 0xfc0b | 0xfd00 | 0xfd0b, _) if !is_fixed => {
                    uses_ml = true;
                }
                _ => {}
            }
        });
        let mut ranked: Vec<(u32, usize, usize)> =
            uses.into_iter().map(|(x, (r, w))| (x, r, w)).collect();
        ranked.sort_by_key(|&(x, r, w)| (std::cmp::Reverse(r + w), x));
        let allowance = GLOBAL_LINES_PER_INSTRUCTION * code.len() + 64;
        let mut cost = 0;
        let mut cached = reg_set.clone();
        let mut init_cached = Vec::new();
        let mut set = Vec::new();
        let mut return_store = Vec::new();
        let mut globals_reloaded = Vec::new();
        for (x, reads, writes) in ranked {
            let is_set = writes > 0;
            let mutable = self.module.globals_mutable[x as usize];
            if mutable {
                let mut spill_sites = 0usize;
                let mut reload_sites = 0usize;
                for &c in &call_targets {
                    if is_set
                        && (self.module.fn_reads[c].contains(x)
                            || self.module.fn_writes[c].contains(x))
                    {
                        spill_sites += 1;
                    }
                    if self.module.fn_net_writes[c].contains(x) {
                        reload_sites += 1;
                    }
                }
                for &ci_id in &ci_calls {
                    if is_set
                        && (self.module.ci_reads[ci_id].contains(x)
                            || self.module.ci_writes[ci_id].contains(x))
                    {
                        spill_sites += 1;
                    }
                    if self.module.ci_net_writes[ci_id].contains(x) {
                        reload_sites += 1;
                    }
                }
                // Skip caching a mutable global when reloads/spills outweigh its uses
                // (e.g., a rarely-read scope marker reloaded after hundreds of calls).
                let total_uses = reads + writes;
                if !is_set && reads <= reload_sites + 1 {
                    continue;
                }
                if is_set
                    && spill_sites + reload_sites > total_uses * 2
                    && spill_sites + reload_sites > 2
                {
                    continue;
                }
            }
            let lines = (usize::from(is_set) + usize::from(mutable)) * sites;
            if cost + lines > allowance {
                continue;
            }
            cost += lines;
            cached.insert(x);
            init_cached.push(x);
            if is_set {
                set.push(x);
                // Skip storing `ctx.g{x} = gv{x}` on return when this function
                // preserves `x` and never lets any call observe or modify `ctx.g{x}`.
                if !(self.module.fn_preserves[self.index].contains(x)
                    && !callee_touches.contains(x))
                {
                    return_store.push(x);
                }
            }
            if mutable {
                globals_reloaded.push(x);
            }
        }
        init_cached.sort_unstable();
        set.sort_unstable();
        return_store.sort_unstable();
        globals_reloaded.sort_unstable();

        // Precompute cached set-globals written anywhere inside each `loop` body
        // so loop headers conservatively mark them dirty for backedges.
        let mut loop_written_globals: Vec<Vec<u32>> = vec![Vec::new(); code.len()];
        if !set.is_empty() {
            let mut open_loops: Vec<(usize, usize)> = Vec::new();
            let mut depth = 0usize;
            for (pc, i) in code.iter().enumerate() {
                match i.opcode {
                    0x02 | 0x04 => depth += 1,
                    0x03 => {
                        depth += 1;
                        open_loops.push((pc, depth));
                    }
                    0x0b => {
                        if open_loops.last().is_some_and(|&(_, d)| d == depth) {
                            open_loops.pop();
                        }
                        depth = depth.saturating_sub(1);
                    }
                    0x24 => {
                        if let Immediate::Index(x) = i.immediate
                            && set.binary_search(&x).is_ok()
                        {
                            for &(lpc, _) in &open_loops {
                                if !loop_written_globals[lpc].contains(&x) {
                                    loop_written_globals[lpc].push(x);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        let mut e = Emitter {
            s: String::new(),
            next: 0,
            ret_abi: fn_abi,
            stack: vec![],
            deps: vec![],
            frames: vec![],
            dead: 0,
            pending: vec![],
            globals_set: set,
            globals_return_store: return_store,
            globals_reloaded,
            globals_cached: cached,
            dirty_globals: BitSet::new(n_globals),
            n_globals,
            reg_globals: fn_reg_globals.clone(),
            uses_ml,
            in_flat_region: false,
            budget,
            over: false,
        };
        let mut all = vec![
            "ctx: &mut Ctx".to_string(),
            format!("mb: {}", self.module.mb_type()),
        ];
        let mut call_args = vec!["ctx".to_string(), "mb".to_string()];
        for &g in &fn_reg_globals {
            all.push(format!(
                "mut gv{g}: {}",
                rust_type(self.module.globals[g as usize].0)?
            ));
            call_args.push(format!("gv{g}"));
        }
        for (i, t) in ty.params.iter().enumerate() {
            all.push(format!("mut l{i}: {}", rust_type(*t)?));
            call_args.push(format!("l{i}"));
        }
        let is_root = self.module.start == Some(self.index as u32)
            || self.module.entry == Some(self.index as u32)
            || self.module.reserve == Some(self.index as u32)
            || (self.module.target == Target::BlotCdylib
                && self.module.exported_funcs.contains(&self.index));
        let need_stack_check = !self.module.fn_no_stack_check[self.index];
        // Conservative frame-size bound (locals + expression spills + call overhead).
        let frame_est = self
            .locals
            .len()
            .saturating_mul(24)
            .saturating_add(code.len().min(65_536).saturating_mul(4))
            .saturating_add(8192);
        // For larger functions (including large leaf functions), check stack headroom
        // inside a tiny wrapper BEFORE calling a `#[inline(never)]` body so LLVM
        // cannot allocate a giant stack frame prologue past the guard page before the
        // check executes.
        let need_large_stub = self.locals.len() > 64 || code.len() > 256;
        let ret_ty = fn_return_type(&ty.results)?;
        if need_large_stub {
            let stack_trap = ret_err_expr(fn_abi, "Trap::CallStackExhausted")?;
            writeln!(
                out,
                "pub fn f{}<M>({}) -> {ret_ty}\nwhere\n    Ctx: Fns<M>,\n{{\n    let frame = std::mem::MaybeUninit::<u8>::uninit();\n    let sp = frame.as_ptr() as usize;\n    if sp <= ctx.stack_floor || sp - ctx.stack_floor < {frame_est}usize {{ return {stack_trap}; }}\n    f{}_body::<M>({})\n}}\n\n#[inline(never)]\nfn f{}_body<M>({}) -> {ret_ty}\nwhere\n    Ctx: Fns<M>,\n{{",
                self.index,
                all.join(", "),
                self.index,
                call_args.join(", "),
                self.index,
                all.join(", "),
            )
            .unwrap();
        } else {
            let inline_always = self.module.fn_inline_always(self.index);
            let should_inline = !self.module.fn_in_cycle[self.index]
                && (code.len() <= INLINE_INSTRUCTIONS
                    || (!self.module.fn_has_calls[self.index]
                        && code.len() <= INLINE_LEAF_INSTRUCTIONS
                        && self.module.fn_callers[self.index] <= INLINE_LEAF_MAX_CALLERS)
                    || (!is_root
                        && self.module.fn_callers[self.index] == 1
                        && code.len() <= INLINE_SINGLE_CALLER_INSTRUCTIONS));
            if inline_always {
                writeln!(out, "#[inline(always)]").unwrap();
            } else if should_inline {
                writeln!(out, "#[inline]").unwrap();
            }
            writeln!(
                out,
                "pub fn f{}<M>({}) -> {ret_ty}\nwhere\n    Ctx: Fns<M>,\n{{",
                self.index,
                all.join(", "),
            )
            .unwrap();
            if need_stack_check {
                let stack_trap = ret_err_expr(fn_abi, "Trap::CallStackExhausted")?;
                writeln!(
                    out,
                    "    let frame = std::mem::MaybeUninit::<u8>::uninit();\n    let sp = frame.as_ptr() as usize;\n    if sp <= ctx.stack_floor || sp - ctx.stack_floor < {frame_est}usize {{ return {stack_trap}; }}"
                )
                .unwrap();
            }
        }
        if uses_ml {
            let oob = ret_err_expr(fn_abi, "Trap::OutOfBounds")?;
            if self.module.memory64 {
                writeln!(out, "    let mut ml: usize = ctx.outer.fast();").unwrap();
                if !self.module.flat64 {
                    writeln!(out, "    let mut mf: bool = ctx.outer.is_flat();").unwrap();
                    writeln!(out, "    if ml > mb.len() {{ return {oob}; }}").unwrap();
                } else if self.module.cap64.is_some() {
                    if self.module.min_bytes > 0 {
                        writeln!(
                            out,
                            "    if ml < MIN_BYTES || ml > CAP64 {{ return {oob}; }}"
                        )
                        .unwrap();
                    } else {
                        writeln!(out, "    if ml > CAP64 {{ return {oob}; }}").unwrap();
                    }
                } else if self.module.min_bytes > 0 {
                    writeln!(
                        out,
                        "    if ml < MIN_BYTES || ml > mb.len() {{ return {oob}; }}"
                    )
                    .unwrap();
                } else {
                    writeln!(out, "    if ml > mb.len() {{ return {oob}; }}").unwrap();
                }
            } else {
                writeln!(out, "    let mut ml: usize = ctx.mem.len;").unwrap();
                let max_expr = if self.module.target == Target::BlotCdylib {
                    "mb.len()"
                } else {
                    "CAP32"
                };
                if self.module.min_bytes > 0 {
                    writeln!(
                        out,
                        "    if ml < MIN_BYTES || ml > {max_expr} {{ return {oob}; }}"
                    )
                    .unwrap();
                } else {
                    writeln!(out, "    if ml > {max_expr} {{ return {oob}; }}").unwrap();
                }
            }
        }
        for (i, t) in self.locals.iter().enumerate().skip(self.n_params) {
            writeln!(out, "    let mut l{i}: {} = {};", rust_type(*t)?, zero(*t)?).unwrap();
        }
        for &x in &init_cached {
            let t = self.module.globals[x as usize].0;
            writeln!(out, "    let mut gv{x}: {} = ctx.g{x};", rust_type(t)?).unwrap();
        }

        // In chunked 64-bit mode, precompute non-growing void blocks/loops to
        // specialize under `if mf { <flat m64> } else { <chunked m64> }`.
        let chunked64 = self.module.memory64 && !self.module.flat64 && uses_ml;
        let mut region_end = vec![0usize; code.len()];
        let mut region_may_grow = vec![false; code.len()];
        let mut region_mem_ops = vec![0usize; code.len()];
        if chunked64 {
            let mut open: Vec<usize> = Vec::new();
            for (pc, i) in code.iter().enumerate() {
                match i.opcode {
                    0x02..=0x04 => open.push(pc),
                    0x0b => {
                        if let Some(start_pc) = open.pop() {
                            region_end[start_pc] = pc;
                        }
                    }
                    0x40 => {
                        for &s in &open {
                            region_may_grow[s] = true;
                        }
                    }
                    0x10 => {
                        if let Immediate::Index(c) = i.immediate {
                            let c = c as usize;
                            if !self
                                .module
                                .noreturn_imports
                                .get(c)
                                .copied()
                                .unwrap_or(false)
                                && self.module.fn_may_grow.get(c).copied().unwrap_or(false)
                            {
                                for &s in &open {
                                    region_may_grow[s] = true;
                                }
                            }
                        }
                    }
                    0x11 => {
                        if let Immediate::Pair(ti, tbl) = i.immediate
                            && self
                                .module
                                .ci_index
                                .get(&(tbl, ti))
                                .is_none_or(|&id| self.module.ci_may_grow[id])
                        {
                            for &s in &open {
                                region_may_grow[s] = true;
                            }
                        }
                    }
                    0x28..=0x3e | 0xfc0a | 0xfc0b | 0xfd00 | 0xfd0b => {
                        for &s in &open {
                            region_mem_ops[s] += 1;
                        }
                    }
                    _ => {}
                }
            }
        }

        e.frames.push(Frame {
            kind: Kind::Function,
            label: 0,
            results: ty.results.clone(),
            height: 0,
            unreachable: false,
            result_vars: vec![],
            loop_vars: vec![],
            if_params: vec![],
            has_else: false,
            if_entry_dirty: BitSet::default(),
            end_dirty: BitSet::default(),
        });
        let mut pc = 0usize;
        while pc < code.len() {
            let i = &code[pc];
            let can_specialize = chunked64
                && !e.in_flat_region
                && !e.frames.last().is_some_and(|f| f.unreachable)
                && matches!(i.opcode, 0x02 | 0x03)
                && matches!(i.immediate, Immediate::Block(-64))
                && region_end[pc] > pc
                && region_end[pc] - pc < FLAT_REGION_MAX_INSTRUCTIONS
                && !region_may_grow[pc]
                && region_mem_ops[pc]
                    >= if i.opcode == 0x03 {
                        FLAT_LOOP_MIN_MEM_OPS
                    } else {
                        FLAT_BLOCK_MIN_MEM_OPS
                    };
            if can_specialize {
                let end_pc = region_end[pc];
                e.materialize(|_| true, None)?;
                let saved_stack = e.stack.clone();
                let saved_deps = e.deps.clone();
                let saved_pending = e.pending.clone();
                let saved_unreach = e.frames.last().unwrap().unreachable;
                let saved_dirty = e.dirty_globals.clone();
                e.line("if mf {");
                e.in_flat_region = true;
                for (sub_pc, sub_i) in code.iter().enumerate().take(end_pc + 1).skip(pc) {
                    self.instruction(&mut e, sub_pc, sub_i, &loop_written_globals)
                        .map_err(|err| {
                            format!("instruction {sub_pc} ({:#x}): {err}", sub_i.opcode)
                        })?;
                    if e.over {
                        return Err(format!(
                            "the generated source exceeds {MAX_SOURCE_BYTES} bytes"
                        ));
                    }
                    if e.frames.len() > MAX_NESTING {
                        return Err(format!(
                            "instruction {sub_pc}: blocks nested deeper than {MAX_NESTING}"
                        ));
                    }
                }
                let then_unreach = e.frames.last().unwrap().unreachable;
                let then_dirty = e.dirty_globals.clone();
                e.in_flat_region = false;
                e.stack = saved_stack;
                e.deps = saved_deps;
                e.pending = saved_pending;
                e.dirty_globals = saved_dirty;
                e.frames.last_mut().unwrap().unreachable = saved_unreach;
                e.line("} else {");
                for (sub_pc, sub_i) in code.iter().enumerate().take(end_pc + 1).skip(pc) {
                    self.instruction(&mut e, sub_pc, sub_i, &loop_written_globals)
                        .map_err(|err| {
                            format!("instruction {sub_pc} ({:#x}): {err}", sub_i.opcode)
                        })?;
                    if e.over {
                        return Err(format!(
                            "the generated source exceeds {MAX_SOURCE_BYTES} bytes"
                        ));
                    }
                    if e.frames.len() > MAX_NESTING {
                        return Err(format!(
                            "instruction {sub_pc}: blocks nested deeper than {MAX_NESTING}"
                        ));
                    }
                }
                let else_unreach = e.frames.last().unwrap().unreachable;
                e.line("}");
                e.dirty_globals.union_with(&then_dirty);
                e.frames.last_mut().unwrap().unreachable = then_unreach && else_unreach;
                pc = end_pc + 1;
                continue;
            }
            self.instruction(&mut e, pc, i, &loop_written_globals)
                .map_err(|err| format!("instruction {pc} ({:#x}): {err}", i.opcode))?;
            if e.over {
                return Err(format!(
                    "the generated source exceeds {MAX_SOURCE_BYTES} bytes"
                ));
            }
            if e.frames.len() > MAX_NESTING {
                return Err(format!(
                    "instruction {pc}: blocks nested deeper than {MAX_NESTING}"
                ));
            }
            if e.frames.is_empty() {
                break;
            }
            pc += 1;
        }
        for line in e.s.lines() {
            writeln!(out, "{}", bare_operands(line)).unwrap();
        }
        out.push_str("}\n\n");
        Ok(())
    }

    fn instruction(
        &self,
        e: &mut Emitter,
        pc: usize,
        i: &Instruction,
        loop_written_globals: &[Vec<u32>],
    ) -> Res<()> {
        let op = i.opcode;
        // Dead code after an unconditional branch: skipped to its else or end.
        if e.frames.last().is_some_and(|f| f.unreachable) {
            match op {
                0x02..=0x04 => {
                    e.dead += 1;
                    return Ok(());
                }
                // Exception handling opens and closes blocks of its own: it
                // is refused here as it is in live code, so that skipping
                // cannot misread where the dead code ends.
                0x06..=0x0a | 0x18 | 0x19 | 0x1f => {
                    return Err(format!("unsupported instruction {op:#x}"));
                }
                0x05 if e.dead > 0 => return Ok(()),
                0x0b if e.dead > 0 => {
                    e.dead -= 1;
                    return Ok(());
                }
                0x05 | 0x0b => {}
                _ => return Ok(()),
            }
        }
        let index = || match i.immediate {
            Immediate::Index(x) => Ok(x),
            _ => Err("expected an index".to_string()),
        };
        let suf = try_suffix(e.ret_abi)?;
        match op {
            0x00 => {
                let err = ret_err_expr(e.ret_abi, "Trap::Unreachable")?;
                e.line(&format!("return {err};"));
                e.set_unreachable();
            }
            0x01 => {}
            0x02 | 0x03 => {
                // Pending expressions are evaluated before the block: one
                // read after it would see the block's assignments, and a
                // variable bound inside it would end with it.
                e.materialize(|_| true, None)?;
                let (params, results) = self.block_type(&i.immediate)?;
                e.next += 1;
                let label = e.next;
                let mut result_vars = vec![];
                for t in &results {
                    let v = e.fresh("r");
                    e.line(&format!("let mut {v}: {} = {};", rust_type(*t)?, zero(*t)?));
                    result_vars.push(v);
                }
                let mut loop_vars = vec![];
                if op == 0x03 {
                    for &x in &loop_written_globals[pc] {
                        e.dirty_globals.insert(x);
                    }
                    let values = e.top(params.len())?;
                    let n = e.stack.len();
                    for (k, (v, t)) in values.iter().zip(&params).enumerate() {
                        let lv = e.fresh("p");
                        e.line(&format!("let mut {lv}: {} = {v};", rust_type(*t)?));
                        e.stack[n - params.len() + k].0 = lv.clone();
                        loop_vars.push(lv);
                    }
                }
                let height = e.stack.len() - params.len();
                e.line(&format!(
                    "'L{label}: {}",
                    if op == 0x03 { "loop {" } else { "{" }
                ));
                e.frames.push(Frame {
                    kind: if op == 0x03 { Kind::Loop } else { Kind::Block },
                    label,
                    results,
                    height,
                    unreachable: false,
                    result_vars,
                    loop_vars,
                    if_params: vec![],
                    has_else: false,
                    if_entry_dirty: BitSet::default(),
                    end_dirty: BitSet::new(e.n_globals),
                });
            }
            0x04 => {
                let (params, results) = self.block_type(&i.immediate)?;
                let (c, _) = e.pop()?;
                e.materialize(|_| true, None)?;
                e.next += 1;
                let label = e.next;
                let mut result_vars = vec![];
                for t in &results {
                    let v = e.fresh("r");
                    e.line(&format!("let mut {v}: {} = {};", rust_type(*t)?, zero(*t)?));
                    result_vars.push(v);
                }
                let height = e.stack.len() - params.len();
                let if_params = e.stack[height..].to_vec();
                e.line(&format!("'L{label}: {{ if {c} != 0 {{"));
                e.frames.push(Frame {
                    kind: Kind::If,
                    label,
                    results,
                    height,
                    unreachable: false,
                    result_vars,
                    loop_vars: vec![],
                    if_params,
                    has_else: false,
                    if_entry_dirty: e.dirty_globals.clone(),
                    end_dirty: BitSet::new(e.n_globals),
                });
            }
            0x05 => {
                let frame = e.frames.last().ok_or("else without if")?;
                if !frame.unreachable {
                    let vars = frame.result_vars.clone();
                    let values = e.top(vars.len())?;
                    e.assign(&vars, &values);
                }
                let cur_dirty = e.dirty_globals.clone();
                let frame = e.frames.last_mut().ok_or("else without if")?;
                if !frame.unreachable {
                    frame.end_dirty.union_with(&cur_dirty);
                }
                frame.has_else = true;
                frame.unreachable = false;
                let (height, params, restored_dirty) = (
                    frame.height,
                    frame.if_params.clone(),
                    frame.if_entry_dirty.clone(),
                );
                e.dirty_globals = restored_dirty;
                e.stack.truncate(height);
                e.deps.truncate(height);
                for (v, t) in params {
                    e.push_plain(t, v);
                }
                e.dead = 0;
                e.line("} else {");
            }
            0x0b => {
                let frame = e.frames.last().ok_or("end without block")?;
                let (kind, label, reachable) = (frame.kind, frame.label, !frame.unreachable);
                if reachable {
                    match kind {
                        Kind::Function => {
                            let values = e.top(frame.results.len())?;
                            e.store_globals_return();
                            e.line(&format!("return {};", ok_return_expr(e.ret_abi, &values)));
                        }
                        Kind::Block | Kind::If | Kind::Loop => {
                            let vars = frame.result_vars.clone();
                            let values = e.top(vars.len())?;
                            e.assign(&vars, &values);
                            if kind == Kind::Loop {
                                e.line(&format!("break 'L{label};"));
                            }
                        }
                    }
                }
                let mut frame = e.frames.pop().ok_or("end without block")?;
                if kind == Kind::Function {
                    return Ok(());
                }
                if kind == Kind::If && !frame.has_else {
                    // The implicit else passes the params through as results.
                    e.line("} else {");
                    let values: Vec<String> =
                        frame.if_params.iter().map(|(v, _)| v.clone()).collect();
                    e.assign(&frame.result_vars, &values);
                }
                match kind {
                    Kind::Block => {
                        if reachable {
                            frame.end_dirty.union_with(&e.dirty_globals);
                        }
                        e.dirty_globals = frame.end_dirty;
                    }
                    Kind::If => {
                        if reachable {
                            frame.end_dirty.union_with(&e.dirty_globals);
                        }
                        if !frame.has_else {
                            frame.end_dirty.union_with(&frame.if_entry_dirty);
                        }
                        e.dirty_globals = frame.end_dirty;
                    }
                    Kind::Loop | Kind::Function => {}
                }
                e.line(if kind == Kind::If { "} }" } else { "}" });
                e.stack.truncate(frame.height);
                e.deps.truncate(frame.height);
                for (v, t) in frame.result_vars.iter().zip(&frame.results) {
                    e.push_plain(*t, v.clone());
                }
            }
            0x0c => {
                e.branch(index()? as usize)?;
                e.set_unreachable();
            }
            0x0d => {
                let (c, _) = e.pop()?;
                e.line(&format!("if {c} != 0 {{"));
                e.branch(index()? as usize)?;
                e.line("}");
            }
            0x0e => {
                let Immediate::BrTable(ref targets, default) = i.immediate else {
                    return Err("br_table immediate".into());
                };
                let (idx, _) = e.pop()?;
                e.materialize(|_| true, None)?;
                e.line(&format!("match {idx} as u32 {{"));
                for (k, &d) in targets.iter().enumerate() {
                    e.line(&format!("{k} => {{"));
                    e.branch(d as usize)?;
                    e.line("}");
                }
                e.line("_ => {");
                e.branch(default as usize)?;
                e.line("}");
                e.line("}");
                e.set_unreachable();
            }
            0x0f => {
                let depth = e.frames.len() - 1;
                e.branch(depth)?;
                e.set_unreachable();
            }
            0x10 => {
                let f = index()? as usize;
                let ty = self.module.types.get(f).ok_or("call index")?;
                let n = ty.params.len();
                if e.stack.len() < n {
                    return Err("operand stack underflow".into());
                }
                let at = e.stack.len() - n;
                let args: Vec<String> = e.stack.drain(at..).map(|(v, _)| v).collect();
                e.deps.truncate(at);
                if self.module.noreturn_imports[f] {
                    // Cold, never-returning host import (`goose.abort`, `goose.exit`):
                    // returns `Trap` directly, so no globals need storing or reloading.
                    let mut all = vec!["mb".to_string()];
                    all.extend(args);
                    let err =
                        ret_err_expr(e.ret_abi, &format!("ctx.import{f}({})", all.join(", ")))?;
                    e.line(&format!("return {err};"));
                    e.set_unreachable();
                } else {
                    // Materialize pending expressions reading globals only if callee net-writes globals.
                    if !self.module.fn_net_writes[f].is_empty() {
                        e.materialize(|d| d.globals, None)?;
                    }
                    e.store_globals_for_call(self.module, f);
                    let mut all = vec!["mb".to_string()];
                    for g in self.module.fn_reg_globals(f) {
                        all.push(format!("gv{g}"));
                    }
                    all.extend(args);
                    if f < self.module.imports.len() {
                        let call = format!("t!(ctx.import{f}({}){suf})", all.join(", "));
                        match ty.results.as_slice() {
                            [] => e.line(&format!("{call};")),
                            [t] => e.push(*t, &call)?,
                            ts => {
                                let vars: Vec<String> = ts.iter().map(|_| e.fresh("v")).collect();
                                e.line(&format!("let ({}) = {call};", vars.join(", ")));
                                for (v, t) in vars.into_iter().zip(ts) {
                                    e.push_plain(*t, v);
                                }
                            }
                        }
                    } else {
                        let raw = format!("ctx.f{f}({})", all.join(", "));
                        self.emit_wasm_call_results(e, &ty.results, &raw, &suf)?;
                    }
                    if self.module.fn_may_grow[f] {
                        e.refresh_ml(self.module)?;
                    }
                    e.load_globals_after_call(self.module, f);
                }
            }
            0x11 => {
                let Immediate::Pair(ti, tbl) = i.immediate else {
                    return Err("call_indirect immediate".into());
                };
                let &ci_id = self
                    .module
                    .ci_index
                    .get(&(tbl, ti))
                    .ok_or("call_indirect not registered")?;
                let ty = self
                    .module
                    .module_types
                    .get(ti as usize)
                    .ok_or("call_indirect type")?;
                let (idx, _) = e.pop()?;
                let n = ty.params.len();
                if e.stack.len() < n {
                    return Err("operand stack underflow".into());
                }
                let at = e.stack.len() - n;
                let args: Vec<String> = e.stack.drain(at..).map(|(v, _)| v).collect();
                e.deps.truncate(at);
                if !self.module.ci_net_writes[ci_id].is_empty() {
                    e.materialize(|d| d.globals, None)?;
                }
                e.store_globals_for_ci(self.module, ci_id);
                let mut all = vec!["mb".to_string()];
                for &g in &self.module.ci_reg_globals[ci_id] {
                    all.push(format!("gv{g}"));
                }
                all.push(idx);
                all.extend(args);
                let raw = format!("ctx.ci{ci_id}({})", all.join(", "));
                self.emit_wasm_call_results(e, &ty.results, &raw, &suf)?;
                if self.module.ci_may_grow[ci_id] {
                    e.refresh_ml(self.module)?;
                }
                e.load_globals_after_ci(self.module, ci_id);
            }
            0x1a => {
                e.pop()?;
            }
            0x1b | 0x1c => {
                let (c, _, dc) = e.pop_dep()?;
                let (b, _, db) = e.pop_dep()?;
                let (a, t, da) = e.pop_dep()?;
                e.push_pure(
                    t,
                    &format!("if {c} != 0 {{ {a} }} else {{ {b} }}"),
                    da.union(&db).union(&dc),
                )?;
            }
            0x20 => {
                let x = index()? as usize;
                let t = *self.locals.get(x).ok_or("local index")?;
                e.push_pure(t, &format!("l{x}"), Dep::local(x as u32))?;
            }
            0x21 => {
                let x = index()?;
                let (v, _) = e.pop()?;
                e.materialize(|d| d.locals.contains(&x), None)?;
                e.line(&format!("l{x} = {v};"));
            }
            0x22 => {
                let x = index()?;
                // The value stays on the stack: bound once, then copied.
                let (v, t) = e.pop()?;
                e.push(t, &v)?;
                e.materialize(|d| d.locals.contains(&x), None)?;
                let v = e.top(1)?.remove(0);
                e.line(&format!("l{x} = {v};"));
            }
            0x23 => {
                let x = index()? as usize;
                let (t, _) = self.module.globals.get(x).ok_or("global index")?;
                if let Some(c) = self.module.global_consts[x] {
                    e.push_plain(*t, c.rust_literal());
                } else {
                    let place = if e.globals_cached.contains(&(x as u32)) {
                        format!("gv{x}")
                    } else {
                        format!("ctx.g{x}")
                    };
                    e.push_pure(*t, &place, Dep::global())?;
                }
            }
            0x24 => {
                let x = index()?;
                let (v, _) = e.pop()?;
                if self
                    .module
                    .global_consts
                    .get(x as usize)
                    .and_then(|c| *c)
                    .is_some()
                {
                    // Proven constant-init global: every write stores the same constant.
                } else {
                    e.materialize(|d| d.globals, None)?;
                    if e.globals_cached.contains(&x) {
                        e.line(&format!("gv{x} = {v};"));
                        if e.globals_set.binary_search(&x).is_ok() {
                            e.dirty_globals.insert(x);
                        }
                    } else {
                        e.line(&format!("ctx.g{x} = {v};"));
                    }
                }
            }
            0x25 => {
                let t = index()?;
                if !self.module.mut_extern_tables.contains_key(&t) {
                    return Err(format!("table.get on unsupported table {t}"));
                }
                let (idx, _) = e.pop()?;
                e.push(
                    ValType::EXTERNREF,
                    &format!("t!(ctx.t{t}.get({idx} as u32 as usize).copied().ok_or(Trap::TableOutOfBounds){suf})"),
                )?;
            }
            0x26 => {
                let t = index()?;
                if !self.module.mut_extern_tables.contains_key(&t) {
                    return Err(format!("table.set on unsupported table {t}"));
                }
                let (val, _) = e.pop()?;
                let (idx, _) = e.pop()?;
                e.line(&format!("*t!(ctx.t{t}.get_mut({idx} as u32 as usize).ok_or(Trap::TableOutOfBounds){suf}) = {val};"));
            }
            0xd0 => {
                e.push_plain(ValType::EXTERNREF, "0u64".into());
            }
            0xd1 => {
                let (v, _, dv) = e.pop_dep()?;
                e.push_pure(ValType::I32, &format!("({v} == 0u64) as i32"), dv)?;
            }
            0x28..=0x35 | 0xfd00 => self.load(e, i)?,
            0x36..=0x3e | 0xfd0b => self.store(e, i)?,
            0x3f => {
                if !self.module.memory64 {
                    if let Some(bytes) = self.module.fixed32 {
                        e.push_plain(ValType::I32, format!("({}i32)", bytes >> 16));
                    } else {
                        e.push(ValType::I32, "(ml >> 16) as i32")?;
                    }
                } else if let Some(bytes) = self.module.fixed64 {
                    e.push_plain(ValType::I64, format!("({}i64)", bytes >> 16));
                } else if self.module.flat64 {
                    e.push(ValType::I64, "(ml >> 16) as i64")?;
                } else {
                    e.push(ValType::I64, "(ctx.outer.len() >> 16) as i64")?;
                }
            }
            0x40 => {
                let (n, _) = e.pop()?;
                if !self.module.memory64 {
                    if let Some(bytes) = self.module.fixed32 {
                        e.push(
                            ValType::I32,
                            &format!("if ({n}) == 0 {{ {}i32 }} else {{ -1i32 }}", bytes >> 16),
                        )?;
                    } else {
                        let delta = self.address(&n);
                        e.push(ValType::I32, &format!("ctx.mem.grow({delta}) as i32"))?;
                        e.refresh_ml(self.module)?;
                    }
                } else if let Some(bytes) = self.module.fixed64 {
                    e.push(
                        ValType::I64,
                        &format!("if ({n}) == 0 {{ {}i64 }} else {{ -1i64 }}", bytes >> 16),
                    )?;
                } else {
                    let delta = self.address(&n);
                    e.push(ValType::I64, &format!("ctx.outer.grow({delta})"))?;
                    e.refresh_ml(self.module)?;
                }
            }
            0x41 => {
                let Immediate::I32(v) = i.immediate else {
                    return Err("i32.const".into());
                };
                e.push_plain(ValType::I32, format!("({v}i32)"));
            }
            0x42 => {
                let Immediate::I64(v) = i.immediate else {
                    return Err("i64.const".into());
                };
                e.push_plain(ValType::I64, format!("({v}i64)"));
            }
            0x43 => {
                let Immediate::F32(v) = i.immediate else {
                    return Err("f32.const".into());
                };
                e.push_plain(ValType::F32, format!("f32::from_bits({v:#x})"));
            }
            0x44 => {
                let Immediate::F64(v) = i.immediate else {
                    return Err("f64.const".into());
                };
                e.push_plain(ValType::F64, format!("f64::from_bits({v:#x})"));
            }
            0xfc0a => {
                let (n, _) = e.pop()?;
                let (src, _) = e.pop()?;
                let (dst, _) = e.pop()?;
                let literal = n.trim_matches(['(', ')']);
                let constant = literal
                    .strip_suffix("i32")
                    .or(literal.strip_suffix("i64"))
                    .and_then(|k| k.parse::<usize>().ok());
                let ml = self.ml();
                match constant {
                    Some(k @ 1..=COPY_INLINE) if !self.module.memory64 && self.module.target == Target::BlotCdylib => {
                        e.line(&format!("t!(m32s::copy_n::<{k}>(mb, {ml}, {dst} as u32, {src} as u32){suf});"))
                    }
                    Some(k @ 1..=COPY_INLINE) if !self.module.memory64 => {
                        e.line(&format!("t!(m32::copy_n::<CAP32, {k}>(mb, {ml}, {dst} as u32, {src} as u32){suf});"))
                    }
                    Some(k @ 1..=COPY_INLINE) if self.module.cap64.is_some() => {
                        e.line(&format!("t!(m64f::copy_n::<CAP64, {k}>(mb, {ml}, {dst} as u64, {src} as u64){suf});"))
                    }
                    Some(k @ 1..=COPY_INLINE) if self.module.flat64 || e.in_flat_region => {
                        e.line(&format!("t!(m64::fcopy_n::<{k}>(mb, {ml}, {dst} as u64, {src} as u64){suf});"))
                    }
                    Some(k @ 1..=COPY_INLINE) => {
                        e.line(&format!("t!(m64::copy_n::<{k}>(mb, {ml}, &mut ctx.outer, {dst} as u64, {src} as u64){suf});"))
                    }
                    _ if !self.module.memory64 && self.module.target == Target::BlotCdylib => {
                        e.line(&format!("t!(m32s::copy(mb, {ml}, {dst} as u32, {src} as u32, {n} as u32){suf});"));
                    }
                    _ if !self.module.memory64 => {
                        e.line(&format!("t!(m32::copy::<CAP32>(mb, {ml}, {dst} as u32, {src} as u32, {n} as u32){suf});"));
                    }
                    _ => {
                        let (dst, src, n) = (self.address(&dst), self.address(&src), self.address(&n));
                        e.line(&format!("t!(copy_parts(mb, &mut ctx.outer, {dst}, {src}, {n}){suf});"));
                    }
                }
            }
            0xfc0b => {
                let (n, _) = e.pop()?;
                let (value, _) = e.pop()?;
                let (dst, _) = e.pop()?;
                let literal = n.trim_matches(['(', ')']);
                let constant = literal
                    .strip_suffix("i32")
                    .or(literal.strip_suffix("i64"))
                    .and_then(|k| k.parse::<usize>().ok());
                let ml = self.ml();
                match constant {
                    Some(k @ 1..=COPY_INLINE) if !self.module.memory64 && self.module.target == Target::BlotCdylib => {
                        e.line(&format!("t!(m32s::fill_n::<{k}>(mb, {ml}, {dst} as u32, {value} as u8){suf});"))
                    }
                    Some(k @ 1..=COPY_INLINE) if !self.module.memory64 => {
                        e.line(&format!("t!(m32::fill_n::<CAP32, {k}>(mb, {ml}, {dst} as u32, {value} as u8){suf});"))
                    }
                    Some(k @ 1..=COPY_INLINE) if self.module.cap64.is_some() => {
                        e.line(&format!("t!(m64f::fill_n::<CAP64, {k}>(mb, {ml}, {dst} as u64, {value} as u8){suf});"))
                    }
                    Some(k @ 1..=COPY_INLINE) if self.module.flat64 || e.in_flat_region => {
                        e.line(&format!("t!(m64::ffill_n::<{k}>(mb, {ml}, {dst} as u64, {value} as u8){suf});"))
                    }
                    Some(k @ 1..=COPY_INLINE) => {
                        e.line(&format!("t!(m64::fill_n::<{k}>(mb, {ml}, &mut ctx.outer, {dst} as u64, {value} as u8){suf});"))
                    }
                    _ if !self.module.memory64 && self.module.target == Target::BlotCdylib => {
                        e.line(&format!("t!(m32s::fill(mb, {ml}, {dst} as u32, {value} as u8, {n} as u32){suf});"));
                    }
                    _ if !self.module.memory64 => {
                        e.line(&format!("t!(m32::fill::<CAP32>(mb, {ml}, {dst} as u32, {value} as u8, {n} as u32){suf});"));
                    }
                    _ => {
                        let (dst, n) = (self.address(&dst), self.address(&n));
                        e.line(&format!("t!(fill_parts(mb, &mut ctx.outer, {dst}, {value} as u8, {n}){suf});"));
                    }
                }
            }
            0xfd0c => {
                let Immediate::Bytes(ref b) = i.immediate else {
                    return Err("v128.const".into());
                };
                let bytes: [u8; 16] = b.as_ref().try_into().map_err(|_| "v128.const")?;
                e.push_plain(ValType::V128, format!("{}u128", u128::from_le_bytes(bytes)));
            }
            // i8x16, i16x8 and i32x4.splat: the low lane bits times a
            // multiplier with a 1 at the start of every lane.
            0xfd0f..=0xfd11 => {
                let (a, _) = e.pop()?;
                let (lane, ones) = match op {
                    0xfd0f => ("u8", 0x0101_0101_0101_0101_0101_0101_0101_0101u128),
                    0xfd10 => ("u16", 0x0001_0001_0001_0001_0001_0001_0001_0001u128),
                    _ => ("u32", 0x0000_0001_0000_0001_0000_0001_0000_0001u128),
                };
                e.push(
                    ValType::V128,
                    &format!("(({a} as {lane} as u128).wrapping_mul({ones}u128))"),
                )?;
            }
            0xfd12 => {
                let (a, _) = e.pop()?;
                e.push(
                    ValType::V128,
                    &format!("(({a} as u64 as u128) | (({a} as u64 as u128) << 64))"),
                )?;
            }
            0xfd1d => {
                let lane = index()?;
                let (v, _) = e.pop()?;
                e.push(
                    ValType::I64,
                    &format!("({v} >> {}) as u64 as i64", 64 * lane),
                )?;
            }
            0xfd1e => {
                let lane = index()?;
                let (x, _) = e.pop()?;
                let (v, _) = e.pop()?;
                let shift = 64 * lane;
                e.push(
                    ValType::V128,
                    &format!("({v} & !(0xffff_ffff_ffff_ffffu128 << {shift})) | (({x} as u64 as u128) << {shift})"),
                )?;
            }
            _ => match numeric(op) {
                Some((arity, out, template)) => {
                    let b = if arity == 2 { Some(e.pop_dep()?) } else { None };
                    let (a, _, da) = e.pop_dep()?;
                    let mut expr = template.replace("{a}", &a);
                    let mut dep = da;
                    if let Some((b, _, db)) = b {
                        expr = expr.replace("{b}", &b);
                        dep = dep.union(&db);
                    }
                    if template.contains("t!(") {
                        // Traps: evaluated here, in order.
                        let expr = expr.replace("{try}", &suf);
                        e.push(out, &expr)?;
                    } else {
                        e.push_pure(out, &expr, dep)?;
                    }
                }
                None => return Err(format!("unsupported instruction {op:#x}")),
            },
        }
        Ok(())
    }

    fn emit_wasm_call_results(
        &self,
        e: &mut Emitter,
        results: &[ValType],
        raw: &str,
        suf: &str,
    ) -> Res<()> {
        match ret_abi(results) {
            RetAbi::Void => {
                e.line(&format!("tv!({raw}{suf});"));
            }
            RetAbi::Scalar(t) => {
                e.push(t, &format!("tc!({raw}{suf})"))?;
            }
            RetAbi::Pair32(a, b) => {
                let p = e.fresh("v");
                e.line(&format!("let {p}: u64 = tc!({raw}{suf});"));
                let v0 = e.fresh("v");
                let v1 = e.fresh("v");
                e.line(&format!(
                    "let {v0}: {} = {};",
                    rust_type(a)?,
                    unpack_u32_lane(a, &format!("{p} as u32"))
                ));
                e.line(&format!(
                    "let {v1}: {} = {};",
                    rust_type(b)?,
                    unpack_u32_lane(b, &format!("({p} >> 32) as u32"))
                ));
                e.push_plain(a, v0);
                e.push_plain(b, v1);
            }
            RetAbi::Result => {
                let call = format!("t!({raw}{suf})");
                match results {
                    [] => e.line(&format!("{call};")),
                    [t] => e.push(*t, &call)?,
                    ts => {
                        let vars: Vec<String> = ts.iter().map(|_| e.fresh("v")).collect();
                        e.line(&format!("let ({}) = {call};", vars.join(", ")));
                        for (v, t) in vars.into_iter().zip(ts) {
                            e.push_plain(*t, v);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn memarg(&self, i: &Instruction) -> Res<u64> {
        match i.immediate {
            Immediate::MemArg {
                memory: 0, offset, ..
            } => Ok(offset),
            _ => Err("memory instruction on another memory".into()),
        }
    }

    fn load(&self, e: &mut Emitter, i: &Instruction) -> Res<()> {
        let ml = self.ml();
        let suf = try_suffix(e.ret_abi)?;
        let offset = self.memarg(i)?;
        let (a, _) = e.pop()?;
        // (bytes, result type, conversion of the loaded unsigned integer)
        let (n, t, conversion) = match i.opcode {
            0x28 => (4, ValType::I32, "{} as i32"),
            0x29 => (8, ValType::I64, "{} as i64"),
            0x2a => (4, ValType::F32, "f32::from_bits({})"),
            0x2b => (8, ValType::F64, "f64::from_bits({})"),
            0x2c => (1, ValType::I32, "{} as i8 as i32"),
            0x2d => (1, ValType::I32, "{} as i32"),
            0x2e => (2, ValType::I32, "{} as i16 as i32"),
            0x2f => (2, ValType::I32, "{} as i32"),
            0x30 => (1, ValType::I64, "{} as i8 as i64"),
            0x31 => (1, ValType::I64, "{} as i64"),
            0x32 => (2, ValType::I64, "{} as i16 as i64"),
            0x33 => (2, ValType::I64, "{} as i64"),
            0x34 => (4, ValType::I64, "{} as i32 as i64"),
            0x35 => (4, ValType::I64, "{} as i64"),
            0xfd00 => (16, ValType::V128, "{}"),
            op => return Err(format!("load {op:#x}")),
        };
        let loaded = if !self.module.memory64 && self.module.target == Target::BlotCdylib {
            format!("t!(m32s::l{}(mb, {ml}, {a} as u32, {offset}){suf})", n * 8)
        } else if !self.module.memory64 {
            format!("t!(m32::l{}(mb, {ml}, {a} as u32, {offset}){suf})", n * 8)
        } else if self.module.cap64.is_some() {
            format!(
                "t!(m64f::l{}::<CAP64>(mb, {ml}, {}, {offset}){suf})",
                n * 8,
                self.address(&a)
            )
        } else if self.module.flat64 || e.in_flat_region {
            format!(
                "t!(m64::fl{}(mb, {ml}, {}, {offset}){suf})",
                n * 8,
                self.address(&a)
            )
        } else {
            format!(
                "t!(m64::l{}(mb, {ml}, &ctx.outer, {}, {offset}){suf})",
                n * 8,
                self.address(&a)
            )
        };
        e.push(t, &conversion.replace("{}", &loaded))
    }

    fn store(&self, e: &mut Emitter, i: &Instruction) -> Res<()> {
        let ml = self.ml();
        let suf = try_suffix(e.ret_abi)?;
        let offset = self.memarg(i)?;
        let (v, _) = e.pop()?;
        let (a, _) = e.pop()?;
        // (bytes, the stored unsigned integer)
        let (n, value) = match i.opcode {
            0x36 => (4, format!("{v} as u32")),
            0x37 => (8, format!("{v} as u64")),
            0x38 => (4, format!("{v}.to_bits()")),
            0x39 => (8, format!("{v}.to_bits()")),
            0x3a | 0x3c => (1, format!("{v} as u8")),
            0x3b | 0x3d => (2, format!("{v} as u16")),
            0x3e => (4, format!("{v} as u32")),
            0xfd0b => (16, v.to_string()),
            op => return Err(format!("store {op:#x}")),
        };
        let store = if !self.module.memory64 && self.module.target == Target::BlotCdylib {
            format!(
                "t!(m32s::s{}(mb, {ml}, {a} as u32, {offset}, {value}){suf});",
                n * 8
            )
        } else if !self.module.memory64 {
            format!(
                "t!(m32::s{}(mb, {ml}, {a} as u32, {offset}, {value}){suf});",
                n * 8
            )
        } else if self.module.cap64.is_some() {
            format!(
                "t!(m64f::s{}::<CAP64>(mb, {ml}, {}, {offset}, {value}){suf});",
                n * 8,
                self.address(&a)
            )
        } else if self.module.flat64 || e.in_flat_region {
            format!(
                "t!(m64::fs{}(mb, {ml}, {}, {offset}, {value}){suf});",
                n * 8,
                self.address(&a)
            )
        } else {
            format!(
                "t!(m64::s{}(mb, {ml}, &mut ctx.outer, {}, {offset}, {value}){suf});",
                n * 8,
                self.address(&a)
            )
        };
        e.line(&store);
        Ok(())
    }
}

/// (arity, result type, Rust template over {a} and {b}) of a numeric
/// instruction.
fn numeric(op: u16) -> Option<(usize, ValType, &'static str)> {
    use ValType::{F32, F64, I32, I64};
    Some(match op {
        0x45 => (1, I32, "({a} == 0) as i32"),
        0x46 => (2, I32, "({a} == {b}) as i32"),
        0x47 => (2, I32, "({a} != {b}) as i32"),
        0x48 => (2, I32, "({a} < {b}) as i32"),
        0x49 => (2, I32, "(({a} as u32) < ({b} as u32)) as i32"),
        0x4a => (2, I32, "({a} > {b}) as i32"),
        0x4b => (2, I32, "(({a} as u32) > ({b} as u32)) as i32"),
        0x4c => (2, I32, "({a} <= {b}) as i32"),
        0x4d => (2, I32, "(({a} as u32) <= ({b} as u32)) as i32"),
        0x4e => (2, I32, "({a} >= {b}) as i32"),
        0x4f => (2, I32, "(({a} as u32) >= ({b} as u32)) as i32"),
        0x50 => (1, I32, "({a} == 0) as i32"),
        0x51 => (2, I32, "({a} == {b}) as i32"),
        0x52 => (2, I32, "({a} != {b}) as i32"),
        0x53 => (2, I32, "({a} < {b}) as i32"),
        0x54 => (2, I32, "(({a} as u64) < ({b} as u64)) as i32"),
        0x55 => (2, I32, "({a} > {b}) as i32"),
        0x56 => (2, I32, "(({a} as u64) > ({b} as u64)) as i32"),
        0x57 => (2, I32, "({a} <= {b}) as i32"),
        0x58 => (2, I32, "(({a} as u64) <= ({b} as u64)) as i32"),
        0x59 => (2, I32, "({a} >= {b}) as i32"),
        0x5a => (2, I32, "(({a} as u64) >= ({b} as u64)) as i32"),
        0x5b | 0x61 => (2, I32, "({a} == {b}) as i32"),
        0x5c | 0x62 => (2, I32, "({a} != {b}) as i32"),
        0x5d | 0x63 => (2, I32, "({a} < {b}) as i32"),
        0x5e | 0x64 => (2, I32, "({a} > {b}) as i32"),
        0x5f | 0x65 => (2, I32, "({a} <= {b}) as i32"),
        0x60 | 0x66 => (2, I32, "({a} >= {b}) as i32"),
        0x67 => (1, I32, "{a}.leading_zeros() as i32"),
        0x68 => (1, I32, "{a}.trailing_zeros() as i32"),
        0x69 => (1, I32, "{a}.count_ones() as i32"),
        0x6a => (2, I32, "{a}.wrapping_add({b})"),
        0x6b => (2, I32, "{a}.wrapping_sub({b})"),
        0x6c => (2, I32, "{a}.wrapping_mul({b})"),
        0x6d => (2, I32, "t!(i32_div_s({a}, {b}){try})"),
        0x6e => (2, I32, "t!(i32_div_u({a}, {b}){try})"),
        0x6f => (2, I32, "t!(i32_rem_s({a}, {b}){try})"),
        0x70 => (2, I32, "t!(i32_rem_u({a}, {b}){try})"),
        0x71 => (2, I32, "{a} & {b}"),
        0x72 => (2, I32, "{a} | {b}"),
        0x73 => (2, I32, "{a} ^ {b}"),
        0x74 => (2, I32, "{a}.wrapping_shl({b} as u32)"),
        0x75 => (2, I32, "{a}.wrapping_shr({b} as u32)"),
        0x76 => (2, I32, "({a} as u32).wrapping_shr({b} as u32) as i32"),
        0x77 => (2, I32, "{a}.rotate_left({b} as u32)"),
        0x78 => (2, I32, "{a}.rotate_right({b} as u32)"),
        0x79 => (1, I64, "{a}.leading_zeros() as i64"),
        0x7a => (1, I64, "{a}.trailing_zeros() as i64"),
        0x7b => (1, I64, "{a}.count_ones() as i64"),
        0x7c => (2, I64, "{a}.wrapping_add({b})"),
        0x7d => (2, I64, "{a}.wrapping_sub({b})"),
        0x7e => (2, I64, "{a}.wrapping_mul({b})"),
        0x7f => (2, I64, "t!(i64_div_s({a}, {b}){try})"),
        0x80 => (2, I64, "t!(i64_div_u({a}, {b}){try})"),
        0x81 => (2, I64, "t!(i64_rem_s({a}, {b}){try})"),
        0x82 => (2, I64, "t!(i64_rem_u({a}, {b}){try})"),
        0x83 => (2, I64, "{a} & {b}"),
        0x84 => (2, I64, "{a} | {b}"),
        0x85 => (2, I64, "{a} ^ {b}"),
        0x86 => (2, I64, "{a}.wrapping_shl({b} as u32)"),
        0x87 => (2, I64, "{a}.wrapping_shr({b} as u32)"),
        0x88 => (2, I64, "({a} as u64).wrapping_shr({b} as u32) as i64"),
        0x89 => (2, I64, "{a}.rotate_left({b} as u32)"),
        0x8a => (2, I64, "{a}.rotate_right({b} as u32)"),
        0x8b => (1, F32, "{a}.abs()"),
        0x8c => (1, F32, "-{a}"),
        0x8d => (1, F32, "{a}.ceil()"),
        0x8e => (1, F32, "{a}.floor()"),
        0x8f => (1, F32, "{a}.trunc()"),
        0x90 => (1, F32, "{a}.round_ties_even()"),
        0x91 => (1, F32, "{a}.sqrt()"),
        0x92 => (2, F32, "{a} + {b}"),
        0x93 => (2, F32, "{a} - {b}"),
        0x94 => (2, F32, "{a} * {b}"),
        0x95 => (2, F32, "f32_div({a}, {b})"),
        0x96 => (2, F32, "f32_min({a}, {b})"),
        0x97 => (2, F32, "f32_max({a}, {b})"),
        0x98 => (2, F32, "{a}.copysign({b})"),
        0x99 => (1, F64, "{a}.abs()"),
        0x9a => (1, F64, "-{a}"),
        0x9b => (1, F64, "{a}.ceil()"),
        0x9c => (1, F64, "{a}.floor()"),
        0x9d => (1, F64, "{a}.trunc()"),
        0x9e => (1, F64, "{a}.round_ties_even()"),
        0x9f => (1, F64, "{a}.sqrt()"),
        0xa0 => (2, F64, "{a} + {b}"),
        0xa1 => (2, F64, "{a} - {b}"),
        0xa2 => (2, F64, "{a} * {b}"),
        0xa3 => (2, F64, "f64_div({a}, {b})"),
        0xa4 => (2, F64, "f64_min({a}, {b})"),
        0xa5 => (2, F64, "f64_max({a}, {b})"),
        0xa6 => (2, F64, "{a}.copysign({b})"),
        0xa7 => (1, I32, "{a} as i32"),
        0xa8 => (1, I32, "t!(i32_trunc_f32_s({a}){try})"),
        0xa9 => (1, I32, "t!(i32_trunc_f32_u({a}){try}) as i32"),
        0xaa => (1, I32, "t!(i32_trunc_f64_s({a}){try})"),
        0xab => (1, I32, "t!(i32_trunc_f64_u({a}){try}) as i32"),
        0xac => (1, I64, "{a} as i64"),
        0xad => (1, I64, "{a} as u32 as i64"),
        0xae => (1, I64, "t!(i64_trunc_f32_s({a}){try})"),
        0xaf => (1, I64, "t!(i64_trunc_f32_u({a}){try}) as i64"),
        0xb0 => (1, I64, "t!(i64_trunc_f64_s({a}){try})"),
        0xb1 => (1, I64, "t!(i64_trunc_f64_u({a}){try}) as i64"),
        0xb2 => (1, F32, "{a} as f32"),
        0xb3 => (1, F32, "{a} as u32 as f32"),
        0xb4 => (1, F32, "{a} as f32"),
        0xb5 => (1, F32, "{a} as u64 as f32"),
        0xb6 => (1, F32, "{a} as f32"),
        0xb7 => (1, F64, "{a} as f64"),
        0xb8 => (1, F64, "{a} as u32 as f64"),
        0xb9 => (1, F64, "{a} as f64"),
        0xba => (1, F64, "{a} as u64 as f64"),
        0xbb => (1, F64, "{a} as f64"),
        0xbc => (1, I32, "{a}.to_bits() as i32"),
        0xbd => (1, I64, "{a}.to_bits() as i64"),
        0xbe => (1, F32, "f32::from_bits({a} as u32)"),
        0xbf => (1, F64, "f64::from_bits({a} as u64)"),
        0xc0 => (1, I32, "{a} as i8 as i32"),
        0xc1 => (1, I32, "{a} as i16 as i32"),
        0xc2 => (1, I64, "{a} as i8 as i64"),
        0xc3 => (1, I64, "{a} as i16 as i64"),
        0xc4 => (1, I64, "{a} as i32 as i64"),
        0xfc00 => (1, I32, "{a} as i32"),
        0xfc01 => (1, I32, "{a} as u32 as i32"),
        0xfc02 => (1, I32, "{a} as i32"),
        0xfc03 => (1, I32, "{a} as u32 as i32"),
        0xfc04 => (1, I64, "{a} as i64"),
        0xfc05 => (1, I64, "{a} as u64 as i64"),
        0xfc06 => (1, I64, "{a} as i64"),
        0xfc07 => (1, I64, "{a} as u64 as i64"),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        ConstVal, Target, bare_operands, eval_constant, function_preserves_global,
        function_writes_only_same_global, generate, generate_target, tarjan_scc,
    };
    use wtime_types::{FuncType, ValType};
    use wtime_validate::{Immediate, Instruction};

    fn insn(opcode: u16, immediate: Immediate) -> Instruction {
        Instruction { opcode, immediate }
    }

    fn void_fn() -> FuncType {
        FuncType {
            params: vec![],
            results: vec![],
        }
    }

    #[test]
    fn whole_values_lose_their_parentheses() {
        let cases = [
            ("l15 = (self.g0);", "l15 = self.g0;"),
            (
                "l16 = ((l15).wrapping_add((176i32)));",
                "l16 = (l15).wrapping_add(176i32);",
            ),
            (
                "self.f7((3872i32), (26i32), ((l1) + (l2)))?;",
                "self.f7(3872i32, 26i32, (l1) + (l2))?;",
            ),
            ("if ((l1) != 0) {", "if (l1) != 0 {"),
            ("if (l1) != 0 {", "if (l1) != 0 {"),
            ("(r1, r2) = ((l1), (l2));", "(r1, r2) = (l1, l2);"),
            (
                "let v: i32 = ((l1) == ((l2) == (l3)));",
                "let v: i32 = (l1) == ((l2) == (l3));",
            ),
            ("x = (a) <= (b);", "x = (a) <= (b);"),
            ("diff (a) {", "diff (a) {"),
            ("f(());", "f(());"),
            (
                "'L27: { if ((((l2) as u64) > ((((l24).wrapping_sub((l23))) as u32 as i64) as u64)) as i32) != 0 {",
                "'L27: { if ((((l2) as u64) > ((((l24).wrapping_sub(l23)) as u32 as i64) as u64)) as i32) != 0 {",
            ),
            ("'L1: { if (x) {", "'L1: { if x {"),
        ];
        for (line, bare) in cases {
            assert_eq!(bare_operands(line), bare, "{line}");
        }
    }

    #[test]
    fn eval_constant_handles_chains_and_extended_ops() {
        let consts = [Some(ConstVal::I32(10)), None, Some(ConstVal::I64(100))];
        // global.get 0 + 5 * 2 = 20
        let expr = [
            insn(0x23, Immediate::Index(0)),
            insn(0x41, Immediate::I32(5)),
            insn(0x41, Immediate::I32(2)),
            insn(0x6c, Immediate::None),
            insn(0x6a, Immediate::None),
            insn(0x0b, Immediate::None),
        ];
        assert_eq!(eval_constant(&expr, &consts), Some(ConstVal::I32(20)));
        // i64 sub
        let expr64 = [
            insn(0x23, Immediate::Index(2)),
            insn(0x42, Immediate::I64(37)),
            insn(0x7d, Immediate::None),
            insn(0x0b, Immediate::None),
        ];
        assert_eq!(eval_constant(&expr64, &consts), Some(ConstVal::I64(63)));
        // Non-constant global reference must return None.
        let non_const = [insn(0x23, Immediate::Index(1)), insn(0x0b, Immediate::None)];
        assert_eq!(eval_constant(&non_const, &consts), None);
    }

    #[test]
    fn writes_only_same_global_accepts_save_restore_and_rejects_mutation() {
        let types = [void_fn()];
        let noreturn = [false];
        // Save g0 into l0, restore g0 inside a block and loop without mutating l0.
        let save_restore = [
            insn(0x23, Immediate::Index(0)),
            insn(0x21, Immediate::Index(0)),
            insn(0x03, Immediate::Block(-64)),
            insn(0x20, Immediate::Index(0)),
            insn(0x24, Immediate::Index(0)),
            insn(0x0b, Immediate::None),
            insn(0x0b, Immediate::None),
        ];
        assert!(function_writes_only_same_global(
            &save_restore,
            1,
            &types,
            &types,
            &noreturn,
            0
        ));

        // Writing a computed value rejects constant-init elimination.
        let mutated = [
            insn(0x23, Immediate::Index(0)),
            insn(0x41, Immediate::I32(1)),
            insn(0x6a, Immediate::None),
            insn(0x24, Immediate::Index(0)),
            insn(0x0b, Immediate::None),
        ];
        assert!(!function_writes_only_same_global(
            &mutated, 1, &types, &types, &noreturn, 0
        ));

        // Modifying saved local inside a loop must invalidate it at loop header.
        let loop_clobber = [
            insn(0x23, Immediate::Index(0)),
            insn(0x21, Immediate::Index(0)),
            insn(0x03, Immediate::Block(-64)),
            insn(0x20, Immediate::Index(0)),
            insn(0x24, Immediate::Index(0)),
            insn(0x41, Immediate::I32(9)),
            insn(0x21, Immediate::Index(0)),
            insn(0x0c, Immediate::Index(0)),
            insn(0x0b, Immediate::None),
            insn(0x0b, Immediate::None),
        ];
        assert!(!function_writes_only_same_global(
            &loop_clobber,
            1,
            &types,
            &types,
            &noreturn,
            0
        ));
    }

    #[test]
    fn preserves_global_handles_loops_branches_and_callees() {
        // f0 = noreturn abort, f1 = preserving callee, f2 = mutating callee.
        let types = [void_fn(), void_fn(), void_fn()];
        let ci_index = std::collections::BTreeMap::new();
        let ci_targets: Vec<Vec<(u32, usize)>> = Vec::new();
        let noreturn = [true, false, false];
        let preserves = [true, true, false];

        // Save g0, bump g0, call preserving f1, optionally abort via f0, restore g0.
        let ok_loop = [
            insn(0x23, Immediate::Index(0)),
            insn(0x21, Immediate::Index(0)),
            insn(0x03, Immediate::Block(-64)),
            insn(0x23, Immediate::Index(0)),
            insn(0x41, Immediate::I32(1)),
            insn(0x6a, Immediate::None),
            insn(0x24, Immediate::Index(0)),
            insn(0x10, Immediate::Index(1)),
            insn(0x41, Immediate::I32(0)),
            insn(0x04, Immediate::Block(-64)),
            insn(0x10, Immediate::Index(0)), // noreturn abort while g0 is bumped
            insn(0x0b, Immediate::None),
            insn(0x20, Immediate::Index(0)),
            insn(0x24, Immediate::Index(0)), // restored before backedge
            insn(0x41, Immediate::I32(1)),
            insn(0x0d, Immediate::Index(0)),
            insn(0x0b, Immediate::None),
            insn(0x0b, Immediate::None),
        ];
        assert!(function_preserves_global(
            &ok_loop,
            1,
            &types,
            &types,
            &ci_index,
            &ci_targets,
            &noreturn,
            &preserves,
            0
        ));

        // Early return before restoring g0 must be rejected.
        let bad_early_ret = [
            insn(0x23, Immediate::Index(0)),
            insn(0x21, Immediate::Index(0)),
            insn(0x41, Immediate::I32(1)),
            insn(0x24, Immediate::Index(0)),
            insn(0x41, Immediate::I32(1)),
            insn(0x04, Immediate::Block(-64)),
            insn(0x0f, Immediate::None),
            insn(0x0b, Immediate::None),
            insn(0x20, Immediate::Index(0)),
            insn(0x24, Immediate::Index(0)),
            insn(0x0b, Immediate::None),
        ];
        assert!(!function_preserves_global(
            &bad_early_ret,
            1,
            &types,
            &types,
            &ci_index,
            &ci_targets,
            &noreturn,
            &preserves,
            0
        ));

        // Loop backedge taken while g0 is modified breaks header invariant on retry.
        let bad_loop_backedge = [
            insn(0x03, Immediate::Block(-64)),
            insn(0x23, Immediate::Index(0)),
            insn(0x21, Immediate::Index(0)), // saved inside loop -> invalidated
            insn(0x41, Immediate::I32(1)),
            insn(0x24, Immediate::Index(0)),
            insn(0x41, Immediate::I32(1)),
            insn(0x0d, Immediate::Index(0)), // backedge with g0 mutated!
            insn(0x20, Immediate::Index(0)),
            insn(0x24, Immediate::Index(0)),
            insn(0x0b, Immediate::None),
            insn(0x0b, Immediate::None),
        ];
        assert!(!function_preserves_global(
            &bad_loop_backedge,
            1,
            &types,
            &types,
            &ci_index,
            &ci_targets,
            &noreturn,
            &preserves,
            0
        ));

        // Calling non-preserving f2 fails unless g0 is restored from saved local afterwards.
        let bad_call = [insn(0x10, Immediate::Index(2)), insn(0x0b, Immediate::None)];
        assert!(!function_preserves_global(
            &bad_call,
            1,
            &types,
            &types,
            &ci_index,
            &ci_targets,
            &noreturn,
            &preserves,
            0
        ));
        let ok_call_restored = [
            insn(0x23, Immediate::Index(0)),
            insn(0x21, Immediate::Index(0)),
            insn(0x10, Immediate::Index(2)),
            insn(0x20, Immediate::Index(0)),
            insn(0x24, Immediate::Index(0)),
            insn(0x0b, Immediate::None),
        ];
        assert!(function_preserves_global(
            &ok_call_restored,
            1,
            &types,
            &types,
            &ci_index,
            &ci_targets,
            &noreturn,
            &preserves,
            0
        ));
    }

    #[test]
    fn tarjan_scc_orders_callees_before_callers_and_finds_cycles() {
        // 0 -> 1 -> 2, 1 <-> 3 (cycle), 4 -> 4 (self-loop), 3 -> 2
        let edges = vec![vec![1], vec![2, 3], vec![], vec![1, 2], vec![4]];
        let sccs = tarjan_scc(&edges);
        let pos = |u: usize| sccs.iter().position(|c| c.contains(&u)).unwrap();
        // Callees appear before callers in reverse topological order.
        assert!(pos(2) < pos(1));
        assert!(pos(1) < pos(0));
        // 1 and 3 form a 2-node SCC.
        let mut cycle = sccs[pos(1)].clone();
        cycle.sort_unstable();
        assert_eq!(cycle, vec![1, 3]);
        assert_eq!(sccs[pos(4)], vec![4]);
    }

    /// Minimal valid memory64 Wasm module with:
    /// - 1 memory64 (min=1, max=4)
    /// - g0 (i64 mut, preserved via save/restore -> register-passed `gv0`)
    /// - g1 (i32 mut, written only with itself -> constant-folded out of `Ctx`)
    /// - f0: non-growing loop with 2 loads (gets `if mf` in chunked64)
    /// - f1: growing loop with `memory.grow` and 2 loads (must NOT get `if mf`)
    fn sample_mem64_wasm() -> Vec<u8> {
        fn section(id: u8, payload: &[u8]) -> Vec<u8> {
            let mut out = vec![id];
            leb_u32(&mut out, payload.len() as u32);
            out.extend_from_slice(payload);
            out
        }
        fn leb_u32(out: &mut Vec<u8>, mut v: u32) {
            loop {
                let mut b = (v & 0x7f) as u8;
                v >>= 7;
                if v != 0 {
                    b |= 0x80;
                }
                out.push(b);
                if v == 0 {
                    break;
                }
            }
        }
        let mut wasm = b"\0asm\x01\0\0\0".to_vec();
        // Type section: 1 type () -> ()
        wasm.extend(section(1, &[1, 0x60, 0, 0]));
        // Function section: 2 functions of type 0
        wasm.extend(section(3, &[2, 0, 0]));
        // Memory section: 1 memory64, flags=5 (memory64 + max), min=1, max=4
        wasm.extend(section(5, &[1, 0x05, 1, 4]));
        // Global section: g0 = mut i64 0, g1 = mut i32 7
        wasm.extend(section(
            6,
            &[2, 0x7e, 1, 0x42, 0, 0x0b, 0x7f, 1, 0x41, 7, 0x0b],
        ));
        // Export section: "_start" -> func 0, "memory" -> mem 0
        let mut exp = vec![2];
        exp.push(6);
        exp.extend_from_slice(b"_start");
        exp.extend_from_slice(&[0x00, 0]);
        exp.push(6);
        exp.extend_from_slice(b"memory");
        exp.extend_from_slice(&[0x02, 0]);
        wasm.extend(section(7, &exp));

        // f0 body: 1 local i64; save g0, bump g0, loop with 2 i64.load, restore g0, set g1 = g1, call f1.
        let f0 = vec![
            1, 1, 0x7e, // 1 local: l0: i64
            0x23, 0, 0x21, 0, // l0 = g0
            0x23, 0, 0x42, 8, 0x7c, 0x24, 0, // g0 = g0 + 8
            0x03, 0x40, // loop (void)
            0x42, 0, 0x29, 0x03, 0, 0x1a, // drop (i64.load align=8 off=0 (0))
            0x42, 8, 0x29, 0x03, 0, 0x1a, // drop (i64.load align=8 off=0 (8))
            0x0b, // end loop
            0x20, 0, 0x24, 0, // g0 = l0
            0x23, 1, 0x24, 1, // g1 = g1 (constant-init)
            0x10, 1, // call f1
            0x0b,
        ];
        // f1 body: 0 locals; loop with memory.grow and 2 loads (must NOT specialize under `if mf`).
        let f1 = vec![
            0, // 0 locals
            0x03, 0x40, // loop (void)
            0x42, 0, 0x40, 0, 0x1a, // drop (memory.grow 0)
            0x42, 0, 0x29, 0x03, 0, 0x1a, // drop (i64.load 0)
            0x42, 8, 0x29, 0x03, 0, 0x1a, // drop (i64.load 8)
            0x0b, // end loop
            0x0b,
        ];
        let mut code = vec![2];
        leb_u32(&mut code, f0.len() as u32);
        code.extend_from_slice(&f0);
        leb_u32(&mut code, f1.len() as u32);
        code.extend_from_slice(&f1);
        wasm.extend(section(10, &code));
        wasm
    }

    #[test]
    fn generate_emits_cap64_chunked_regions_and_global_analyses() {
        let wasm = sample_mem64_wasm();
        let rt = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("rt");
        let root =
            std::env::temp_dir().join(format!("wtime-aot-codegen-test-{}", std::process::id()));
        let flat_dir = root.join("flat");
        let chunk_dir = root.join("chunked");
        let _ = std::fs::remove_dir_all(&root);

        generate(&wasm, &flat_dir, "test-flat", &rt, "test.wasm", true).unwrap();
        let core_flat = std::fs::read_to_string(flat_dir.join("core/src/lib.rs")).unwrap();
        let fns_flat = std::fs::read_to_string(flat_dir.join("p0/src/f0.rs")).unwrap();
        assert!(core_flat.contains("pub const CAP64: usize = 262144;"));
        assert!(core_flat.contains("pub g0: i64,"));
        // g1 is proven constant (only written with itself), so omitted from Ctx.
        assert!(!core_flat.contains("pub g1:"));
        // g0 is preserved across all functions -> register-passed `mut gv0: i64`.
        assert!(fns_flat.contains("mut gv0: i64"));
        // Flat64 uses fixed-capacity m64f:: accesses.
        assert!(fns_flat.contains("m64f::l64::<CAP64>"));
        assert!(!fns_flat.contains("if mf {"));

        generate(&wasm, &chunk_dir, "test-chunked", &rt, "test.wasm", false).unwrap();
        let core_chunk = std::fs::read_to_string(chunk_dir.join("core/src/lib.rs")).unwrap();
        let fns_chunk = std::fs::read_to_string(chunk_dir.join("p0/src/f0.rs")).unwrap();
        assert!(!core_chunk.contains("CAP64"));
        // f0's non-growing loop specializes under `if mf { ... m64::fl64 ... } else { ... m64::l64 ... }`.
        let f0_idx = fns_chunk.find("pub fn f0<M>").unwrap();
        let f1_idx = fns_chunk.find("pub fn f1<M>").unwrap();
        let f0_src = &fns_chunk[f0_idx..f1_idx];
        let f1_src = &fns_chunk[f1_idx..];
        assert!(f0_src.contains("if mf {"));
        assert!(f0_src.contains("m64::fl64("));
        assert!(f0_src.contains("m64::l64("));
        // f1's loop executes `memory.grow`, so it must NOT specialize under `if mf`.
        assert!(!f1_src.contains("if mf {"));

        let _ = std::fs::remove_dir_all(&root);
        let _ = ValType::I32;
    }

    /// Minimal valid Blot-style 32-bit Wasm module exercising:
    /// - static funcref table (6 targets for `call_indirect` -> O(1) static table)
    /// - mutable externref table (`table.get` / `table.set` / `ref.null` / `ref.is_null`)
    /// - multi-value `(i32, i32)` return (`RetAbi::Pair32` -> `(u64, Option<Trap>)`)
    /// - `br_table`
    /// - global caching ROI + dirty-tracking spill elimination
    /// - `Target::BlotCdylib` C-ABI generation
    #[test]
    fn generate_blot_cdylib_emits_ci_table_scalarpair_and_dirty_tracking() {
        fn section(id: u8, payload: &[u8]) -> Vec<u8> {
            let mut out = vec![id];
            leb_u32(&mut out, payload.len() as u32);
            out.extend_from_slice(payload);
            out
        }
        fn leb_u32(out: &mut Vec<u8>, mut v: u32) {
            loop {
                let mut b = (v & 0x7f) as u8;
                v >>= 7;
                if v != 0 {
                    b |= 0x80;
                }
                out.push(b);
                if v == 0 {
                    break;
                }
            }
        }
        let mut wasm = b"\0asm\x01\0\0\0".to_vec();
        // Type section:
        // t0: (externref) -> ()          [main]
        // t1: (i32) -> (i32, i32)        [pair32 indirect targets]
        // t2: (i32) -> (i32)             [blot:allocate]
        // t3: (externref, i32) -> (i32)  [blot:host/1 import]
        wasm.extend(section(
            1,
            &[
                4, 0x60, 1, 0x6f, 0, // t0
                0x60, 1, 0x7f, 2, 0x7f, 0x7f, // t1
                0x60, 1, 0x7f, 1, 0x7f, // t2
                0x60, 2, 0x6f, 0x7f, 1, 0x7f, // t3
            ],
        ));
        // Import section: 1 import "blot:host/1"."call_array_f32_array_f32" : t3
        let mut imp = vec![1];
        imp.push(11);
        imp.extend_from_slice(b"blot:host/1");
        imp.push(24);
        imp.extend_from_slice(b"call_array_f32_array_f32");
        imp.extend_from_slice(&[0x00, 3]);
        wasm.extend(section(2, &imp));
        // Function section: f1(t0 main), f2..f6(t1 targets), f7(t2 allocate)
        wasm.extend(section(3, &[7, 0, 1, 1, 1, 1, 1, 2]));
        // Table section: table 0 = funcref min=8 max=8; table 1 = externref min=1 max=1
        wasm.extend(section(4, &[2, 0x70, 0x01, 8, 8, 0x6f, 0x01, 1, 1]));
        // Memory section: 1 fixed 32-bit memory min=1 max=1
        wasm.extend(section(5, &[1, 0x01, 1, 1]));
        // Global section: g0 = mut i32 0 (mutated by f2..f6), g1 = mut i32 0 (rarely read in f1)
        wasm.extend(section(
            6,
            &[2, 0x7f, 1, 0x41, 0, 0x0b, 0x7f, 1, 0x41, 0, 0x0b],
        ));
        // Export section: "main" -> f1, "blot:allocate" -> f7, "blot:memory" -> mem 0
        let mut exp = vec![3];
        exp.push(4);
        exp.extend_from_slice(b"main");
        exp.extend_from_slice(&[0x00, 1]);
        exp.push(13);
        exp.extend_from_slice(b"blot:allocate");
        exp.extend_from_slice(&[0x00, 7]);
        exp.push(11);
        exp.extend_from_slice(b"blot:memory");
        exp.extend_from_slice(&[0x02, 0]);
        wasm.extend(section(7, &exp));
        // Element section: table 0 at offset 1 -> [f2, f3, f4, f5, f6] (5 targets => static table!)
        wasm.extend(section(9, &[1, 0x00, 0x41, 1, 0x0b, 5, 2, 3, 4, 5, 6]));

        // f1 (main): stores param externref in table 1, checks ref.is_null,
        // reads g1 once, calls call_indirect t1 twice (with br_table), calls import 0.
        let f1: Vec<u8> = vec![
            0, // 0 locals
            0x41, 0, 0x20, 0, 0x26, 1, // table.set 1 (0, p0)
            0x41, 0, 0x25, 1, 0xd1, 0x1a, // drop (ref.is_null (table.get 1 0))
            0xd0, 0x6f, 0xd1, 0x1a, // drop (ref.is_null (ref.null extern))
            0x23, 1, 0x10, 7, 0x1a, // drop (call f7 (global.get 1))
            0x02, 0x40, // block
            0x41, 0, 0x0e, 1, 0, 0,    // br_table [0] 0
            0x0b, // end
            0x41, 1, 0x41, 1, 0x11, 1, 0, 0x1a, 0x1a, // call_indirect t1 tbl 0
            0x41, 2, 0x41, 2, 0x11, 1, 0, 0x1a, 0x1a, // call_indirect t1 tbl 0
            0x41, 0, 0x25, 1, 0x41, 0, 0x10, 0, 0x1a, // call import0
            0x0b,
        ];
        // f2..f6: bump g0 and g1, return (p0, g0)
        let target_body = |k: u8| -> Vec<u8> {
            vec![
                0, 0x23, 0, 0x41, k, 0x6a, 0x24, 0, // g0 += k
                0x23, 1, 0x41, 1, 0x6a, 0x24, 1, // g1 += 1
                0x20, 0, 0x23, 0, // return (p0, g0)
                0x0b,
            ]
        };
        // f7 (blot:allocate): return p0
        let f7: Vec<u8> = vec![0, 0x20, 0, 0x0b];
        let bodies = [
            f1,
            target_body(1),
            target_body(2),
            target_body(3),
            target_body(4),
            target_body(5),
            f7,
        ];
        let mut code = vec![bodies.len() as u8];
        for b in &bodies {
            leb_u32(&mut code, b.len() as u32);
            code.extend_from_slice(b);
        }
        wasm.extend(section(10, &code));

        let rt = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("rt");
        let root = std::env::temp_dir().join(format!("wtime-aot-blot-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        generate_target(
            &wasm,
            &root,
            "test-blot",
            &rt,
            "test.wasm",
            false,
            Target::BlotCdylib,
        )
        .unwrap();

        let lib_src = std::fs::read_to_string(root.join("src/lib.rs")).unwrap();
        let core_src = std::fs::read_to_string(root.join("core/src/lib.rs")).unwrap();
        let fns_src = std::fs::read_to_string(root.join("p0/src/f0.rs")).unwrap();
        // O(1) static fn-pointer table emitted for 5-target call_indirect
        assert!(lib_src.contains("static CI0_TBL: [Option<Ci0Fn>; 5]"));
        assert!(lib_src.contains("pub extern \"C\" fn blot_aot_call_export"));
        // Externref table field emitted in Ctx (heap-allocated Box<[u64]>)
        assert!(core_src.contains("pub t1: Box<[u64]>,"));
        // ScalarPair & Pair32 return ABIs used for void / 2x32-bit pairs, R<i32> for i32
        assert!(fns_src.contains("-> Option<Trap>"));
        assert!(fns_src.contains("-> (u64, Option<Trap>)"));
        assert!(fns_src.contains("-> R<i32>"));
        // ROI check: f1 reads g1 once across 3 mutating calls, so g1 is NOT cached in f1
        let f1_start = fns_src.find("pub fn f1<M>").unwrap();
        let f2_start = fns_src.find("pub fn f2<M>").unwrap();
        let f1_src = &fns_src[f1_start..f2_start];
        assert!(!f1_src.contains("let mut gv1:"));
        assert!(f1_src.contains("ctx.g1"));

        let _ = std::fs::remove_dir_all(&root);
    }
}
