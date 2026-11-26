use wtime_runtime::*;
fn instance(bytes: &[u8]) -> (Store, Instance) {
    let e = Engine::new(Config::default()).unwrap();
    let m = e.compile(bytes).unwrap();
    let mut s = Store::new(&e);
    let i = Linker::new(&e).instantiate(&mut s, &m).unwrap();
    (s, i)
}
fn invoke(s: &mut Store, i: Instance, n: &str, args: &[Value]) -> Result<Vec<Value>> {
    let External::Func(f) = i.export(s, n)? else {
        panic!("expected function")
    };
    match s.invoke(f, args)? {
        RunOutcome::Returned(v) => Ok(v),
        other => panic!("unexpected {other:?}"),
    }
}
#[test]
fn t086_byte_pages_and_growth() {
    let (mut s, i) = instance(include_bytes!("fixtures/memory-proposals.wasm"));
    assert_eq!(
        invoke(&mut s, i, "byte-size", &[]).unwrap(),
        [Value::I32(4)]
    );
    assert_eq!(
        invoke(&mut s, i, "byte-grow", &[Value::I32(3)]).unwrap(),
        [Value::I32(4)]
    );
    assert_eq!(
        invoke(&mut s, i, "byte-size", &[]).unwrap(),
        [Value::I32(7)]
    );
    assert_eq!(
        invoke(&mut s, i, "byte-grow", &[Value::I32(10)]).unwrap(),
        [Value::I32(-1)]
    );
    let External::Memory(m) = i.export(&s, "bytes").unwrap() else {
        panic!()
    };
    assert_eq!(s.read_memory(m, 0, 7).unwrap(), [0; 7]);
}
#[test]
fn t082_memory64_addresses_and_width() {
    let (mut s, i) = instance(include_bytes!("fixtures/memory-proposals.wasm"));
    assert_eq!(
        invoke(&mut s, i, "wide-size", &[]).unwrap(),
        [Value::I64(1)]
    );
    assert_eq!(
        invoke(&mut s, i, "write-read", &[Value::I64(3), Value::I64(-7)]).unwrap(),
        [Value::I64(-7)]
    );
    assert_eq!(
        invoke(&mut s, i, "write-read", &[Value::I64(-1), Value::I64(0)])
            .unwrap_err()
            .kind,
        ErrorKind::Trap
    );
    assert_eq!(
        invoke(&mut s, i, "wide-grow", &[Value::I64(1)]).unwrap(),
        [Value::I64(1)]
    );
    assert_eq!(
        invoke(&mut s, i, "wide-grow", &[Value::I64(i64::MAX)]).unwrap(),
        [Value::I64(-1)]
    );
}
#[test]
fn t083_mixed_width_copy_is_prechecked() {
    let (mut s, i) = instance(include_bytes!("fixtures/memory-proposals.wasm"));
    invoke(
        &mut s,
        i,
        "write-read",
        &[Value::I64(0), Value::I64(0x01020304)],
    )
    .unwrap();
    invoke(
        &mut s,
        i,
        "copy",
        &[Value::I32(0), Value::I64(0), Value::I32(4)],
    )
    .unwrap();
    let External::Memory(m) = i.export(&s, "bytes").unwrap() else {
        panic!()
    };
    assert_eq!(s.read_memory(m, 0, 4).unwrap(), [4, 3, 2, 1]);
    assert_eq!(
        invoke(
            &mut s,
            i,
            "copy",
            &[Value::I32(1), Value::I64(0), Value::I32(4)]
        )
        .unwrap_err()
        .kind,
        ErrorKind::Trap
    );
    assert_eq!(s.read_memory(m, 0, 4).unwrap(), [4, 3, 2, 1]);
}
#[test]
fn t092_wide_arithmetic_public_execution() {
    let (mut s, i) = instance(include_bytes!("fixtures/wide.wasm"));
    assert_eq!(
        invoke(
            &mut s,
            i,
            "add",
            &[Value::I64(-1), Value::I64(0), Value::I64(1), Value::I64(0)]
        )
        .unwrap(),
        [Value::I64(0), Value::I64(1)]
    );
    assert_eq!(
        invoke(&mut s, i, "mul-u", &[Value::I64(-1), Value::I64(-1)]).unwrap(),
        [Value::I64(1), Value::I64(-2)]
    );
}

