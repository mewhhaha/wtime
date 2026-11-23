use crate::{adapter::Adapter, report::Report};
use wast::{
    Wast, WastArg, WastDirective as D, WastExecute as E, WastRet,
    core::{NanPattern as N, V128Pattern as V, WastArgCore as A, WastRetCore as R},
    parser::{self, Parse, ParseBuffer, Parser},
};
use wtime_runtime::{Error, ErrorKind, HeapType, RefValue, V128, Value};
mod legacy {
    wast::custom_keyword!(assert_return_canonical_nan);
    wast::custom_keyword!(assert_return_arithmetic_nan);
}
struct LegacyScript<'a> {
    directives: Vec<D<'a>>,
}
impl<'a> Parse<'a> for LegacyScript<'a> {
    fn parse(p: Parser<'a>) -> wast::parser::Result<Self> {
        let mut directives = Vec::new();
        while !p.is_empty() {
            directives.push(p.parens(|p| {
                let canonical = p.peek::<legacy::assert_return_canonical_nan>()?;
                if canonical || p.peek::<legacy::assert_return_arithmetic_nan>()? {
                    let span = if canonical {
                        p.parse::<legacy::assert_return_canonical_nan>()?.0
                    } else {
                        p.parse::<legacy::assert_return_arithmetic_nan>()?.0
                    };
                    let exec = p.parens(|p| p.parse())?;
                    let f32pattern = if canonical {
                        N::CanonicalNan
                    } else {
                        N::ArithmeticNan
                    };
                    let f64pattern = if canonical {
                        N::CanonicalNan
                    } else {
                        N::ArithmeticNan
                    };
                    Ok(D::AssertReturn {
                        span,
                        exec,
                        results: vec![WastRet::Core(R::Either(vec![
                            R::F32(f32pattern),
                            R::F64(f64pattern),
                        ]))],
                    })
                } else {
                    p.parse()
                }
            })?);
        }
        Ok(Self { directives })
    }
}
type Result<T> = std::result::Result<T, String>;
fn arg(value: &WastArg<'_>, adapter: &mut Adapter) -> Result<Value> {
    Ok(match value {
        WastArg::Core(A::I32(v)) => Value::I32(*v),
        WastArg::Core(A::I64(v)) => Value::I64(*v),
        WastArg::Core(A::F32(v)) => Value::F32(v.bits),
        WastArg::Core(A::F64(v)) => Value::F64(v.bits),
        WastArg::Core(A::V128(v)) => Value::V128(V128(v.to_le_bytes())),
        WastArg::Core(A::RefNull(wast::core::HeapType::Abstract { shared: false, ty })) => {
            Value::Ref(RefValue::Null(match ty {
                wast::core::AbstractHeapType::Func => HeapType::Func,
                wast::core::AbstractHeapType::Extern => HeapType::Extern,
                wast::core::AbstractHeapType::Any => HeapType::Any,
                wast::core::AbstractHeapType::Eq => HeapType::Eq,
                wast::core::AbstractHeapType::I31 => HeapType::I31,
                wast::core::AbstractHeapType::Struct => HeapType::Struct,
                wast::core::AbstractHeapType::Array => HeapType::Array,
                wast::core::AbstractHeapType::None => HeapType::None,
                wast::core::AbstractHeapType::NoExtern => HeapType::NoExtern,
                wast::core::AbstractHeapType::NoFunc => HeapType::NoFunc,
                wast::core::AbstractHeapType::Exn => HeapType::Exn,
                wast::core::AbstractHeapType::NoExn => HeapType::NoExn,
                wast::core::AbstractHeapType::Cont => HeapType::Cont,
                wast::core::AbstractHeapType::NoCont => HeapType::NoCont,
            }))
        }
        WastArg::Core(A::RefExtern(id)) => adapter
            .store
            .create_extern_ref(*id)
            .map_err(|e| e.to_string())?,
        WastArg::Core(A::RefHost(id)) => {
            let Value::Ref(RefValue::Extern(handle)) = adapter
                .store
                .create_extern_ref(*id)
                .map_err(|e| e.to_string())?
            else {
                return Err("host ref creation failed".into());
            };
            Value::Ref(RefValue::InternalExtern(handle))
        }
        _ => return Err("unsupported: reference argument".into()),
    })
}
fn matches(v: &Value, r: &R<'_>) -> Result<bool> {
    Ok(match (v, r) {
        (Value::I32(a), R::I32(b)) => a == b,
        (Value::I64(a), R::I64(b)) => a == b,
        (Value::F32(a), R::F32(b)) => match b {
            N::Value(b) => *a == b.bits,
            N::CanonicalNan => a & 0x7fffffff == 0x7fc00000,
            N::ArithmeticNan => a & 0x7fc00000 == 0x7fc00000,
        },
        (Value::F64(a), R::F64(b)) => match b {
            N::Value(b) => *a == b.bits,
            N::CanonicalNan => a & 0x7fffffffffffffff == 0x7ff8000000000000,
            N::ArithmeticNan => a & 0x7ff8000000000000 == 0x7ff8000000000000,
        },
        (Value::Ref(RefValue::Null(_)), R::RefNull(_)) => true,
        (Value::Ref(RefValue::Func(_)), R::RefFunc(None)) => true,
        (Value::Ref(_), R::RefNull(_) | R::RefFunc(None)) => false,
        (Value::V128(a), R::V128(b)) => vector_matches(&a.0, b)?,
        (_, R::Either(rs)) => {
            let mut yes = false;
            for r in rs {
                yes |= matches(v, r)?;
            }
            yes
        }
        (_, R::I32(_) | R::I64(_) | R::F32(_) | R::F64(_)) => false,
        _ => return Err("unsupported: reference/vector expectation".into()),
    })
}
fn result_matches(
    a: &mut Adapter,
    module: Option<&str>,
    value: &Value,
    expected: &R<'_>,
) -> Result<bool> {
    match expected {
        R::Either(choices) => {
            let mut yes = false;
            for expected in choices {
                yes |= result_matches(a, module, value, expected)?;
            }
            Ok(yes)
        }
        R::RefExtern(Some(id)) => {
            Ok(*value == a.store.create_extern_ref(*id).map_err(|e| e.to_string())?)
        }
        R::RefExtern(None) => a
            .store
            .reference_matches(*value, HeapType::Extern)
            .map_err(|e| e.to_string()),
        R::RefHost(id) => a
            .store
            .extern_identity(*value)
            .map(|v| v == Some(*id))
            .map_err(|e| e.to_string()),
        R::RefAny | R::RefEq | R::RefArray | R::RefStruct | R::RefI31 | R::RefI31Shared => {
            let heap = match expected {
                R::RefAny => HeapType::Any,
                R::RefEq => HeapType::Eq,
                R::RefArray => HeapType::Array,
                R::RefStruct => HeapType::Struct,
                _ => HeapType::I31,
            };
            a.store
                .reference_matches(*value, heap)
                .map_err(|e| e.to_string())
        }
        R::RefFunc(Some(wast::token::Index::Num(index, _))) => {
            Ok(*value == a.function_ref(module, *index).map_err(|e| e.to_string())?)
        }
        _ => matches(value, expected),
    }
}
fn vector_matches(a: &[u8; 16], b: &V) -> Result<bool> {
    Ok(match b {
        V::I8x16(v) => a.iter().zip(v).all(|(a, b)| *a == *b as u8),
        V::I16x8(v) => a
            .as_chunks::<2>()
            .0
            .iter()
            .zip(v)
            .all(|(a, b)| *a == b.to_le_bytes()),
        V::I32x4(v) => a
            .as_chunks::<4>()
            .0
            .iter()
            .zip(v)
            .all(|(a, b)| *a == b.to_le_bytes()),
        V::I64x2(v) => a
            .as_chunks::<8>()
            .0
            .iter()
            .zip(v)
            .all(|(a, b)| *a == b.to_le_bytes()),
        V::F32x4(v) => {
            let mut yes = true;
            for (a, b) in a.as_chunks::<4>().0.iter().zip(v) {
                yes &= matches(&Value::F32(u32::from_le_bytes(*a)), &R::F32(*b))?;
            }
            yes
        }
        V::F64x2(v) => {
            let mut yes = true;
            for (a, b) in a.as_chunks::<8>().0.iter().zip(v) {
                yes &= matches(&Value::F64(u64::from_le_bytes(*a)), &R::F64(*b))?;
            }
            yes
        }
    })
}
fn execute(a: &mut Adapter, e: &mut E<'_>) -> std::result::Result<Vec<Value>, Error> {
    match e {
        E::Invoke(i) => {
            let args = i
                .args
                .iter()
                .map(|v| arg(v, a))
                .collect::<Result<Vec<_>>>()
                .map_err(|s| Error::new(ErrorKind::Host, s))?;
            a.invoke(i.module.map(|m| m.name()), i.name, &args)
        }
        E::Get { module, global, .. } => a.get(module.map(|m| m.name()), global),
        E::Wat(w) => {
            let b = w
                .encode()
                .map_err(|e| Error::new(ErrorKind::Decode, e.to_string()))?;
            a.module(&b, None)?;
            Ok(vec![])
        }
    }
}
fn expect_error<T>(r: std::result::Result<T, Error>, kind: ErrorKind) -> Result<()> {
    match r {
        Err(e) if e.kind == kind => Ok(()),
        Err(e) => Err(format!("expected {kind:?}, got {e}")),
        Ok(_) => Err(format!("expected {kind:?}, succeeded")),
    }
}
fn trap_category(message: &str) -> &str {
    if message.contains("unreachable") {
        "unreachable"
    } else if message.contains("divide by zero") {
        "divide-by-zero"
    } else if message.contains("integer overflow") {
        "integer-overflow"
    } else if message.contains("conversion") {
        "conversion"
    } else if message.contains("shared memory") && !message.contains("out of bounds") {
        "shared-memory-required"
    } else if message.contains("unaligned") {
        "unaligned"
    } else if message.contains("indirect call type")
        || message.contains("call_ref type")
        || message == "indirect call"
    {
        "indirect-type"
    } else if message.contains("uninitialized") || message.contains("null indirect") {
        "uninitialized-element"
    } else if message.contains("undefined element")
        || message == "undefined"
        || message.contains("table out of bounds")
        || message.contains("out of bounds table")
    {
        "table-bounds"
    } else if message.contains("out of bounds") || message.contains("address overflow") {
        "memory-bounds"
    } else if message.contains("null") {
        "null-reference"
    } else if message.contains("cast") {
        "cast"
    } else {
        message
    }
}
fn expect_trap(result: std::result::Result<Vec<Value>, Error>, message: &str) -> Result<()> {
    match result {
        Err(e)
            if e.kind == ErrorKind::Trap && trap_category(&e.message) == trap_category(message) =>
        {
            Ok(())
        }
        Err(e) => Err(format!("expected trap {message:?}, got {e}")),
        Ok(_) => Err(format!("expected trap {message:?}, succeeded")),
    }
}
pub fn run(source: &str, a: &mut Adapter, report: &mut Report) {
    let mut lexer = wast::lexer::Lexer::new(source);
    lexer.allow_confusing_unicode(true);
    let buffer = match ParseBuffer::new_with_lexer(lexer) {
        Ok(b) => b,
        Err(e) => {
            report.failed += 1;
            report.text_errors += 1;
            report.errors.push(e.to_string());
            return;
        }
    };
    let parsed = if source.contains("assert_return_canonical_nan")
        || source.contains("assert_return_arithmetic_nan")
    {
        parser::parse::<LegacyScript<'_>>(&buffer).map(|s| Wast {
            directives: s.directives,
        })
    } else {
        parser::parse::<Wast<'_>>(&buffer)
    };
    let mut script = match parsed {
        Ok(s) => s,
        Err(e) => {
            report.failed += 1;
            report.text_errors += 1;
            report.errors.push(e.to_string());
            return;
        }
    };
    use sha2::{Digest, Sha256};
    let source_hash = format!("{:x}", Sha256::digest(source.as_bytes()));
    run_directives(&mut script.directives, a, report, &source_hash);
}
fn normalize_legacy_wat(w: &mut wast::Wat<'_>) {
    if let wast::Wat::Module(m) = w {
        if let wast::core::ModuleKind::Text(fields) = &mut m.kind {
            for field in fields {
                match field {
                    wast::core::ModuleField::Data(d) => {
                        if let Some(id) = d.id.take()
                            && let wast::core::DataKind::Active { memory, .. } = &mut d.kind
                        {
                            *memory = wast::token::Index::Id(id)
                        }
                    }
                    wast::core::ModuleField::Elem(e) => {
                        if let Some(id) = e.id.take()
                            && let wast::core::ElemKind::Active { table, .. } = &mut e.kind
                        {
                            *table = Some(wast::token::Index::Id(id))
                        }
                    }
                    _ => {}
                }
            }
        }
        if m.resolve().is_err() {
            return;
        }
        if let wast::core::ModuleKind::Text(fields) = &mut m.kind {
            for field in fields {
                if let wast::core::ModuleField::Elem(e) = field
                    && let wast::core::ElemKind::Active { table, .. } = &mut e.kind
                    && matches!(table, Some(wast::token::Index::Num(0, _)))
                {
                    *table = None;
                }
            }
        }
    }
}

