//! Run under a host address-space limit to test recoverable allocator failure.
#![forbid(unsafe_code)]
use wtime_runtime::{Config, Engine, Linker, Store};
use wtime_types::{ErrorKind, FeatureSet, IndexType, Limits, MemoryType};

fn leb(out: &mut Vec<u8>, mut n: u32) {
    loop {
        let b = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            out.push(b);
            break;
        }
        out.push(b | 0x80);
    }
}

fn section(out: &mut Vec<u8>, id: u8, payload: &[u8]) {
    out.push(id);
    leb(out, payload.len() as u32);
    out.extend_from_slice(payload);
}

fn guest_table_module(min_elements: u32) -> Vec<u8> {
    let mut payload = vec![1, 0x70, 0x00]; // 1 funcref table, min only
    leb(&mut payload, min_elements);
    let mut wasm = b"\0asm\x01\0\0\0".to_vec();
    section(&mut wasm, 4, &payload);
    wasm
}

fn passive_element_module(seg_count: usize, elems_per_seg: usize) -> Vec<u8> {
    let mut wasm = b"\0asm\x01\0\0\0".to_vec();
    section(&mut wasm, 1, &[1, 0x60, 0, 0]); // type () -> ()
    section(&mut wasm, 3, &[1, 0]); // func 0
    // Element section: `seg_count` passive funcref segments (`flags = 1`, elemkind `0x00`, vec(funcidx 0))
    let mut elem = Vec::new();
    leb(&mut elem, seg_count as u32);
    for _ in 0..seg_count {
        elem.push(0x01); // passive, elemkind + func indices
        elem.push(0x00); // funcref
        leb(&mut elem, elems_per_seg as u32);
        elem.resize(elem.len() + elems_per_seg, 0x00);
    }
    section(&mut wasm, 9, &elem);
    section(&mut wasm, 10, &[1, 2, 0, 0x0b]); // empty func 0
    wasm
}

fn recursive_locals_module(local_count: u32) -> Vec<u8> {
    let mut wasm = b"\0asm\x01\0\0\0".to_vec();
    section(&mut wasm, 1, &[1, 0x60, 0, 0]); // type () -> ()
    section(&mut wasm, 3, &[1, 0]); // func 0 -> type 0
    section(&mut wasm, 7, &[1, 3, b'r', b'e', b'c', 0, 0]); // export "rec"
    let mut body = vec![1]; // 1 local decl
    leb(&mut body, local_count);
    body.push(0x7e); // i64
    body.extend_from_slice(&[0x10, 0x00, 0x0b]); // call 0; end
    let mut code = vec![1];
    leb(&mut code, body.len() as u32);
    code.extend_from_slice(&body);
    section(&mut wasm, 10, &code);
    wasm
}

fn nested_blocks_recursive_module(depth: usize) -> Vec<u8> {
    let mut wasm = b"\0asm\x01\0\0\0".to_vec();
    section(&mut wasm, 1, &[1, 0x60, 0, 0]);
    section(&mut wasm, 3, &[1, 0]);
    section(&mut wasm, 7, &[1, 3, b'r', b'u', b'n', 0, 0]);
    let mut body = Vec::with_capacity(depth * 3 + 4);
    body.push(0); // 0 locals
    for _ in 0..depth {
        body.extend_from_slice(&[0x02, 0x40]); // block void
    }
    body.extend_from_slice(&[0x10, 0x00]); // call 0
    body.extend(std::iter::repeat_n(0x0b, depth)); // end blocks
    body.push(0x0b); // end func
    let mut code = vec![1];
    leb(&mut code, body.len() as u32);
    code.extend_from_slice(&body);
    section(&mut wasm, 10, &code);
    wasm
}

