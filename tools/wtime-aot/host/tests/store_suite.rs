//! Comprehensive end-to-end integration tests for `wtime-aot-host` on native x86-64 `StoreCdylib`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use wtime_aot_host::{
    AnyRef, ArrayRef, ArrayRefPre, ArrayType, Caller, Collector, Config, Engine, EqRef, ExnRef,
    ExnRefPre, ExnType, Extern, ExternRef, FieldType, GcEphemeron, GcEphemeronUpdate,
    GcHeapOutOfMemory, I31, Instance, Linker, Memory, MemoryType, Module, Mutability, RootScope,
    Rooted, SharedMemory, StorageType, Store, StoreLimits, StoreLimitsBuilder, StructRef,
    StructRefPre, StructType, Tag, Trap, Val, ValType, WeakAnyRef,
};

fn parse_wat(src: &str) -> Vec<u8> {
    wat::parse_str(src).expect("valid WAT test module")
}

fn default_engine() -> Engine {
    let mut cfg = Config::new();
    cfg.collector(Collector::Copying)
        .wasm_gc(true)
        .wasm_function_references(true)
        .wasm_reference_types(true)
        .wasm_exceptions(true)
        .wasm_threads(true)
        .shared_memory(true)
        .wasm_multi_memory(true)
        .wasm_tail_call(true);
    Engine::new(&cfg).expect("engine")
}

#[test]
fn multi_module_r_and_p_linking_globals_tables_memory_and_multivalue() {
    let engine = default_engine();

    // Module R: defines GC struct `$box`, mutable GC global `$root`, private memory,
    // funcref table, helper functions, and a 5-return function matching Scriptw's completion ABI.
    let r_wasm = parse_wat(
        r#"
        (module
          (type $box (struct (field (mut i32)) (field (mut i64))))
          (type $binop (func (param i32 i32) (result i32)))
          (memory (export "mem") 1 4)
          (global $g_box (export "g_box") (mut (ref null $box)) (ref.null $box))
          (table $tbl (export "tbl") 4 funcref)
          (elem (i32.const 0) func $add_mul)

          (func $add_mul (export "add_mul") (type $binop)
            (i32.add (i32.mul (local.get 0) (i32.const 3)) (local.get 1)))

          (func $make_box (export "make_box") (param i32 i64) (result (ref $box))
            (struct.new $box (local.get 0) (local.get 1)))

          (func $complete5 (export "complete5") (param i32 i64 (ref null eq))
                (result i32 i64 (ref null eq) i32 i32)
            (local.get 0)
            (i64.add (local.get 1) (i64.const 100))
            (local.get 2)
            (i32.const 1)
            (i32.const 7))
        )
        "#,
    );

    // Module P: imports R's memory, global, table, and functions, plus a host callback.
    let p_wasm = parse_wat(
        r#"
        (module
          (type $box (struct (field (mut i32)) (field (mut i64))))
          (type $binop (func (param i32 i32) (result i32)))
          (import "host" "bump" (func $host_bump (param i32) (result i32)))
          (import "rt" "mem" (memory $m 1 4))
          (import "rt" "g_box" (global $g (mut (ref null $box))))
          (import "rt" "tbl" (table $t 4 funcref))
          (import "rt" "make_box" (func $r_make_box (param i32 i64) (result (ref $box))))
          (import "rt" "complete5" (func $r_complete5
            (param i32 i64 (ref null eq)) (result i32 i64 (ref null eq) i32 i32)))

          (func (export "run") (param i32) (result i32 i64 (ref null eq) i32 i32)
            (local $b (ref $box))
            (local $v i32)
            ;; Call host import
            (local.set $v (call $host_bump (local.get 0)))
            ;; Write into R's private memory
            (i32.store (i32.const 64) (local.get $v))
            ;; Cross-module call_indirect via R's table
            (local.set $v (call_indirect $t (type $binop) (local.get $v) (i32.const 5) (i32.const 0)))
            ;; Cross-module GC struct allocation & shared mutable global update
            (local.set $b (call $r_make_box (local.get $v) (i64.const 777)))
            (global.set $g (local.get $b))
            ;; Cross-module tail call returning 5 values!
            (return_call $r_complete5 (local.get $v) (i64.const 23) (local.get $b)))
        )
        "#,
    );

    // Also test Module::serialize -> Module::deserialize round-trip on R!
    let r_mod_orig = Module::new(&engine, &r_wasm).expect("compile R");
    let serialized = r_mod_orig.serialize().expect("serialize R");
    let r_mod = unsafe { Module::deserialize(&engine, &serialized) }.expect("deserialize R");
    let p_mod = Module::new(&engine, &p_wasm).expect("compile P");

    let mut store = Store::new(&engine, 10i32);
    let r_inst = Instance::new(&mut store, &r_mod, &[]).expect("instantiate R");

    let mut linker = Linker::new(&engine);
    linker
        .func_wrap("host", "bump", |mut caller: Caller<'_, i32>, x: i32| {
            *caller.data_mut() += 1;
            x + *caller.data()
        })
        .unwrap();
    for name in ["mem", "g_box", "tbl", "make_box", "complete5"] {
        let exp = r_inst.get_export(&mut store, name).expect("R export");
        linker.define(&store, "rt", name, exp).unwrap();
    }

    let p_inst = linker
        .instantiate(&mut store, &p_mod)
        .expect("instantiate P");
    let run = p_inst
        .get_typed_func::<i32, (i32, i64, Option<Rooted<EqRef>>, i32, i32)>(&mut store, "run")
        .expect("typed run");

    let (tag, scalar, eq_ref, kind, target) = run.call(&mut store, 4).expect("call run");
    // host_bump(4) = 4 + 11 = 15; add_mul(15, 5) = 15*3 + 5 = 50; complete5 -> (50, 123, b, 1, 7)
    assert_eq!(tag, 50);
    assert_eq!(scalar, 123);
    assert_eq!(kind, 1);
    assert_eq!(target, 7);

    // Check R's memory was mutated by P.
    let mem = r_inst.get_memory(&mut store, "mem").unwrap();
    let mut buf = [0u8; 4];
    mem.read(&store, 64, &mut buf).unwrap();
    assert_eq!(i32::from_le_bytes(buf), 15);

    // Check returned GC struct AND R's exported global point to the exact same GC struct!
    let eq_ref = eq_ref.expect("non-null box");
    let st = eq_ref.as_struct(&store).unwrap().expect("is struct");
    assert_eq!(st.field(&mut store, 0).unwrap().unwrap_i32(), 50);
    assert_eq!(st.field(&mut store, 1).unwrap().unwrap_i64(), 777);

    let g_box = r_inst.get_global(&mut store, "g_box").unwrap();
    let g_val = g_box.get(&mut store).unwrap_anyref().expect("global box");
    assert!(Rooted::ref_eq(&store, &g_val, &eq_ref.to_anyref()).unwrap());
}