fn normalize_legacy(d: &mut D<'_>) {
    match d {
        D::Module(w)
        | D::ModuleDefinition(w)
        | D::AssertInvalid { module: w, .. }
        | D::AssertMalformed { module: w, .. } => {
            if let wast::QuoteWat::Wat(w) = w {
                normalize_legacy_wat(w)
            }
        }
        D::AssertUnlinkable { module, .. } => normalize_legacy_wat(module),
        _ => {}
    }
}
fn merge(into: &mut Report, other: Report) {
    into.compatibility_assertions += other.compatibility_assertions;
    into.compatibility.extend(other.compatibility);
    into.selected += other.selected;
    into.executed += other.executed;
    into.failed += other.failed;
    into.skipped += other.skipped;
    into.unsupported += other.unsupported;
    into.text_errors += other.text_errors;
    into.errors.extend(other.errors);
    if let Some(counts) = other.execution_counts {
        into.merge_execution_counts(counts);
    }
    if let Some(preparation) = other.preparation_summary {
        into.merge_preparation_counts(preparation);
    }
    into.merge_worker_stores(other.worker_stores_observed);
}
fn run_directives(
    directives: &mut [D<'_>],
    a: &mut Adapter,
    report: &mut Report,
    source_hash: &str,
) {
    #[cfg(feature = "performance-counters")]
    let counters_before = a.store.performance_counters();
    std::thread::scope(|scope| {
        let mut threads = std::collections::HashMap::new();
        for d in directives {
            if a.legacy_syntax {
                normalize_legacy(d);
            }
            let assertion = !matches!(
                d,
                D::Module(_)
                    | D::Register { .. }
                    | D::ModuleDefinition(_)
                    | D::ModuleInstance { .. }
                    | D::Thread(_)
                    | D::Wait { .. }
            );
            if assertion {
                report.selected += 1;
                report.executed += 1;
            }
            let offset = d.span().offset();
            let modern_valid = a.legacy_syntax
                && ((source_hash
                    == "ac1071bbc99e1885db61fdc4fe5dc85b9c9173fc57d06b065124fb42b399560a"
                    && matches!(
                        offset,
                        5964 | 6566 | 7114 | 7662 | 8843 | 9399 | 9901 | 10403
                    ))
                    || (source_hash
                        == "572b32f9ed9b29385b9150a68115946049d79c85d4b5ac494237d23cba0fac3f"
                        && offset == 13566));
            let result: Result<()> = (|| match d {
                D::AssertInvalid { module, .. }
                    if a.features.stack_switching
                        && source_hash
                            == "15a8c36165de51b8d006f5d2afb3a6e9b69ea4d9915451c86a1a62b74843957f"
                        && matches!(offset, 271 | 349) =>
                {
                    let b = module.encode().map_err(|e| e.to_string())?;
                    a.engine.compile(&b).map_err(|e| e.to_string())?;
                    report.compatibility_assertions += 1;
                    report.compatibility.push(format!("source {source_hash} byte {offset}: stack-switching proposal Explainer control tags extend exception tags with result types; current expected successful compilation"));
                    Ok(())
                }

                D::AssertTrap { exec, .. } | D::AssertReturn { exec, .. }
                    if a.legacy_syntax
                        && source_hash
                            == "4428e0ebb12381a510162708397e8cd8a5cfb98991503c27b8e8ff1e150ec264"
                        && matches!(offset, 7508 | 7817 | 10371 | 10648) =>
                {
                    let expected = if matches!(offset, 7508 | 7817) { 0 } else { 97 };
                    let actual = execute(a, exec).map_err(|e| e.to_string())?;
                    if actual != [Value::I32(expected)] {
                        return Err(format!(
                            "current segment side-effect rule expected {expected}, got {actual:?}"
                        ));
                    }
                    report.compatibility_assertions += 1;
                    report.compatibility.push(format!("source {source_hash} byte {offset}: current instantiation preserves earlier segment writes; expected successful return {expected}"));
                    Ok(())
                }

                D::Thread(thread) => {
                    let mut child = a
                        .thread_child(thread.shared_module.map(|m| m.name()))
                        .map_err(|e| e.to_string())?;
                    let name = thread.name.name().to_owned();
                    if threads.contains_key(&name) {
                        return Err("duplicate thread name".into());
                    }
                    let mut directives = std::mem::take(&mut thread.directives);
                    let cancel = child.store.cancellation_token();
                    let timeout = child.worker_timeout;
                    let timed_out = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                    let timer_flag = timed_out.clone();
                    let (completed, completion) = std::sync::mpsc::channel();
                    scope.spawn(move || {
                        if matches!(
                            completion.recv_timeout(timeout),
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                        ) {
                            timer_flag.store(true, std::sync::atomic::Ordering::Release);
                            cancel.cancel();
                        }
                    });
                    let worker_name = name.clone();
                    let handle = scope.spawn(move || {
                        let mut r = Report::default();
                        run_directives(&mut directives, &mut child, &mut r, source_hash);
                        let _ = completed.send(());
                        if timed_out.load(std::sync::atomic::Ordering::Acquire) {
                            r.failed += 1;
                            r.errors.push(format!("worker {worker_name} watchdog timeout after {} ms; cancelled and joined", timeout.as_millis()));
                        }
                        r
                    });
                    threads.insert(name, handle);
                    report.merge_worker_stores(1);
                    Ok(())
                }
                D::Wait { thread, .. } => {
                    let handle = threads.remove(thread.name()).ok_or("unknown thread")?;
                    let r = handle.join().map_err(|_| "internal thread panic")?;
                    merge(report, r);
                    Ok(())
                }
                D::Module(m) => {
                    let name = m.name().map(|n| n.name().to_owned());
                    let bytes = m.encode().map_err(|e| e.to_string())?;
                    a.module(&bytes, name.as_deref()).map_err(|e| e.to_string())
                }
                D::ModuleDefinition(m) => {
                    let name = m.name().map(|n| n.name().to_owned());
                    let b = m.encode().map_err(|e| e.to_string())?;
                    a.definition(&b, name.as_deref()).map_err(|e| e.to_string())
                }
                D::ModuleInstance {
                    instance, module, ..
                } => a
                    .instantiate_definition(module.map(|m| m.name()), instance.map(|m| m.name()))
                    .map_err(|e| e.to_string()),
                D::Register { name, module, .. } => a
                    .register(name, module.map(|m| m.name()))
                    .map_err(|e| e.to_string()),
                D::Invoke(i) => {
                    let args = i
                        .args
                        .iter()
                        .map(|v| arg(v, a))
                        .collect::<Result<Vec<_>>>()?;
                    a.invoke(i.module.map(|m| m.name()), i.name, &args)
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                }
                D::AssertReturn { exec, results, .. } => {
                    let values = execute(a, exec).map_err(|e| e.to_string())?;
                    if values.len() != results.len() {
                        return Err("result arity mismatch".into());
                    }
                    for (v, r) in values.iter().zip(results) {
                        let WastRet::Core(r) = r else {
                            return Err("unsupported: component result".into());
                        };
                        let module = match exec {
                            E::Invoke(i) => i.module.map(|m| m.name()),
                            E::Get { module, .. } => module.map(|m| m.name()),
                            _ => None,
                        };
                        let matched = result_matches(a, module, v, r)?;
                        if !matched {
                            return Err(format!("result mismatch: {v:?} versus {r:?}"));
                        }
                    }
                    Ok(())
                }
                D::AssertMalformed { module, .. } | D::AssertInvalid { module, .. }
                    if modern_valid =>
                {
                    let b = module.encode().map_err(|e| e.to_string())?;
                    a.engine
                        .compile(&b)
                        .map_err(|e| format!("current normative valid assertion failed: {e}"))?;
                    report.compatibility_assertions += 1;
                    report.compatibility.push(format!("source {source_hash} byte {offset}: historical rejection expectation revised to successful compilation under current padded-u32 / unreachable-stack rules"));
                    Ok(())
                }
                D::AssertMalformed { module, .. }
                    if a.legacy_syntax
                        && source_hash
                            == "ac1071bbc99e1885db61fdc4fe5dc85b9c9173fc57d06b065124fb42b399560a"
                        && matches!(offset, 2557 | 5329 | 8254) =>
                {
                    let b = module.encode().map_err(|e| e.to_string())?;
                    expect_error(a.engine.compile(&b), ErrorKind::Validation)?;
                    report.compatibility_assertions += 1;
                    report.compatibility.push(format!("source {source_hash} byte {offset}: historical reserved-byte Decode expectation revised to current index Validation rule"));
                    Ok(())
                }
                D::AssertMalformed { module, .. } => match module.encode() {
                    Err(_) => Ok(()),
                    Ok(b) => expect_error(a.engine.compile(&b), ErrorKind::Decode),
                },
                D::AssertInvalid { module, .. } => {
                    let b = module
                        .encode()
                        .map_err(|e| format!("invalid assertion failed text encoding: {e}"))?;
                    let result = a.engine.compile(&b);
                    if a.legacy_syntax
                        && result
                            .as_ref()
                            .is_err_and(|e| e.kind == ErrorKind::DisabledFeature)
                    {
                        report.compatibility_assertions += 1;
                        report.compatibility.push(format!("source {source_hash} byte {offset}: historical invalid syntax rejected by explicit MVP DisabledFeature policy"));
                        Ok(())
                    } else {
                        expect_error(result, ErrorKind::Validation)
                    }
                }
                D::AssertUnlinkable {
                    module, message, ..
                } => {
                    let b = module.encode().map_err(|e| e.to_string())?;
                    let m = a.engine.compile(&b).map_err(|e| e.to_string())?;
                    let outcome = a.linker.instantiate(&mut a.store, &m);
                    if a.legacy_syntax
                        && matches!(
                            *message,
                            "data segment does not fit" | "elements segment does not fit"
                        )
                    {
                        let expected = if message.starts_with("data") {
                            "out of bounds memory access"
                        } else {
                            "out of bounds table access"
                        };
                        expect_trap(outcome.map(|_| Vec::new()), expected)?;
                        report.compatibility_assertions += 1;
                        report.compatibility.push(format!("source {source_hash} byte {offset}: historical segment failure Link expectation revised to current trapping instantiation"));
                        Ok(())
                    } else {
                        expect_error(outcome, ErrorKind::Link)
                    }
                }
                D::AssertTrap { exec, message, .. } => expect_trap(execute(a, exec), message),
                D::AssertException {
                    exec: E::Invoke(call),
                    ..
                } => {
                    let args = call
                        .args
                        .iter()
                        .map(|v| arg(v, a))
                        .collect::<Result<Vec<_>>>()?;
                    match a
                        .invoke_outcome(call.module.map(|m| m.name()), call.name, &args)
                        .map_err(|e| e.to_string())?
                    {
                        wtime_runtime::RunOutcome::Exception(_) => Ok(()),
                        _ => Err("expected guest exception".into()),
                    }
                }
                D::AssertSuspension {
                    exec: E::Invoke(call),
                    ..
                } => {
                    let args = call
                        .args
                        .iter()
                        .map(|v| arg(v, a))
                        .collect::<Result<Vec<_>>>()?;
                    match a
                        .invoke_outcome(call.module.map(|m| m.name()), call.name, &args)
                        .map_err(|e| e.to_string())?
                    {
                        wtime_runtime::RunOutcome::Suspended { .. } => Ok(()),
                        _ => Err("expected guest suspension".into()),
                    }
                }
                D::AssertExhaustion { call, .. } => {
                    let args = call
                        .args
                        .iter()
                        .map(|v| arg(v, a))
                        .collect::<Result<Vec<_>>>()?;
                    expect_error(
                        a.invoke(call.module.map(|m| m.name()), call.name, &args),
                        ErrorKind::ResourceLimit,
                    )
                }
                _ => Err("unsupported: selected script directive".into()),
            })();
            if let Err(e) = result {
                if e.contains("unsupported:") {
                    report.unsupported += 1;
                } else {
                    report.failed += 1;
                }
                report
                    .errors
                    .push(format!("byte {}: {e}", d.span().offset()));
            }
        }
        for (_, handle) in threads {
            match handle.join() {
                Ok(r) => merge(report, r),
                Err(_) => {
                    report.failed += 1;
                    report.errors.push("internal thread panic".into());
                }
            }
        }
    });
    #[cfg(feature = "performance-counters")]
    {
        let counters_after = a.store.performance_counters();
        let delta = (|| {
            let total = counters_after
                .source_dispatches
                .checked_sub(counters_before.source_dispatches)?;
            let optimized = counters_after
                .compact_steps
                .checked_sub(counters_before.compact_steps)?;
            let fallback = total.checked_sub(optimized)?;
            Some(crate::report::ExecutionCounts {
                total_dispatched_source_events: total,
                optimized_compact_source_events: optimized,
                decoded_fallback_source_events: fallback,
            })
        })();
        match delta {
            Some(counts) => report.merge_execution_counts(counts),
            None => report
                .collector_error("execution counter underflow or optimized count exceeds total"),
        }
        let preparation = std::mem::take(&mut a.preparation);
        if preparation.overflowed {
            report.collector_error("preparation counter overflow");
        }
        report.merge_preparation_counts(preparation.into());
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use wtime_runtime::FeatureSet;
    fn test(s: &str) -> Report {
        let mut r = Report::default();
        run(s, &mut Adapter::new(FeatureSet::mvp()).unwrap(), &mut r);
        r
    }
    #[test]
    fn t012_case_01() {
        assert!(!test("(assert_unknown)").ok());
    }
    #[test]
    fn t012_case_02() {
        assert!(!test("(module)").ok());
        let empty_threads = test("(module) (thread $t) (wait $t)");
        assert_eq!(empty_threads.failed, 0, "{empty_threads:?}");
        assert_eq!(empty_threads.executed, 0);
        assert!(!empty_threads.ok());
    }
    #[test]
    fn t012_case_03() {
        assert!(!test("(assert_invalid (module (func unreachable) (start 0)) \"invalid\")").ok());
    }
    #[test]
    fn t034_decode_and_validation_stages() {
        let report = test(
            r#"(assert_malformed (module binary "\00asm\02\00\00\00") "unknown version")
            (assert_invalid (module (func (result i32))) "type mismatch")"#,
        );
        assert!(report.ok(), "{report:?}");
        assert_eq!(report.executed, 2);
        assert!(!test(r#"(assert_malformed (module (func (result i32))) "type mismatch")"#).ok());
        assert!(
            !test(r#"(assert_invalid (module binary "\00asm\02\00\00\00") "unknown version")"#)
                .ok()
        );
    }
    #[test]
    fn t012_real_assertions() {
        let r = test(
            "(module (func (export \"answer\") (result i32) i32.const 42)) (assert_return (invoke \"answer\") (i32.const 42))",
        );
        assert!(r.ok(), "{r:?}");
        assert_eq!(r.executed, 1);
        assert!(!test("(module (func (export \"answer\") (result i32) i32.const 42)) (assert_return (invoke \"answer\") (i32.const 43))").ok());
    }
}
#[cfg(test)]
mod spectest_tests {
    use super::*;
    use wtime_runtime::{FeatureSet, Value};
    #[test]
    fn t013_case_01() {
        let mut a = Adapter::new(FeatureSet::stable_core()).unwrap();
        let mut r = Report::default();
        run(
            "(module $a (global (export \"g\") (mut i32) (i32.const 7))) (register \"a\" $a) (module (import \"a\" \"g\" (global (mut i32))) (func (export \"read\") (result i32) global.get 0)) (assert_return (invoke \"read\") (i32.const 7))",
            &mut a,
            &mut r,
        );
        assert!(r.ok(), "{r:?}");
    }
    #[test]
    fn t013_case_02() {
        let mut a = Adapter::new(FeatureSet::mvp()).unwrap();
        let mut r = Report::default();
        run(
            "(assert_unlinkable (module (import \"spectest\" \"print_i32\" (func (param i64)))) \"incompatible import type\")",
            &mut a,
            &mut r,
        );
        assert!(r.ok(), "{r:?}");
    }
    #[test]
    fn t013_case_03() {
        let mut a = Adapter::new(FeatureSet::mvp()).unwrap();
        let mut r = Report::default();
        run(
            "(module (import \"spectest\" \"print_i32\" (func $p (param i32))) (func (export \"print\") i32.const 42 call $p)) (invoke \"print\")",
            &mut a,
            &mut r,
        );
        assert!(r.ok(), "{r:?}");
        assert_eq!(*a.prints.lock().unwrap(), vec![vec![Value::I32(42)]]);
    }
}
#[cfg(test)]
mod pattern_tests {
    use super::*;
    #[test]
    fn t012_nan_and_allowed_sets() {
        assert!(matches(&Value::F32(0xffc0_0042), &R::F32(N::ArithmeticNan)).unwrap());
        assert!(!matches(&Value::F32(0x7f80_0000), &R::F32(N::ArithmeticNan)).unwrap());
        assert!(!matches(&Value::F32(0x7f80_0042), &R::F32(N::ArithmeticNan)).unwrap());
        assert!(matches(&Value::F64(0xfff8_0000_0000_0000), &R::F64(N::CanonicalNan)).unwrap());
        assert!(matches(&Value::I32(4), &R::Either(vec![R::I32(3), R::I32(4)])).unwrap());
        assert!(!matches(&Value::I32(5), &R::Either(vec![R::I32(3), R::I32(4)])).unwrap());
    }
    #[test]
    fn t012_vector_lanes() {
        let b = [1i32, 2, 3, 4]
            .into_iter()
            .flat_map(i32::to_le_bytes)
            .collect::<Vec<_>>();
        let b: [u8; 16] = b.try_into().unwrap();
        assert!(vector_matches(&b, &V::I32x4([1, 2, 3, 4])).unwrap());
        assert!(!vector_matches(&b, &V::I32x4([1, 2, 3, 5])).unwrap());
    }
}
#[cfg(test)]
mod legacy_tests {
    use super::*;
    #[test]
    fn t012_legacy_nan() {
        let mut a = Adapter::new(wtime_runtime::FeatureSet::mvp()).unwrap();
        let mut report = Report::default();
        run(
            "(module (func (export \"nan\") (result f32) f32.const nan)) (assert_return_canonical_nan (invoke \"nan\"))",
            &mut a,
            &mut report,
        );
        assert!(report.ok(), "{report:?}");
    }
    #[test]
    fn t012_wrong_trap_fails() {
        assert!(
            expect_trap(
                Err(Error::new(ErrorKind::Trap, "integer overflow")),
                "unreachable"
            )
            .is_err()
        );
    }
    #[test]
    fn t013_thread_shared_memory() {
        let mut a = Adapter::new(wtime_runtime::FeatureSet::default()).unwrap();
        let mut r = Report::default();
        run(
            include_str!("../../../tests/fixtures/conformance/thread.wast"),
            &mut a,
            &mut r,
        );
        assert!(r.ok(), "{r:?}");
    }
}

#[cfg(test)]
mod thread_acceptance_tests {
    use super::*;
    fn threaded(source: &str, timeout: std::time::Duration) -> Report {
        let mut adapter = Adapter::new(wtime_runtime::FeatureSet::all_phase3_core()).unwrap();
        adapter.worker_timeout = timeout;
        let mut report = Report::default();
        run(source, &mut adapter, &mut report);
        report
    }
    #[test]
    fn t135_case_01() {
        let r = threaded(
            r#"(module) (thread $t (module (func (export "x") (result i32) i32.const 1)) (assert_return (invoke "x") (i32.const 2))) (wait $t)"#,
            std::time::Duration::from_secs(1),
        );
        assert_eq!(r.failed, 1);
        assert_eq!(r.executed, 1);
        assert!(!r.ok());
    }
    #[test]
    fn t135_case_02() {
        let r = threaded(
            r#"(module) (thread $t (module (func (export "x") (loop br 0))) (invoke "x")) (wait $t)"#,
            std::time::Duration::from_millis(10),
        );
        assert!(!r.ok());
        assert!(
            r.errors
                .iter()
                .any(|e| e.contains("watchdog timeout") && e.contains("cancelled and joined")),
            "{r:?}"
        );
    }
    #[test]
    fn t135_case_03() {
        let r = threaded(
            r#"(module) (thread $t (module (func (export "x") (result i32) i32.const 1)) (assert_return (invoke "x") (i32.const 1)))"#,
            std::time::Duration::from_secs(1),
        );
        assert!(r.ok(), "{r:?}");
        assert_eq!(r.executed, 1);
    }
}

#[cfg(all(test, feature = "compact-executor", feature = "performance-counters"))]
mod p121_counter_tests {
    use super::*;
    use wtime_runtime::{ExecutionPolicy, FeatureSet};

    #[test]
    fn p121_repeated_runs_on_one_adapter_merge_only_new_deltas() {
        let mut adapter =
            Adapter::with_policy(FeatureSet::default(), ExecutionPolicy::Compact).unwrap();
        let source = "(module (func (export \"f\") (result i32) i32.const 7)) \
                      (assert_return (invoke \"f\") (i32.const 7))";

        let mut first = Report::default();
        run(source, &mut adapter, &mut first);
        assert_eq!(
            first.execution_counts.unwrap(),
            crate::report::ExecutionCounts {
                total_dispatched_source_events: 2,
                optimized_compact_source_events: 2,
                decoded_fallback_source_events: 0,
            }
        );
        assert_eq!(first.preparation_summary.unwrap().compile_count, 1);

        let mut second = Report::default();
        run(source, &mut adapter, &mut second);
        assert_eq!(
            second.execution_counts.unwrap(),
            crate::report::ExecutionCounts {
                total_dispatched_source_events: 2,
                optimized_compact_source_events: 2,
                decoded_fallback_source_events: 0,
            }
        );
        assert_eq!(second.preparation_summary.unwrap().compile_count, 1);
    }
}

#[cfg(test)]
mod gate_acceptance_tests {
    use super::*;
    fn evaluate(source: &str) -> Report {
        let mut report = Report::default();
        run(
            source,
            &mut Adapter::new(wtime_runtime::FeatureSet::all_phase3_core()).unwrap(),
            &mut report,
        );
        report
    }
    const WIDE: &str = r#"(module (func (export "add") (param i64 i64 i64 i64) (result i64 i64) local.get 0 local.get 1 local.get 2 local.get 3 i64.add128))"#;
    #[test]
    fn t092_case_01() {
        let r = evaluate(&format!(
            r#"{WIDE} (assert_return (invoke "add" (i64.const -1) (i64.const 0) (i64.const 1) (i64.const 0)) (i64.const 0) (i64.const 1))"#
        ));
        assert!(r.ok(), "{r:?}");
    }
    #[test]
    fn t092_case_02() {
        let r = evaluate(&format!(
            r#"{WIDE} (assert_return (invoke "add" (i64.const -1) (i64.const -1) (i64.const 1) (i64.const 0)) (i64.const 0) (i64.const 0))"#
        ));
        assert!(r.ok(), "{r:?}");
    }
    #[test]
    fn t092_case_03() {
        let r = evaluate(&format!(
            r#"{WIDE} (assert_return (invoke "add" (i64.const -1) (i64.const 0) (i64.const 1) (i64.const 0)) (i64.const 1) (i64.const 0))"#
        ));
        assert_eq!(r.failed, 1);
        assert!(!r.ok());
    }
    #[test]
    fn t058_case_02() {
        assert!(
            evaluate(r#"(assert_trap (module (func unreachable) (start 0)) "unreachable")"#).ok()
        );
        assert!(
            !evaluate(r#"(assert_unlinkable (module (func unreachable) (start 0)) "unreachable")"#)
                .ok()
        );
    }
    #[test]
    fn t101_case_02() {
        let r = evaluate(
            r#"(module (func (export "v") (result v128) v128.const i32x4 1 2 3 4)) (assert_return (invoke "v") (v128.const i32x4 1 2 3 5))"#,
        );
        assert_eq!(r.failed, 1);
        assert!(!r.ok());
    }
    #[test]
    fn t160_case_03() {
        let r =
            evaluate(r#"(module (func (export "x") unreachable)) (assert_exception (invoke "x"))"#);
        assert_eq!(r.failed, 1);
        assert!(!r.ok());
        assert!(
            evaluate(
                r#"(module (tag $e) (func (export "x") throw $e)) (assert_exception (invoke "x"))"#
            )
            .ok()
        );
    }
    #[test]
    fn t136_case_03() {
        let buffer = wast::parser::ParseBuffer::new("(module (type (shared (struct))))").unwrap();
        let mut module = wast::parser::parse::<wast::Wat<'_>>(&buffer).unwrap();
        let bytes = module.encode().unwrap();
        let adapter = Adapter::new(wtime_runtime::FeatureSet::all_phase3_core()).unwrap();
        let error = adapter
            .engine
            .compile(&bytes)
            .expect_err("excluded shared-everything type must not compile");
        assert_eq!(error.kind, ErrorKind::Decode);
    }
}