fn nested_try_table_module(depth: usize, catches_per_try: usize) -> Vec<u8> {
    let mut wasm = b"\0asm\x01\0\0\0".to_vec();
    section(&mut wasm, 1, &[1, 0x60, 0, 0]);
    section(&mut wasm, 3, &[1, 0]);
    section(&mut wasm, 7, &[1, 3, b'r', b'u', b'n', 0, 0]);
    let mut body = Vec::new();
    body.push(0); // 0 locals
    for _ in 0..depth {
        body.extend_from_slice(&[0x1f, 0x40]); // try_table void
        leb(&mut body, catches_per_try as u32);
        for _ in 0..catches_per_try {
            body.extend_from_slice(&[0x02, 0x00]); // catch_all 0
        }
    }
    body.extend_from_slice(&[0x10, 0x00]); // call 0
    body.extend(std::iter::repeat_n(0x0b, depth)); // end try_tables
    body.push(0x0b); // end func
    let mut code = vec![1];
    leb(&mut code, body.len() as u32);
    code.extend_from_slice(&body);
    section(&mut wasm, 10, &code);
    wasm
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = Config {
        features: FeatureSet::all_phase3_core(),
        ..Config::default()
    };
    // Two objects: the failed constructor must not consume one of them.
    config.limits.max_store_objects = 2;
    config.limits.max_memory_pages = 1 << 30;
    config.limits.max_memory_bytes = 1 << 46;
    config.limits.max_store_memory_bytes = 1 << 46;
    let engine = Engine::new(config)?;
    let mut store = Store::new(&engine);

    // 1. Construction. Unshared memory is stored as 64 KiB pages allocated on
    // first write, so creation allocates only the page table. For a 1 TiB
    // memory64 that table is 256 MiB; the external RLIMIT_AS of 128 MiB makes
    // its fallible reservation fail.
    let error = store
        .create_memory(MemoryType {
            limits: Limits::new(1 << 24, Some(1 << 24)),
            index_type: IndexType::I64,
            ..MemoryType::default()
        })
        .expect_err("run this worker with a 128 MiB address-space limit");
    assert_eq!(error.kind, ErrorKind::ResourceLimit);
    assert_eq!(error.message, "memory allocation failed");

    // Recovery, while address space remains. With this memory and the large
    // one below, both permitted objects are used: the failed constructor did
    // not consume a live arena slot or leave a half-object.
    let memory = store.create_memory(MemoryType {
        limits: Limits::new(1, Some(1)),
        ..MemoryType::default()
    })?;
    store.write_memory(memory, 65535, &[42])?;
    assert_eq!(store.read_memory(memory, 65535, 1)?, [42]);

    // 2. Pages. A 512 MiB memory is created without committing its pages;
    // writing one byte to each page must fail recoverably once the address
    // space is exhausted, leaving the bytes already written intact.
    let large = store.create_memory(MemoryType {
        limits: Limits::new(8192, Some(8192)),
        ..MemoryType::default()
    })?;
    let mut written = 0u64;
    let page_error = loop {
        if written == 8192 {
            panic!("all 512 MiB were written: run this worker with a 128 MiB address-space limit");
        }
        match store.write_memory(large, written * 65536, &[(written % 251) as u8 + 1]) {
            Ok(()) => written += 1,
            Err(error) => break error,
        }
    };
    assert_eq!(page_error.kind, ErrorKind::ResourceLimit);
    assert_eq!(page_error.message, "memory allocation failed");
    assert!(written > 0, "no page could be written");
    for page in [0, written / 2, written - 1] {
        assert_eq!(
            store.read_memory(large, page * 65536, 1)?,
            [(page % 251) as u8 + 1]
        );
    }
    assert_eq!(store.read_memory(large, written * 65536, 1)?, [0]);
    drop(store);
    drop(engine);

    // 3. Guest-defined table instantiation on a populated store. Pre-populate
    // an existing table so rollback exercises live store state under 128 MiB
    // RLIMIT_AS; a 16M-element funcref table needs ~512 MiB and must fail
    // recoverably while leaving the store usable for a small module.
    let mut tcfg = Config {
        features: FeatureSet::all_phase3_core(),
        ..Config::default()
    };
    tcfg.limits.max_table_elements = 1 << 24;
    tcfg.limits.max_store_table_elements = 1 << 24;
    let tengine = Engine::new(tcfg)?;
    let mut tstore = Store::new(&tengine);
    let existing_table_mod = tengine.compile(&guest_table_module(4096))?;
    Linker::new(&tengine).instantiate(&mut tstore, &existing_table_mod)?;
    let huge_table_mod = tengine.compile(&guest_table_module(1 << 24))?;
    let table_err = Linker::new(&tengine)
        .instantiate(&mut tstore, &huge_table_mod)
        .expect_err("16M-element table must fail under 128 MiB RLIMIT_AS");
    assert_eq!(table_err.kind, ErrorKind::ResourceLimit);
    let small_table_mod = tengine.compile(&guest_table_module(4))?;
    Linker::new(&tengine).instantiate(&mut tstore, &small_table_mod)?;

    // 4. Passive element segment initialization under tight memory pressure.
    // A 200k-element module uses ~6 MiB per instance; repeatedly instantiating
    // into the same store under 128 MiB RLIMIT_AS must hit fallible element
    // reservation and leave the store healthy for a small module.
    let elem_bytes = passive_element_module(4, 50_000);
    let elem_mod = tengine.compile(&elem_bytes)?;
    let mut elem_failed = false;
    for _ in 0..64 {
        match Linker::new(&tengine).instantiate(&mut tstore, &elem_mod) {
            Ok(_) => {}
            Err(err) => {
                assert_eq!(err.kind, ErrorKind::ResourceLimit);
                elem_failed = true;
                break;
            }
        }
    }
    assert!(
        elem_failed,
        "repeated passive element instantiation must hit fallible allocation limit under 128 MiB RLIMIT_AS"
    );
    drop(tstore);
    drop(tengine);

    // Verify recovery with element OOM followed immediately by small instantiation in same store.
    let mut ecfg = Config {
        features: FeatureSet::all_phase3_core(),
        ..Config::default()
    };
    ecfg.limits.max_instructions = 1_000_000;
    let eengine = Engine::new(ecfg)?;
    let mut estore = Store::new(&eengine);
    let emod = eengine.compile(&elem_bytes)?;
    let mut kept = 0usize;
    loop {
        match Linker::new(&eengine).instantiate(&mut estore, &emod) {
            Ok(_) => {
                kept += 1;
                assert!(
                    kept < 64,
                    "element instantiation should hit OOM under 128 MiB"
                );
            }
            Err(err) => {
                assert_eq!(err.kind, ErrorKind::ResourceLimit);
                break;
            }
        }
    }
    let ok_mod = eengine.compile(&guest_table_module(2))?;
    Linker::new(&eengine).instantiate(&mut estore, &ok_mod)?;
    drop(estore);
    drop(eengine);

    // 5. Active frame local-slot budget & real OS fallible frame allocation.
    let rec_bytes = recursive_locals_module(50_000);
    for max_total_locals in [1_000_000usize, 100_000_000usize] {
        let mut fcfg = Config::default();
        fcfg.limits.max_total_locals = max_total_locals;
        let fengine = Engine::new(fcfg)?;
        let mut fstore = Store::new(&fengine);
        let rec_mod = fengine.compile(&rec_bytes)?;
        let rec_inst = Linker::new(&fengine).instantiate(&mut fstore, &rec_mod)?;
        let wtime_runtime::External::Func(rec_fn) = rec_inst.export(&fstore, "rec")? else {
            panic!("expected exported func rec");
        };
        let frame_err = fstore
            .invoke(rec_fn, &[])
            .expect_err("recursive 50k-local module must fail with ResourceLimit");
        assert_eq!(frame_err.kind, ErrorKind::ResourceLimit);
        let ok_mod = fengine.compile(&guest_table_module(2))?;
        Linker::new(&fengine).instantiate(&mut fstore, &ok_mod)?;
    }

    // 6. Control label stack & try_table catch-list OOM protection.
    // Exercise both deeply nested blocks around `call 0` (hitting default
    // active_labels budget and real OS try_reserve OOM) and nested `try_table`
    // with large catch lists (zero-copy catch table + bounded labels).
    let block_bytes = nested_blocks_recursive_module(5_000);
    for max_total_locals in [1_000_000usize, 100_000_000usize] {
        let mut lcfg = Config::default();
        lcfg.limits.max_total_locals = max_total_locals;
        lcfg.limits.max_store_memory_bytes = 1 << 36;
        let lengine = Engine::new(lcfg)?;
        let mut lstore = Store::new(&lengine);
        let lmod = lengine.compile(&block_bytes)?;
        let linst = Linker::new(&lengine).instantiate(&mut lstore, &lmod)?;
        let wtime_runtime::External::Func(lfn) = linst.export(&lstore, "run")? else {
            panic!("expected exported func run");
        };
        let label_err = lstore
            .invoke(lfn, &[])
            .expect_err("deeply nested recursive blocks must fail with ResourceLimit");
        assert_eq!(label_err.kind, ErrorKind::ResourceLimit);
        let ok_mod = lengine.compile(&guest_table_module(2))?;
        Linker::new(&lengine).instantiate(&mut lstore, &ok_mod)?;
    }

    let try_bytes = nested_try_table_module(50, 1_000);
    {
        let tcfg2 = Config {
            features: FeatureSet::all_phase3_core(),
            ..Config::default()
        };
        let tengine2 = Engine::new(tcfg2)?;
        let mut tstore2 = Store::new(&tengine2);
        let tmod2 = tengine2.compile(&try_bytes)?;
        let tinst2 = Linker::new(&tengine2).instantiate(&mut tstore2, &tmod2)?;
        let wtime_runtime::External::Func(tfn2) = tinst2.export(&tstore2, "run")? else {
            panic!("expected exported func run");
        };
        let try_err = tstore2
            .invoke(tfn2, &[])
            .expect_err("recursive try_table with large catch lists must fail with ResourceLimit");
        assert_eq!(try_err.kind, ErrorKind::ResourceLimit);
        let ok_mod = tengine2.compile(&guest_table_module(2))?;
        Linker::new(&tengine2).instantiate(&mut tstore2, &ok_mod)?;
    }

    println!(
        "{{\"allocator_failed\":true,\"page_allocation_failed\":true,\"guest_table_allocation_failed\":true,\"element_allocation_failed\":true,\"frame_allocation_failed\":true,\"label_allocation_failed\":true,\"pages_written\":{written},\"recovery_passed\":true,\"executed\":1}}"
    );
    Ok(())
}