#[test]
fn reentrant_host_wasm_gc_stack_walking_and_cycles() {
    let mut cfg = Config::new();
    cfg.collector(Collector::Copying)
        .gc_heap_initial_size(64 * 1024);
    let engine = Engine::new(&cfg).unwrap();

    // Module R holds a live struct across a host callback that re-enters P and triggers GC!
    let r_wasm = parse_wat(
        r#"
        (module
          (type $node (struct (field (mut (ref null $node))) (field i32)))
          (import "host" "reenter_and_gc" (func $reenter (param i32) (result i32)))

          (func (export "alloc_chain_and_reenter") (param $depth i32) (result (ref $node) i32)
            (local $a (ref $node))
            (local $b (ref $node))
            (local $sum i32)
            (local.set $a (struct.new $node (ref.null $node) (i32.const 111)))
            (local.set $b (struct.new $node (local.get $a) (i32.const 222)))
            ;; Both `$a` and `$b` are live across this host call which will run Cheney GC
            ;; multiple times AND recursively invoke Wasm again!
            (local.set $sum (call $reenter (local.get $depth)))
            ;; Verify both moved structs survived intact after nested GCs!
            (local.set $sum
              (i32.add (local.get $sum)
                (i32.add (struct.get $node 1 (local.get $a))
                         (struct.get $node 1 (local.get $b)))))
            (local.get $b)
            (local.get $sum))

          ;; Allocates garbage cycles to force automatic in-Wasm Cheney GC!
          (func (export "churn") (param $iters i32) (result i32)
            (local $i i32)
            (local $x (ref null $node))
            (local $y (ref null $node))
            (loop $l
              (local.set $x (struct.new $node (ref.null $node) (local.get $i)))
              (local.set $y (struct.new $node (local.get $x) (local.get $i)))
              (struct.set $node 0 (local.get $x) (local.get $y))
              (local.set $i (i32.add (local.get $i) (i32.const 1)))
              (br_if $l (i32.lt_u (local.get $i) (local.get $iters))))
            (local.get $i))
        )
        "#,
    );

    let r_mod = Module::new(&engine, &r_wasm).unwrap();

    #[derive(Default)]
    struct State {
        limits: StoreLimits,
        reenter_fn: Option<wtime_aot_host::TypedFunc<i32, (Rooted<AnyRef>, i32)>>,
        churn_fn: Option<wtime_aot_host::TypedFunc<i32, i32>>,
    }

    let mut store = Store::new(
        &engine,
        State {
            limits: StoreLimitsBuilder::new().memory_size(64 * 1024).build(),
            ..Default::default()
        },
    );
    store.limiter(|s| &mut s.limits);

    let mut linker = Linker::new(&engine);
    linker
        .func_wrap(
            "host",
            "reenter_and_gc",
            |mut caller: Caller<'_, State>, depth: i32| -> wtime_aot_host::Result<i32> {
                // Force a GC while outer Wasm frames are live on the stack chain!
                caller.gc(None)?;
                let churn = caller.data().churn_fn.unwrap();
                // Churn 6,000 cycles (> 192 KiB > 3x the 64 KiB heap cap!) inside nested Wasm!
                churn.call(&mut caller, 6_000)?;
                if depth > 0 {
                    let f = caller.data().reenter_fn.unwrap();
                    let (node, sub_sum) = f.call(&mut caller, depth - 1)?;
                    caller.gc(None)?;
                    let st = node.as_struct(&caller)?.expect("struct");
                    let v = st.field(&mut caller, 1)?.unwrap_i32();
                    assert_eq!(v, 222);
                    Ok(sub_sum + 1)
                } else {
                    Ok(10)
                }
            },
        )
        .unwrap();

    let inst = linker.instantiate(&mut store, &r_mod).unwrap();
    let f = inst
        .get_typed_func::<i32, (Rooted<AnyRef>, i32)>(&mut store, "alloc_chain_and_reenter")
        .unwrap();
    let churn = inst
        .get_typed_func::<i32, i32>(&mut store, "churn")
        .unwrap();
    store.data_mut().reenter_fn = Some(f);
    store.data_mut().churn_fn = Some(churn);

    let (top_node, total) = f.call(&mut store, 3).unwrap();
    // Depth 0: 10 + 333 = 343; Depth 1: 344 + 333 = 677; Depth 2: 678 + 333 = 1011; Depth 3: 1012 + 333 = 1345
    assert_eq!(total, 1345);
    assert!(store.gc_heap_capacity() <= 64 * 1024);
    let top_st = top_node.as_struct(&store).unwrap().unwrap();
    assert_eq!(top_st.field(&mut store, 1).unwrap().unwrap_i32(), 222);
    let inner = top_st
        .field(&mut store, 0)
        .unwrap()
        .unwrap_anyref()
        .unwrap()
        .as_struct(&store)
        .unwrap()
        .unwrap();
    assert_eq!(inner.field(&mut store, 1).unwrap().unwrap_i32(), 111);
}