fn leb(out: &mut Vec<u8>, mut v: u32) {
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

fn sec(out: &mut Vec<u8>, id: u8, payload: &[u8]) {
    out.push(id);
    leb(out, payload.len() as u32);
    out.extend_from_slice(payload);
}

#[test]
fn t093_gc_array_init_and_copy_respect_budget_without_double_charging() {
    // Module with:
    //   type 0: (array (mut i32))
    //   type 1: (array (mut funcref))
    //   type 2: () -> i32
    //   passive data 0: 64 little-endian i32s (1..=64)
    //   passive elem 0: 64 funcrefs to func 0
    //   func 0 ("init_data"): array.new_default 0 (64), array.init_data 0 0, returns sum (= 2080)
    //   func 1 ("init_elem"): array.new_default 1 (64), array.init_elem 1 0, returns non-null count (= 64)
    //   func 2 ("copy_distinct"): two arrays of 32 i32s, init_data first 32 into src, array.copy src->dst, returns sum (= 528)

    let mut wasm = vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
    // Type section: 3 types
    let mut ty = Vec::new();
    leb(&mut ty, 3);
    // type 0: (array (mut i32))
    ty.extend_from_slice(&[0x5e, 0x7f, 0x01]);
    // type 1: (array (mut funcref))
    ty.extend_from_slice(&[0x5e, 0x70, 0x01]);
    // type 2: (func (result i32))
    ty.extend_from_slice(&[0x60, 0x00, 0x01, 0x7f]);
    sec(&mut wasm, 1, &ty);

    // Function section: 3 functions of type 2
    sec(&mut wasm, 3, &[3, 2, 2, 2]);

    // Export section
    let mut exp = Vec::new();
    leb(&mut exp, 3);
    for (name, idx) in [("init_data", 0u32), ("init_elem", 1), ("copy_distinct", 2)] {
        leb(&mut exp, name.len() as u32);
        exp.extend_from_slice(name.as_bytes());
        exp.push(0x00);
        leb(&mut exp, idx);
    }
    sec(&mut wasm, 7, &exp);

    // Element section: 1 passive funcref segment with 64 entries pointing to func 0
    let mut elem = Vec::new();
    leb(&mut elem, 1);
    elem.push(0x01); // passive, func indices
    elem.push(0x00); // elemkind funcref
    leb(&mut elem, 64);
    for _ in 0..64 {
        leb(&mut elem, 0);
    }
    sec(&mut wasm, 9, &elem);

    // DataCount section: 1
    sec(&mut wasm, 12, &[1]);

    // Code section
    let mut code = Vec::new();
    leb(&mut code, 3);

    // func 0: init_data
    let mut f0 = Vec::new();
    // locals: 1 (ref null 0), 2 i32 (idx, sum)
    f0.extend_from_slice(&[2, 1, 0x63, 0x00, 2, 0x7f]);
    // local 0 = array.new_default 0 (len = 64)
    f0.extend_from_slice(&[0x41, 0xc0, 0x00, 0xfb, 0x07, 0x00, 0x21, 0x00]);
    // array.init_data type=0 data=0 (dst=local 0, d_off=0, s_off=0, len=64)
    f0.extend_from_slice(&[
        0x20, 0x00, 0x41, 0x00, 0x41, 0x00, 0x41, 0xc0, 0x00, 0xfb, 0x12, 0x00, 0x00,
    ]);
    // loop sum
    f0.extend_from_slice(&[
        0x03, 0x40, // loop
        0x20, 0x02, // sum
        0x20, 0x00, 0x20, 0x01, 0xfb, 0x0b, 0x00, // array.get 0
        0x6a, 0x21, 0x02, // sum += elem
        0x20, 0x01, 0x41, 0x01, 0x6a, 0x22, 0x01, // idx += 1
        0x41, 0xc0, 0x00, 0x49, // idx <u 64
        0x0d, 0x00, // br_if 0
        0x0b, // end loop
        0x20, 0x02, 0x0b,
    ]);
    leb(&mut code, f0.len() as u32);
    code.extend_from_slice(&f0);

    // func 1: init_elem
    let mut f1 = Vec::new();
    // locals: 1 (ref null 1), 2 i32 (idx, count)
    f1.extend_from_slice(&[2, 1, 0x63, 0x01, 2, 0x7f]);
    // local 0 = array.new_default 1 (len = 64)
    f1.extend_from_slice(&[0x41, 0xc0, 0x00, 0xfb, 0x07, 0x01, 0x21, 0x00]);
    // array.init_elem type=1 elem=0 (dst=local 0, d_off=0, s_off=0, len=64)
    f1.extend_from_slice(&[
        0x20, 0x00, 0x41, 0x00, 0x41, 0x00, 0x41, 0xc0, 0x00, 0xfb, 0x13, 0x01, 0x00,
    ]);
    // loop count non-null
    f1.extend_from_slice(&[
        0x03, 0x40, // loop
        0x20, 0x02, // count
        0x20, 0x00, 0x20, 0x01, 0xfb, 0x0b, 0x01, // array.get 1
        0xd1, 0x45, // !ref.is_null
        0x6a, 0x21, 0x02, // count += ...
        0x20, 0x01, 0x41, 0x01, 0x6a, 0x22, 0x01, 0x41, 0xc0, 0x00, 0x49, 0x0d, 0x00, 0x0b, 0x20,
        0x02, 0x0b,
    ]);
    leb(&mut code, f1.len() as u32);
    code.extend_from_slice(&f1);

    // func 2: copy_distinct
    let mut f2 = Vec::new();
    // locals: 2 (ref null 0) (src, dst), 2 i32 (idx, sum)
    f2.extend_from_slice(&[2, 2, 0x63, 0x00, 2, 0x7f]);
    // src = array.new_default 0 (32); dst = array.new_default 0 (32)
    f2.extend_from_slice(&[
        0x41, 0x20, 0xfb, 0x07, 0x00, 0x21, 0x00, 0x41, 0x20, 0xfb, 0x07, 0x00, 0x21, 0x01,
    ]);
    // array.init_data src 0 0 32
    f2.extend_from_slice(&[
        0x20, 0x00, 0x41, 0x00, 0x41, 0x00, 0x41, 0x20, 0xfb, 0x12, 0x00, 0x00,
    ]);
    // array.copy dst_type=0 src_type=0 (dst, 0, src, 0, 32)
    f2.extend_from_slice(&[
        0x20, 0x01, 0x41, 0x00, 0x20, 0x00, 0x41, 0x00, 0x41, 0x20, 0xfb, 0x11, 0x00, 0x00,
    ]);
    // sum dst
    f2.extend_from_slice(&[
        0x03, 0x40, 0x20, 0x03, 0x20, 0x01, 0x20, 0x02, 0xfb, 0x0b, 0x00, 0x6a, 0x21, 0x03, 0x20,
        0x02, 0x41, 0x01, 0x6a, 0x22, 0x02, 0x41, 0x20, 0x49, 0x0d, 0x00, 0x0b, 0x20, 0x03, 0x0b,
    ]);
    leb(&mut code, f2.len() as u32);
    code.extend_from_slice(&f2);
    sec(&mut wasm, 10, &code);

    // Passive data segment 0: 64 little-endian i32s (1..=64)
    let mut data = Vec::new();
    leb(&mut data, 1);
    data.push(0x01); // passive
    leb(&mut data, 64 * 4);
    for k in 1..=64i32 {
        data.extend_from_slice(&k.to_le_bytes());
    }
    sec(&mut wasm, 11, &data);

    // Budget sized so ONE 64-element array fits (>50% of budget), so double-charging in
    // `array.init_data`/`array.init_elem` would falsely fail with `ResourceLimit`.
    let one_array_upper = 64 * std::mem::size_of::<Value>() + 256;
    for (export, expected) in [
        ("init_data", 64 * 65 / 2),
        ("init_elem", 64),
        ("copy_distinct", 32 * 33 / 2),
    ] {
        let mut cfg = Config::default();
        cfg.limits.max_gc_bytes = if export == "copy_distinct" {
            one_array_upper + 256
        } else {
            one_array_upper
        };
        let e = Engine::new(cfg).unwrap();
        let m = e.compile(&wasm).unwrap();
        let mut s = Store::new(&e);
        let inst = Linker::new(&e).instantiate(&mut s, &m).unwrap();
        assert_eq!(
            invoke(&mut s, inst, export, &[]).unwrap(),
            [Value::I32(expected)],
            "{export} failed under tight max_gc_bytes"
        );
    }
}

#[test]
fn gc_large_struct_field_access_avoids_type_clone() {
    // Regression: `struct.get`/`struct.set` must borrow `&SubType` instead of cloning the full
    // `StructType::fields` vector on every field access.
    const FIELDS: u32 = 4096;
    const ITERS: i32 = 2000;
    let mut wasm = b"\0asm\x01\0\0\0".to_vec();
    let mut ty = Vec::new();
    leb(&mut ty, 2);
    // type 0: (struct (field (mut i32)) * FIELDS)
    ty.push(0x5f);
    leb(&mut ty, FIELDS);
    for _ in 0..FIELDS {
        ty.extend_from_slice(&[0x7f, 0x01]);
    }
    // type 1: () -> i32
    ty.extend_from_slice(&[0x60, 0x00, 0x01, 0x7f]);
    sec(&mut wasm, 1, &ty);
    sec(&mut wasm, 3, &[1, 1]);
    let mut exp = Vec::new();
    leb(&mut exp, 1);
    exp.push(4);
    exp.extend_from_slice(b"loop");
    exp.extend_from_slice(&[0x00, 0x00]);
    sec(&mut wasm, 7, &exp);

    let mut body = Vec::new();
    // locals: 1 (ref null 0), 1 i32 (counter)
    body.extend_from_slice(&[2, 1, 0x63, 0x00, 1, 0x7f]);
    // local 0 = struct.new_default 0
    body.extend_from_slice(&[0xfb, 0x01, 0x00, 0x21, 0x00]);
    // loop: s.field[4095] += 1; counter += 1; br_if counter < ITERS
    body.extend_from_slice(&[0x03, 0x40]);
    body.extend_from_slice(&[0x20, 0x00, 0x20, 0x00, 0xfb, 0x02, 0x00]);
    leb(&mut body, FIELDS - 1);
    body.extend_from_slice(&[0x41, 0x01, 0x6a, 0xfb, 0x05, 0x00]);
    leb(&mut body, FIELDS - 1);
    body.extend_from_slice(&[0x20, 0x01, 0x41, 0x01, 0x6a, 0x22, 0x01, 0x41]);
    // sleb128(ITERS = 2000 = 0x7d0)
    let mut v = ITERS;
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if (v == 0 && (b & 0x40) == 0) || (v == -1 && (b & 0x40) != 0) {
            body.push(b);
            break;
        }
        body.push(b | 0x80);
    }
    body.extend_from_slice(&[0x49, 0x0d, 0x00, 0x0b]);
    // return struct.get 0 (FIELDS - 1)
    body.extend_from_slice(&[0x20, 0x00, 0xfb, 0x02, 0x00]);
    leb(&mut body, FIELDS - 1);
    body.push(0x0b);
    let mut code = Vec::new();
    leb(&mut code, 1);
    leb(&mut code, body.len() as u32);
    code.extend_from_slice(&body);
    sec(&mut wasm, 10, &code);

    let mut cfg = Config::default();
    cfg.limits.fuel = Some(50_000);
    let e = Engine::new(cfg).unwrap();
    let m = e.compile(&wasm).unwrap();
    let mut s = Store::new(&e);
    let inst = Linker::new(&e).instantiate(&mut s, &m).unwrap();
    assert_eq!(
        invoke(&mut s, inst, "loop", &[]).unwrap(),
        [Value::I32(ITERS)]
    );
}