#[test]
fn weak_anyref_ephemeron_chains_weakmap_table_and_externref_finalization() {
    let engine = default_engine();
    let struct_ty = StructType::new(
        &engine,
        [
            FieldType::new(Mutability::Var, StorageType::ValType(ValType::EQREF)),
            FieldType::new(Mutability::Const, StorageType::ValType(ValType::I32)),
        ],
    )
    .unwrap();
    let arr_ty = ArrayType::new(
        &engine,
        FieldType::new(Mutability::Var, StorageType::ValType(ValType::EQREF)),
    );

    let mut store = Store::new(&engine, ());
    let pre = StructRefPre::new(&store, struct_ty);
    let arr_pre = ArrayRefPre::new(&store, arr_ty);

    let make_obj = |s: &mut Store<()>, m: i32| -> Rooted<AnyRef> {
        StructRef::new(s, &pre, &[Val::AnyRef(None), Val::I32(m)])
            .unwrap()
            .to_anyref()
    };

    // 1. WeakAnyRef: dead target cleared; surviving target moves across 6 GCs; stale slot rejected.
    let owner = make_obj(&mut store, 1);
    let dead_weak = {
        let mut scope = RootScope::new(&mut store);
        let t = StructRef::new(&mut scope, &pre, &[Val::AnyRef(None), Val::I32(42)])
            .unwrap()
            .to_anyref();
        WeakAnyRef::new(&mut scope, &owner, &t).unwrap()
    };
    store.gc(None).unwrap();
    assert!(dead_weak.upgrade(&mut store).unwrap().is_none());

    let (live_weak, owned_target) = {
        let mut scope = RootScope::new(&mut store);
        let t = StructRef::new(&mut scope, &pre, &[Val::AnyRef(None), Val::I32(99)])
            .unwrap()
            .to_anyref();
        let w = WeakAnyRef::new(&mut scope, &owner, &t).unwrap();
        (w, t.to_owned_rooted(&mut scope).unwrap())
    };
    for _ in 0..6 {
        store.gc(None).unwrap();
        let mut scope = RootScope::new(&mut store);
        let up = live_weak.upgrade(&mut scope).unwrap().unwrap();
        assert!(Rooted::ref_eq(&scope, &owned_target, &up).unwrap());
        assert_eq!(
            up.as_struct(&scope)
                .unwrap()
                .unwrap()
                .field(&mut scope, 1)
                .unwrap()
                .unwrap_i32(),
            99
        );
    }
    drop(owned_target);
    store.gc(None).unwrap();
    assert!(live_weak.upgrade(&mut store).unwrap().is_none());

    // 2. Reverse-order GcEphemeron chain A -> B -> C + value->key cycle reclamation.
    let (a_owned, e1, e2, weak_c, cycle_e, cycle_wk) = {
        let mut scope = RootScope::new(&mut store);
        let a = StructRef::new(&mut scope, &pre, &[Val::AnyRef(None), Val::I32(10)])
            .unwrap()
            .to_anyref();
        let b = StructRef::new(&mut scope, &pre, &[Val::AnyRef(None), Val::I32(20)])
            .unwrap()
            .to_anyref();
        let c = StructRef::new(&mut scope, &pre, &[Val::AnyRef(None), Val::I32(30)])
            .unwrap()
            .to_anyref();
        // Register in reverse order so fixed-point iteration is required!
        let e2 = GcEphemeron::new(&mut scope, &owner, &b, Some(&c)).unwrap();
        let e1 = GcEphemeron::new(&mut scope, &owner, &a, Some(&b)).unwrap();
        let weak_c = WeakAnyRef::new(&mut scope, &owner, &c).unwrap();

        // Value -> key cycle without external root for key MUST be collected!
        let ck = StructRef::new(&mut scope, &pre, &[Val::AnyRef(None), Val::I32(55)])
            .unwrap()
            .to_anyref();
        let cv = StructRef::new(&mut scope, &pre, &[Val::AnyRef(Some(ck)), Val::I32(66)])
            .unwrap()
            .to_anyref();
        let cycle_e = GcEphemeron::new(&mut scope, &owner, &ck, Some(&cv)).unwrap();
        let cycle_wk = WeakAnyRef::new(&mut scope, &owner, &ck).unwrap();

        (
            a.to_owned_rooted(&mut scope).unwrap(),
            e1,
            e2,
            weak_c,
            cycle_e,
            cycle_wk,
        )
    };
    store.gc(None).unwrap();
    assert!(!cycle_e.is_live(&store).unwrap());
    assert!(cycle_wk.upgrade(&mut store).unwrap().is_none());
    assert!(e1.is_live(&store).unwrap());
    assert!(e2.is_live(&store).unwrap());
    {
        let mut scope = RootScope::new(&mut store);
        let c_up = weak_c.upgrade(&mut scope).unwrap().unwrap();
        assert_eq!(
            c_up.as_struct(&scope)
                .unwrap()
                .unwrap()
                .field(&mut scope, 1)
                .unwrap()
                .unwrap_i32(),
            30
        );
    }
    drop(a_owned);
    store.gc(None).unwrap();
    assert!(!e1.is_live(&store).unwrap());
    assert!(!e2.is_live(&store).unwrap());
    assert!(weak_c.upgrade(&mut store).unwrap().is_none());

    // 3. Ephemeron marking precedes ExternRef host-data sweeping!
    struct DropCounter(Arc<AtomicUsize>);
    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let drops = Arc::new(AtomicUsize::new(0));
    let (ext_key, ext_eph) = {
        let mut scope = RootScope::new(&mut store);
        let k = StructRef::new(&mut scope, &pre, &[Val::AnyRef(None), Val::I32(7)])
            .unwrap()
            .to_anyref();
        let native = ExternRef::new(&mut scope, DropCounter(Arc::clone(&drops))).unwrap();
        let boxed = AnyRef::convert_extern(&mut scope, native).unwrap();
        let e = GcEphemeron::new(&mut scope, &owner, &k, Some(&boxed)).unwrap();
        (k.to_owned_rooted(&mut scope).unwrap(), e)
    };
    store.gc(None).unwrap();
    assert!(ext_eph.is_live(&store).unwrap());
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop(ext_key);
    store.gc(None).unwrap();
    assert!(!ext_eph.is_live(&store).unwrap());
    assert_eq!(drops.load(Ordering::SeqCst), 1);

    // 4. Registered WeakMap table array (`register_table_raw`): live pair moves; dead pair clears!
    let table = ArrayRef::new(&mut store, &arr_pre, &Val::AnyRef(None), 4).unwrap();
    let table_raw = table.to_raw(&mut store).unwrap();
    unsafe { GcEphemeron::register_table_raw(&mut store, table_raw) }.unwrap();
    let (live_key, weak_live_val, weak_dead_val) = {
        let mut scope = RootScope::new(&mut store);
        let k1 = StructRef::new(&mut scope, &pre, &[Val::AnyRef(None), Val::I32(101)])
            .unwrap()
            .to_anyref();
        let v1 = StructRef::new(&mut scope, &pre, &[Val::AnyRef(None), Val::I32(202)])
            .unwrap()
            .to_anyref();
        let k2 = StructRef::new(&mut scope, &pre, &[Val::AnyRef(None), Val::I32(303)])
            .unwrap()
            .to_anyref();
        let v2 = StructRef::new(&mut scope, &pre, &[Val::AnyRef(None), Val::I32(404)])
            .unwrap()
            .to_anyref();
        table.set(&mut scope, 0, Val::AnyRef(Some(k1))).unwrap();
        table.set(&mut scope, 1, Val::AnyRef(Some(v1))).unwrap();
        table.set(&mut scope, 2, Val::AnyRef(Some(k2))).unwrap();
        table.set(&mut scope, 3, Val::AnyRef(Some(v2))).unwrap();
        (
            k1.to_owned_rooted(&mut scope).unwrap(),
            WeakAnyRef::new(&mut scope, &owner, &v1).unwrap(),
            WeakAnyRef::new(&mut scope, &owner, &v2).unwrap(),
        )
    };
    for _ in 0..3 {
        store.gc(None).unwrap();
        let mut scope = RootScope::new(&mut store);
        // Pair 0 (live_key) survived and moved; pair 1 (k2) died and was cleared to null!
        assert!(table.get(&mut scope, 0).unwrap().unwrap_anyref().is_some());
        let v1_got = table.get(&mut scope, 1).unwrap().unwrap_anyref().unwrap();
        assert_eq!(
            v1_got
                .as_struct(&scope)
                .unwrap()
                .unwrap()
                .field(&mut scope, 1)
                .unwrap()
                .unwrap_i32(),
            202
        );
        assert!(weak_live_val.upgrade(&mut scope).unwrap().is_some());
        assert!(table.get(&mut scope, 2).unwrap().unwrap_anyref().is_none());
        assert!(table.get(&mut scope, 3).unwrap().unwrap_anyref().is_none());
        assert!(weak_dead_val.upgrade(&mut scope).unwrap().is_none());
    }
    drop(live_key);
    store.gc(None).unwrap();
    assert!(table.get(&mut store, 0).unwrap().unwrap_anyref().is_none());
    assert!(table.get(&mut store, 1).unwrap().unwrap_anyref().is_none());
    assert!(weak_live_val.upgrade(&mut store).unwrap().is_none());

    // 5. Raw access API + i31/null validation + foreign store rejection.
    let i31 = AnyRef::from_i31(&mut store, I31::wrapping_i32(7));
    assert!(WeakAnyRef::new(&mut store, &owner, &i31).is_err());
    let k = make_obj(&mut store, 88);
    let (o_raw, k_raw) = (
        owner.to_raw(&mut store).unwrap(),
        k.to_raw(&mut store).unwrap(),
    );
    let e_raw = GcEphemeron::with_raw_access(&mut store, |_, raw| {
        let e = unsafe { raw.insert_raw(o_raw, k_raw, 0)? };
        assert!(raw.is_live(e)?);
        let upd = unsafe {
            raw.set_value_if_matches_raw(
                e,
                o_raw,
                k_raw,
                i31.to_raw(&mut Store::new(&engine, ())).unwrap_or(15),
            )?
        };
        assert_eq!(upd, GcEphemeronUpdate::Updated);
        Ok(e)
    })
    .unwrap();
    let foreign = Store::new(&engine, ());
    assert!(e_raw.matches_raw(&foreign, o_raw, k_raw).is_err());
}

#[test]
fn recoverable_gc_oom_and_bulk_i8_array_copy() {
    let mut cfg = Config::new();
    cfg.collector(Collector::Copying)
        .gc_heap_initial_size(64 * 1024);
    let engine = Engine::new(&cfg).unwrap();

    let wasm = parse_wat(
        r#"
        (module
          (type $bytes (array (mut i8)))
          (func (export "xor_bytes") (param $arr (ref $bytes)) (param $mask i32)
            (local $i i32)
            (local $n i32)
            (local.set $n (array.len (local.get $arr)))
            (loop $l
              (if (i32.lt_u (local.get $i) (local.get $n))
                (then
                  (array.set $bytes (local.get $arr) (local.get $i)
                    (i32.xor (array.get_u $bytes (local.get $arr) (local.get $i)) (local.get $mask)))
                  (local.set $i (i32.add (local.get $i) (i32.const 1)))
                  (br $l)))))
        )
        "#,
    );
    let module = Module::new(&engine, &wasm).unwrap();
    let mut store = Store::new(
        &engine,
        StoreLimitsBuilder::new().memory_size(64 * 1024).build(),
    );
    store.limiter(|l| l);
    let inst = Instance::new(&mut store, &module, &[]).unwrap();

    let byte_arr_ty = ArrayType::new(&engine, FieldType::new(Mutability::Var, StorageType::I8));
    let pre = ArrayRefPre::new(&store, byte_arr_ty);

    // Requesting 1 MiB under a 64 KiB cap MUST return recoverable `GcHeapOutOfMemory<()>`!
    let err = ArrayRef::new(&mut store, &pre, &Val::I32(0), 1 << 20).unwrap_err();
    assert!(
        err.is::<GcHeapOutOfMemory<()>>(),
        "expected GcHeapOutOfMemory, got {err}"
    );

    // Store stays completely usable after recoverable OOM!
    let input = b"hello wtime-aot-host bulk i8 array copy!";
    let arr = ArrayRef::new_from_i8_slice(&mut store, &pre, input).unwrap();
    let xor_fn = inst.get_func(&mut store, "xor_bytes").unwrap();
    xor_fn
        .call(
            &mut store,
            &[Val::AnyRef(Some(arr.to_anyref())), Val::I32(0x5a)],
            &mut [],
        )
        .unwrap();
    let mut out = vec![0u8; input.len()];
    arr.copy_to_i8_slice(&mut store, &mut out).unwrap();
    let expected: Vec<u8> = input.iter().map(|&b| b ^ 0x5a).collect();
    assert_eq!(out, expected);
}