#[test]
fn untrusted_worker_config_rejects_threads_and_enforces_fuel() {
    let engine = Engine::new(Config::untrusted_worker()).unwrap();
    // Shared memory module (flags=0x03 min=1 max=1) must be rejected because `threads` is off.
    let shared_mem_wasm = include_bytes!("fixtures/feature_profiles/threads.wasm");
    assert_eq!(
        engine.compile(shared_mem_wasm).unwrap_err().kind,
        ErrorKind::DisabledFeature
    );

    // Infinite loop module must stop cleanly with ResourceLimit under the default worker fuel cap.
    let mut loop_wasm = b"\0asm\x01\0\0\0".to_vec();
    sec(&mut loop_wasm, 1, &[1, 0x60, 0, 0]);
    sec(&mut loop_wasm, 3, &[1, 0]);
    sec(&mut loop_wasm, 7, &[1, 4, b's', b'p', b'i', b'n', 0, 0]);
    // body: loop br 0 end end
    sec(
        &mut loop_wasm,
        10,
        &[1, 7, 0, 0x03, 0x40, 0x0c, 0x00, 0x0b, 0x0b],
    );
    let mut cfg = Config::untrusted_worker();
    cfg.limits.fuel = Some(1_000);
    let e = Engine::new(cfg).unwrap();
    let m = e.compile(&loop_wasm).unwrap();
    let mut s = Store::new(&e);
    let inst = Linker::new(&e).instantiate(&mut s, &m).unwrap();
    assert_eq!(
        invoke(&mut s, inst, "spin", &[]).unwrap_err().kind,
        ErrorKind::ResourceLimit
    );
}