#[test]
fn cross_module_exceptions_wasm_and_host_thrown() {
    let engine = default_engine();
    let wasm = parse_wat(
        r#"
        (module
          (type $box (struct (field i32)))
          (import "host" "err_tag" (tag $err (param i32 (ref null $box))))
          (import "host" "throw_from_host" (func $host_throw (param i32)))

          (func (export "catch_host_throw") (param i32) (result i32)
            (block $caught (result i32 (ref null $box))
              (try_table (catch $err $caught)
                (call $host_throw (local.get 0)))
              (return (i32.const -1)))
            ;; Stack now has: [i32 code, (ref null $box)]
            (i32.add (struct.get $box 0) ))

          (func (export "throw_to_host") (param i32)
            (throw $err (local.get 0) (struct.new $box (i32.mul (local.get 0) (i32.const 10)))))
        )
        "#,
    );
    let module = Module::new(&engine, &wasm).unwrap();
    let err_tag_ty = match module
        .imports()
        .find(|i| i.name() == "err_tag")
        .unwrap()
        .ty()
    {
        wtime_aot_host::ExternType::Tag(t) => t,
        _ => panic!("expected tag import"),
    };
    let box_struct_ty = err_tag_ty
        .params()
        .nth(1)
        .unwrap()
        .as_ref()
        .unwrap()
        .heap_type()
        .as_concrete_struct()
        .unwrap()
        .clone();

    let mut store = Store::new(&engine, ());
    let tag = Tag::new(&mut store, &err_tag_ty).unwrap();
    let box_pre = StructRefPre::new(&store, box_struct_ty);
    let exn_pre = ExnRefPre::new(&store, ExnType::from_tag_type(&err_tag_ty).unwrap());

    let mut linker = Linker::new(&engine);
    linker.define(&store, "host", "err_tag", tag).unwrap();
    linker
        .func_wrap(
            "host",
            "throw_from_host",
            move |mut caller: Caller<'_, ()>, code: i32| -> wtime_aot_host::Result<()> {
                let s = StructRef::new(&mut caller, &box_pre, &[Val::I32(code * 100)])?;
                // Trigger GC before throwing to ensure `s` survives!
                caller.gc(None)?;
                let exn = ExnRef::new(
                    &mut caller,
                    &exn_pre,
                    &tag,
                    &[Val::I32(code), Val::AnyRef(Some(s.to_anyref()))],
                )?;
                caller.throw(exn)
            },
        )
        .unwrap();

    let inst = linker.instantiate(&mut store, &module).unwrap();
    let catch_fn = inst
        .get_typed_func::<i32, i32>(&mut store, "catch_host_throw")
        .unwrap();
    assert_eq!(catch_fn.call(&mut store, 7).unwrap(), 7 + 700);

    // Wasm throws uncaught exception -> host inspects payload!
    let throw_fn = inst
        .get_typed_func::<i32, ()>(&mut store, "throw_to_host")
        .unwrap();
    assert!(throw_fn.call(&mut store, 9).is_err());
    let exn = store
        .take_pending_exception()
        .expect("pending uncaught Wasm exception");
    assert!(Tag::eq(&exn.tag(&store).unwrap(), &tag, &store));
    assert_eq!(exn.field(&mut store, 0).unwrap().unwrap_i32(), 9);
    let payload_box = exn
        .field(&mut store, 1)
        .unwrap()
        .unwrap_anyref()
        .unwrap()
        .as_struct(&store)
        .unwrap()
        .unwrap();
    assert_eq!(payload_box.field(&mut store, 0).unwrap().unwrap_i32(), 90);
}

#[test]
fn shared_memory_multithreaded_atomics_wait_notify_and_grow() {
    let engine = default_engine();
    let wasm = parse_wat(
        r#"
        (module
          (import "env" "mem" (memory 1 4 shared))
          (func (export "wait_until_changed") (param i32) (result i32)
            (memory.atomic.wait32 (i32.const 0) (local.get 0) (i64.const -1)))
          (func (export "store_and_wake") (param i32) (result i32)
            (i32.atomic.store (i32.const 0) (local.get 0))
            (memory.atomic.notify (i32.const 0) (i32.const 1)))
          (func (export "grow_mem") (param i32) (result i32)
            (memory.grow (local.get 0)))
          (func (export "mem_pages") (result i32)
            (memory.size))
          (func (export "load_val") (result i32)
            (i32.atomic.load (i32.const 0)))
        )
        "#,
    );
    let module = Module::new(&engine, &wasm).unwrap();
    let shared = SharedMemory::new(&engine, MemoryType::shared(1, 4)).unwrap();

    let ready = Arc::new(AtomicBool::new(false));
    let ready2 = Arc::clone(&ready);
    let eng2 = engine.clone();
    let mod2 = module.clone();
    let sh2 = shared.clone();

    let waiter = std::thread::spawn(move || {
        let mut store = Store::new(&eng2, ());
        let inst = Instance::new(&mut store, &mod2, &[Extern::SharedMemory(sh2)]).unwrap();
        let wait_fn = inst
            .get_typed_func::<i32, i32>(&mut store, "wait_until_changed")
            .unwrap();
        let pages_fn = inst
            .get_typed_func::<(), i32>(&mut store, "mem_pages")
            .unwrap();
        let load_fn = inst
            .get_typed_func::<(), i32>(&mut store, "load_val")
            .unwrap();
        ready2.store(true, Ordering::Release);
        let rc = wait_fn.call(&mut store, 0).unwrap();
        (
            rc,
            load_fn.call(&mut store, ()).unwrap(),
            pages_fn.call(&mut store, ()).unwrap(),
        )
    });

    while !ready.load(Ordering::Acquire) {
        std::thread::yield_now();
    }
    std::thread::sleep(Duration::from_millis(15));

    let mut store1 = Store::new(&engine, ());
    let inst1 = Instance::new(&mut store1, &module, &[Extern::SharedMemory(shared)]).unwrap();
    let grow_fn = inst1
        .get_typed_func::<i32, i32>(&mut store1, "grow_mem")
        .unwrap();
    let wake_fn = inst1
        .get_typed_func::<i32, i32>(&mut store1, "store_and_wake")
        .unwrap();
    assert_eq!(grow_fn.call(&mut store1, 1).unwrap(), 1);
    let woken = wake_fn.call(&mut store1, 12345).unwrap();
    assert!(woken <= 1);

    let (wait_rc, loaded, pages) = waiter.join().unwrap();
    // Wait returned either 0 (notified) or 1 (not-equal if store happened before wait entered).
    assert!(wait_rc == 0 || wait_rc == 1, "unexpected wait rc {wait_rc}");
    assert_eq!(loaded, 12345);
    assert_eq!(pages, 2);
}

#[test]
fn interrupt_handle_preempts_infinite_wasm_loop_and_recovers() {
    let engine = default_engine();
    let wasm = parse_wat(
        r#"
        (module
          (import "env" "mem" (memory 1 2))
          (func (export "spin") (result i32)
            (local $i i32)
            (loop $l
              (i32.store (i32.const 0) (local.get $i))
              (local.set $i (i32.add (local.get $i) (i32.const 1)))
              (br $l))
            (local.get $i))
          (func (export "ping") (result i32)
            (i32.store (i32.const 4) (i32.const 42))
            (i32.load (i32.const 4)))
        )
        "#,
    );
    let module = Module::new(&engine, &wasm).unwrap();
    let mut store = Store::new(&engine, ());
    let mem = Memory::new(&mut store, MemoryType::new(1, Some(2))).unwrap();
    let inst = Instance::new(&mut store, &module, &[Extern::Memory(mem)]).unwrap();

    let handle = store.interrupt_handle();
    let handle2 = handle.clone();
    let done = Arc::new(AtomicBool::new(false));
    let done2 = Arc::clone(&done);
    let killer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(15));
        while !done2.load(Ordering::Acquire) {
            handle2.interrupt();
            std::thread::sleep(Duration::from_millis(1));
        }
    });

    let spin = inst.get_typed_func::<(), i32>(&mut store, "spin").unwrap();
    let t0 = Instant::now();
    let err = spin.call(&mut store, ()).unwrap_err();
    done.store(true, Ordering::Release);
    killer.join().unwrap();
    handle.cancel();

    assert_eq!(
        err.downcast_ref::<Trap>(),
        Some(&Trap::Interrupt),
        "expected Trap::Interrupt, got {err}"
    );
    assert!(
        t0.elapsed() < Duration::from_secs(2),
        "interrupt took too long: {:?}",
        t0.elapsed()
    );

    // Store is cleanly recoverable after `cancel()`!
    let ping = inst.get_typed_func::<(), i32>(&mut store, "ping").unwrap();
    assert_eq!(ping.call(&mut store, ()).unwrap(), 42);
}

#[test]
fn real_scriptw_r_and_p_load_into_shared_engine() {
    let dir = std::path::Path::new(
        "/home/mewhhaha/.gemini/antigravity-cli/brain/9263a17a-e037-4d2f-8e79-4415cd4c465c/scratch/scriptw_smoke",
    );
    let p_path = dir.join("smoke.wasm");
    let r_path = dir.join("smoke.wasm.runtime.wasm");
    if !p_path.exists() || !r_path.exists() {
        return;
    }
    let engine = default_engine();
    let r_mod = Module::new(&engine, std::fs::read(&r_path).unwrap()).expect("load Scriptw R");
    let p_mod = Module::new(&engine, std::fs::read(&p_path).unwrap()).expect("load Scriptw P");

    assert!(r_mod.exports().len() > 1000);
    assert!(p_mod.get_export("main").is_some());
    assert!(p_mod.get_export_index("main").is_some());
    assert!(r_mod.get_export("memory").is_some());
}

#[test]
fn host_import_preserves_callee_saved_xmm8_13_across_c_abi() {
    let engine = default_engine();
    let wasm = parse_wat(
        r#"
        (module
          (import "env" "clobber" (func $clobber (param f64) (result f64)))
          (func (export "loop_and_sum") (param $a f64) (param $b f64) (param $c f64)
                                         (param $d f64) (param $e f64) (param $f f64)
                                         (result f64)
            (local $i f64)
            (local $acc f64)
            (local.set $i (f64.const 0.0))
            (local.set $acc (f64.const 0.0))
            (block $done
              (loop $lp
                (br_if $done (f64.ge (local.get $i) (f64.const 10.0)))
                (local.set $acc (f64.add (local.get $acc) (call $clobber (local.get $i))))
                (local.set $i (f64.add (local.get $i) (f64.const 1.0)))
                (br $lp)
              )
            )
            (f64.add
              (local.get $acc)
              (f64.add (local.get $a)
                (f64.add (local.get $b)
                  (f64.add (local.get $c)
                    (f64.add (local.get $d)
                      (f64.add (local.get $e) (local.get $f)))))))
          )
        )
        "#,
    );
    let module = Module::new(&engine, &wasm).unwrap();
    let mut store = Store::new(&engine, 0usize);
    let mut linker = Linker::new(&engine);
    linker
        .func_wrap(
            "env",
            "clobber",
            |mut caller: Caller<'_, usize>, x: f64| -> f64 {
                *caller.data_mut() += 1;
                #[cfg(target_arch = "x86_64")]
                unsafe {
                    // Explicitly zero Xmm8..Xmm13 (valid in SysV C ABI, callee-saved in wtime Wasm ABI).
                    core::arch::asm!(
                        "pxor xmm8, xmm8",
                        "pxor xmm9, xmm9",
                        "pxor xmm10, xmm10",
                        "pxor xmm11, xmm11",
                        "pxor xmm12, xmm12",
                        "pxor xmm13, xmm13",
                        out("xmm8") _,
                        out("xmm9") _,
                        out("xmm10") _,
                        out("xmm11") _,
                        out("xmm12") _,
                        out("xmm13") _,
                        options(nomem, nostack, preserves_flags),
                    );
                }
                x * 2.0
            },
        )
        .unwrap();
    let inst = linker.instantiate(&mut store, &module).unwrap();
    let f = inst
        .get_typed_func::<(f64, f64, f64, f64, f64, f64), f64>(&mut store, "loop_and_sum")
        .unwrap();
    let res = f.call(&mut store, (1.5, 2.5, 3.5, 4.5, 5.5, 6.5)).unwrap();
    assert_eq!(*store.data(), 10, "loop must run exactly 10 times");
    // sum_{i=0..9} (2*i) = 90.0; 1.5+2.5+3.5+4.5+5.5+6.5 = 24.0; total = 114.0
    assert_eq!(res, 114.0);
}
