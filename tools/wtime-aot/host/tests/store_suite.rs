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

#[test]
fn large_byte_array_2gib_plus_16_survives_gc() {
    use wtime_aot_host::{ArrayRef, ArrayRefPre, ArrayType, FieldType, Mutability, StorageType};

    let engine = Engine::default();
    let mut store = Store::new(&engine, ());
    let arr_ty = ArrayType::new(&engine, FieldType::new(Mutability::Var, StorageType::I8));
    let pre = ArrayRefPre::new(&mut store, arr_ty);

    let len: u32 = (1u32 << 31) + 16; // 2_147_483_664 bytes (> 2 GiB)
    assert_eq!(len, 2_147_483_664);

    let arr = ArrayRef::new(&mut store, &pre, &Val::I32(0), len)
        .expect("2 GiB + 16 byte ArrayRef allocation must succeed in 8 GiB semi-space");
    assert_eq!(arr.len(&store).unwrap(), len);

    let raw_before = arr.to_anyref().to_raw(&mut store).unwrap();
    assert_ne!(raw_before, 0, "live GC object raw handle must be nonzero");
    assert_eq!(
        raw_before & 1,
        0,
        "live GC object raw handle must have bit 0 == 0"
    );

    arr.set(&mut store, 16, Val::I32(0x5A)).unwrap();
    arr.set(&mut store, len - 1, Val::I32(0xA5)).unwrap();

    assert_eq!(arr.get(&mut store, 16).unwrap().i32(), Some(0x5A));
    assert_eq!(arr.get(&mut store, len - 1).unwrap().i32(), Some(0xA5));
    assert_eq!(arr.get(&mut store, 0).unwrap().i32(), Some(0));
    assert_eq!(arr.get(&mut store, 15).unwrap().i32(), Some(0));
    assert_eq!(arr.get(&mut store, 17).unwrap().i32(), Some(0));
    assert_eq!(arr.get(&mut store, 1u32 << 30).unwrap().i32(), Some(0));
    assert_eq!(arr.get(&mut store, len - 2).unwrap().i32(), Some(0));

    // Trigger copying GC across the 8 GiB Space A <-> Space B boundary.
    store.gc(None).unwrap();

    let raw_after = arr.to_anyref().to_raw(&mut store).unwrap();
    assert_ne!(raw_after, 0);
    assert_eq!(raw_after & 1, 0);
    assert_ne!(
        raw_before & (1u32 << 31),
        raw_after & (1u32 << 31),
        "evacuation between Space A (< 8 GiB) and Space B (>= 8 GiB) must flip bit 31"
    );

    assert_eq!(arr.len(&store).unwrap(), len);
    assert_eq!(arr.get(&mut store, 16).unwrap().i32(), Some(0x5A));
    assert_eq!(arr.get(&mut store, len - 1).unwrap().i32(), Some(0xA5));
    assert_eq!(arr.get(&mut store, 0).unwrap().i32(), Some(0));
    assert_eq!(arr.get(&mut store, 15).unwrap().i32(), Some(0));
    assert_eq!(arr.get(&mut store, 17).unwrap().i32(), Some(0));
    assert_eq!(arr.get(&mut store, 1u32 << 30).unwrap().i32(), Some(0));
    assert_eq!(arr.get(&mut store, len - 2).unwrap().i32(), Some(0));
}

#[test]
fn serialized_wtms0002_starts_with_elf_and_zero_copy_file_range_roundtrips() {
    use std::io::Write;

    let engine = default_engine();
    let wasm = parse_wat(
        r#"
        (module
          (func (export "add") (param i32 i32) (result i32)
            (i32.add (local.get 0) (local.get 1)))
        )
        "#,
    );
    let orig = Module::new(&engine, &wasm).expect("compile");
    let serialized = orig.serialize().expect("serialize");
    assert_eq!(
        &serialized[..4],
        b"\x7fELF",
        "WTMS0002 must place raw ELF64 .so at offset 0 for zero-copy page-aligned mmap"
    );
    assert_eq!(&serialized[serialized.len() - 8..], b"WTMS0002");

    // Write serialized module at a 64 KiB-aligned file offset with non-zero prefix/suffix.
    let dir = std::env::temp_dir().join(format!(
        "wtime-wtms0002-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("bundle.bin");
    let offset = 64 * 1024u64;
    {
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(&vec![0xAAu8; offset as usize]).unwrap();
        f.write_all(&serialized).unwrap();
        f.write_all(&[0x55u8; 4099]).unwrap();
    }

    let f = std::fs::File::open(&path).unwrap();
    let loaded =
        unsafe { Module::deserialize_open_file_range(&engine, f, offset, serialized.len()) }
            .expect("deserialize_open_file_range");
    // Unlink file immediately: kernel keeps the open backing fd valid for both mapping & re-serialize.
    let _ = std::fs::remove_dir_all(&dir);

    let mut store = Store::new(&engine, ());
    let inst = Instance::new(&mut store, &loaded, &[]).expect("instantiate");
    let add = inst
        .get_typed_func::<(i32, i32), i32>(&mut store, "add")
        .expect("add export");
    assert_eq!(add.call(&mut store, (19, 23)).unwrap(), 42);

    // Re-serializing a zero-copy file-backed Module must produce byte-identical output.
    let reserialized = loaded.serialize().expect("re-serialize file-backed module");
    assert_eq!(reserialized, serialized);

    // Verify backwards compatibility with legacy WTMS0001 leading-header format.
    let tail = &serialized[serialized.len() - 20..];
    let ver_len = u32::from_le_bytes(tail[0..4].try_into().unwrap()) as usize;
    let so_len = u64::from_le_bytes(tail[4..12].try_into().unwrap()) as usize;
    let ver = &serialized[so_len..so_len + ver_len];
    let so = &serialized[..so_len];
    let mut v1 = Vec::with_capacity(serialized.len());
    v1.extend_from_slice(b"WTMS0001");
    v1.extend_from_slice(&(ver_len as u32).to_le_bytes());
    v1.extend_from_slice(ver);
    v1.extend_from_slice(&(so_len as u64).to_le_bytes());
    v1.extend_from_slice(so);
    let legacy = unsafe { Module::deserialize(&engine, &v1) }.expect("deserialize legacy WTMS0001");
    let legacy_inst = Instance::new(&mut store, &legacy, &[]).expect("instantiate legacy");
    let legacy_add = legacy_inst
        .get_typed_func::<(i32, i32), i32>(&mut store, "add")
        .expect("legacy add");
    assert_eq!(legacy_add.call(&mut store, (100, 23)).unwrap(), 123);
}

#[test]
fn gc_semispace_stays_bounded_under_garbage_and_shrinks_after_burst() {
    let mut cfg = Config::new();
    cfg.collector(Collector::Copying)
        .wasm_gc(true)
        .gc_heap_initial_size(256 * 1024);
    let engine = Engine::new(&cfg).expect("engine");
    let mut store = Store::new(&engine, ());
    store.limiter(|_| {
        Box::leak(Box::new(
            StoreLimitsBuilder::new()
                .memory_size(16 * 1024 * 1024)
                .build(),
        ))
    });
    let arr_ty = ArrayType::new(&engine, FieldType::new(Mutability::Var, StorageType::I8));
    let pre = ArrayRefPre::new(&mut store, arr_ty);

    // Steady-state garbage allocation: 200 * 32 KiB = 6.4 MiB with 0 live objects.
    // Because the semi-space must not double on pre-GC garbage, GC triggers regularly and
    // stays near `init_cap` (256 KiB) instead of growing to 16 MiB.
    for _ in 0..200 {
        let mut scope = RootScope::new(&mut store);
        let arr = ArrayRef::new(&mut scope, &pre, &Val::I32(7), 32 * 1024).expect("alloc 32KiB");
        assert_eq!(arr.len(&scope).unwrap(), 32 * 1024);
    }
    assert!(
        store.gc_count() >= 10,
        "expected >= 10 collections at bounded 256 KiB semi-space, got {}",
        store.gc_count()
    );

    // Transient 4 MiB burst grows the semi-space, then drops and shrinks cleanly on subsequent GCs.
    {
        let mut scope = RootScope::new(&mut store);
        let big = ArrayRef::new(&mut scope, &pre, &Val::I32(9), 4 * 1024 * 1024).expect("burst");
        assert_eq!(
            big.get(&mut scope, 4 * 1024 * 1024 - 1).unwrap().i32(),
            Some(9)
        );
    }
    store.gc(None).unwrap();
    store.gc(None).unwrap();
    let gc_before = store.gc_count();
    for _ in 0..40 {
        let mut scope = RootScope::new(&mut store);
        let _ = ArrayRef::new(&mut scope, &pre, &Val::I32(1), 32 * 1024).expect("post-burst alloc");
    }
    assert!(
        store.gc_count() > gc_before,
        "semi-space must shrink after burst so 40*32KiB triggers GC again (before={gc_before}, after={})",
        store.gc_count()
    );
}

#[test]
fn warm_semispace_reuse_zero_initializes_large_arrays() {
    let mut cfg = Config::new();
    cfg.collector(Collector::Copying)
        .wasm_gc(true)
        .gc_heap_initial_size(2 * 1024 * 1024);
    let engine = Engine::new(&cfg).expect("engine");
    let mut store = Store::new(&engine, ());
    let arr_ty = ArrayType::new(&engine, FieldType::new(Mutability::Var, StorageType::I8));
    let pre = ArrayRefPre::new(&mut store, arr_ty);

    // Dirty both warm semi-spaces with non-zero bytes across several automatic bump-alloc GCs.
    for _ in 0..40 {
        let mut scope = RootScope::new(&mut store);
        let arr = ArrayRef::new(&mut scope, &pre, &Val::I32(0x5A), 256 * 1024).expect("dirty");
        assert_eq!(arr.get(&mut scope, 123_456).unwrap().i32(), Some(0x5A));
    }
    assert!(store.gc_count() >= 3);

    // Now allocate > 1 MiB zero-initialized arrays inside the reused warm semi-space AND
    // spanning past the warm watermark; every byte must read back as 0.
    for &len in &[1_500_000u32, 3_000_000u32] {
        let mut scope = RootScope::new(&mut store);
        let z = ArrayRef::new(&mut scope, &pre, &Val::I32(0), len).expect("zero alloc");
        let mut buf = vec![0xFFu8; len as usize];
        z.copy_to_i8_slice(&mut scope, &mut buf).unwrap();
        assert!(
            buf.iter().all(|&b| b == 0),
            "warm semi-space reuse leaked non-zero bytes into {len}-byte zero array"
        );
    }
}

#[test]
fn precompiled_module_deserializes_across_opt_levels_and_reports_image_range() {
    let wasm = wat::parse_str(
        r#"(module
            (func (export "mul") (param i32 i32) (result i32)
                local.get 0
                local.get 1
                i32.mul)
        )"#,
    )
    .expect("wat");

    let mut compile_cfg = Config::new();
    compile_cfg.cranelift_opt_level(wtime_aot_host::OptLevel::SpeedAndSize);
    let compile_engine = Engine::new(&compile_cfg).expect("compile engine");
    let bytes = compile_engine.precompile_module(&wasm).expect("precompile");

    // Deserializing with a different OptLevel and compiler disabled must succeed.
    let mut load_cfg = Config::new();
    load_cfg
        .cranelift_opt_level(wtime_aot_host::OptLevel::Speed)
        .enable_compiler(false);
    let load_engine = Engine::new(&load_cfg).expect("load engine");
    let mem_mod = unsafe { Module::deserialize(&load_engine, &bytes) }.expect("deserialize mem");
    let mem_range = mem_mod.image_range();
    assert_eq!(
        mem_range.end as usize - mem_range.start as usize,
        bytes.len()
    );

    let tmp_path = std::env::temp_dir().join(format!(
        "wtime-aot-host-image-range-{}.wtmn",
        std::process::id()
    ));
    std::fs::write(&tmp_path, &bytes).expect("write wtmn");
    let file = std::fs::File::open(&tmp_path).expect("open wtmn");
    let file_mod =
        unsafe { Module::deserialize_open_file(&load_engine, file) }.expect("deserialize file");
    let _ = std::fs::remove_file(&tmp_path);
    let file_range = file_mod.image_range();
    assert_eq!(
        file_range.end as usize - file_range.start as usize,
        bytes.len()
    );

    let mut store = Store::new(&load_engine, ());
    let inst = Instance::new(&mut store, &file_mod, &[]).expect("instantiate");
    let mul = inst
        .get_typed_func::<(i32, i32), i32>(&mut store, "mul")
        .expect("mul");
    assert_eq!(mul.call(&mut store, (6, 7)).unwrap(), 42);
}

#[test]
fn reclaim_inactive_gc_space_preserves_live_objects_and_subsequent_gcs() {
    let mut cfg = Config::new();
    cfg.collector(Collector::Copying)
        .wasm_gc(true)
        .gc_heap_initial_size(2 * 1024 * 1024);
    let engine = Engine::new(&cfg).expect("engine");
    let mut store = Store::new(&engine, ());
    let arr_ty = ArrayType::new(&engine, FieldType::new(Mutability::Var, StorageType::I8));
    let pre = ArrayRefPre::new(&mut store, arr_ty);

    // Dirty both semi-spaces so `inactive` is populated.
    let mut scope = RootScope::new(&mut store);
    let kept = ArrayRef::new(&mut scope, &pre, &Val::I32(0x3C), 4096).expect("kept");
    for _ in 0..16 {
        let mut inner = RootScope::new(&mut scope);
        let _ = ArrayRef::new(&mut inner, &pre, &Val::I32(0x77), 256 * 1024).expect("garbage");
    }
    drop(scope);
    // Keep a rooted object across reclaim + another GC cycle via a Store global.
    let mut scope = RootScope::new(&mut store);
    let survivor = ArrayRef::new(&mut scope, &pre, &Val::I32(0x55), 8192).expect("survivor");
    let rooted = survivor.to_anyref().to_owned_rooted(&mut scope).unwrap();
    drop(scope);

    store.gc(None).unwrap();
    store.reclaim_inactive_gc_space();
    // Second call is a safe no-op.
    store.reclaim_inactive_gc_space();

    // Allocate again and trigger another GC flip after reclaiming inactive space.
    for _ in 0..20 {
        let mut inner = RootScope::new(&mut store);
        let _ = ArrayRef::new(&mut inner, &pre, &Val::I32(0x11), 256 * 1024).expect("post-reclaim");
    }
    store.gc(None).unwrap();

    let mut scope = RootScope::new(&mut store);
    let arr = rooted
        .to_rooted(&mut scope)
        .as_array(&scope)
        .unwrap()
        .expect("survivor array");
    assert_eq!(arr.len(&scope).unwrap(), 8192);
    assert_eq!(arr.get(&mut scope, 0).unwrap().i32(), Some(0x55));
    assert_eq!(arr.get(&mut scope, 8191).unwrap().i32(), Some(0x55));
    let _ = kept;
}

#[test]
fn generational_minor_gc_write_barriers_preserve_old_to_young_edges() {
    let wasm = wat::parse_str(
        r#"(module
            (type $node (struct (field (mut (ref null $node))) (field (mut i32))))
            (type $arr (array (mut (ref null $node))))

            (func (export "mk_node") (param i32) (result anyref)
                (struct.new $node (ref.null $node) (local.get 0)))

            (func (export "mk_arr") (param i32) (result anyref)
                (array.new_default $arr (local.get 0)))

            (func (export "set_child") (param anyref anyref)
                (struct.set $node 0
                    (ref.cast (ref $node) (local.get 0))
                    (ref.cast (ref null $node) (local.get 1))))

            (func (export "get_child_val") (param anyref) (result i32)
                (struct.get $node 1
                    (ref.as_non_null
                        (struct.get $node 0
                            (ref.cast (ref $node) (local.get 0))))))

            (func (export "arr_set") (param anyref i32 anyref)
                (array.set $arr
                    (ref.cast (ref $arr) (local.get 0))
                    (local.get 1)
                    (ref.cast (ref null $node) (local.get 2))))

            (func (export "arr_fill") (param anyref i32 anyref i32)
                (array.fill $arr
                    (ref.cast (ref $arr) (local.get 0))
                    (local.get 1)
                    (ref.cast (ref null $node) (local.get 2))
                    (local.get 3)))

            (func (export "arr_copy") (param anyref i32 anyref i32 i32)
                (array.copy $arr $arr
                    (ref.cast (ref $arr) (local.get 0))
                    (local.get 1)
                    (ref.cast (ref $arr) (local.get 2))
                    (local.get 3)
                    (local.get 4)))

            (func (export "arr_get_val") (param anyref i32) (result i32)
                (struct.get $node 1
                    (ref.as_non_null
                        (array.get $arr
                            (ref.cast (ref $arr) (local.get 0))
                            (local.get 1)))))

            ;; Allocates short-lived garbage to trigger multiple Minor GCs.
            (func (export "churn") (param $n i32) (result i32)
                (local $i i32)
                (local $sum i32)
                (loop $lp
                    (local.set $sum
                        (i32.add (local.get $sum)
                            (struct.get $node 1
                                (struct.new $node (ref.null $node) (local.get $i)))))
                    (local.set $i (i32.add (local.get $i) (i32.const 1)))
                    (br_if $lp (i32.lt_u (local.get $i) (local.get $n))))
                (local.get $sum))

            ;; Keeps 8 live GC refs in registers across allocation safepoints (Hill 10 + Hill 11).
            (func (export "live_regs_across_gc") (param $iters i32) (result i32)
                (local $a (ref $node))
                (local $b (ref $node))
                (local $c (ref $node))
                (local $d (ref $node))
                (local $e (ref $node))
                (local $f (ref $node))
                (local $g (ref $node))
                (local $h (ref $node))
                (local $i i32)
                (local.set $a (struct.new $node (ref.null $node) (i32.const 10)))
                (local.set $b (struct.new $node (ref.null $node) (i32.const 20)))
                (local.set $c (struct.new $node (ref.null $node) (i32.const 30)))
                (local.set $d (struct.new $node (ref.null $node) (i32.const 40)))
                (local.set $e (struct.new $node (ref.null $node) (i32.const 50)))
                (local.set $f (struct.new $node (ref.null $node) (i32.const 60)))
                (local.set $g (struct.new $node (ref.null $node) (i32.const 70)))
                (local.set $h (struct.new $node (ref.null $node) (i32.const 80)))
                (loop $lp
                    (drop (struct.new $node (local.get $a) (local.get $i)))
                    (local.set $i (i32.add (local.get $i) (i32.const 1)))
                    (br_if $lp (i32.lt_u (local.get $i) (local.get $iters))))
                (i32.add
                    (i32.add
                        (i32.add (struct.get $node 1 (local.get $a)) (struct.get $node 1 (local.get $b)))
                        (i32.add (struct.get $node 1 (local.get $c)) (struct.get $node 1 (local.get $d))))
                    (i32.add
                        (i32.add (struct.get $node 1 (local.get $e)) (struct.get $node 1 (local.get $f)))
                        (i32.add (struct.get $node 1 (local.get $g)) (struct.get $node 1 (local.get $h))))))
        )"#,
    )
    .expect("wat");

    let mut cfg = Config::new();
    cfg.collector(Collector::Copying)
        .wasm_gc(true)
        .gc_heap_initial_size(256 * 1024);
    let engine = Engine::new(&cfg).expect("engine");
    let module = Module::new(&engine, &wasm).expect("module");
    let mut store = Store::new(&engine, ());
    let inst = Instance::new(&mut store, &module, &[]).expect("inst");

    let mk_node = inst
        .get_typed_func::<i32, Option<Rooted<AnyRef>>>(&mut store, "mk_node")
        .unwrap();
    let mk_arr = inst
        .get_typed_func::<i32, Option<Rooted<AnyRef>>>(&mut store, "mk_arr")
        .unwrap();
    let set_child = inst
        .get_typed_func::<(Option<Rooted<AnyRef>>, Option<Rooted<AnyRef>>), ()>(
            &mut store,
            "set_child",
        )
        .unwrap();
    let get_child_val = inst
        .get_typed_func::<Option<Rooted<AnyRef>>, i32>(&mut store, "get_child_val")
        .unwrap();
    let arr_set = inst
        .get_typed_func::<(Option<Rooted<AnyRef>>, i32, Option<Rooted<AnyRef>>), ()>(
            &mut store, "arr_set",
        )
        .unwrap();
    let arr_fill = inst
        .get_typed_func::<(Option<Rooted<AnyRef>>, i32, Option<Rooted<AnyRef>>, i32), ()>(
            &mut store, "arr_fill",
        )
        .unwrap();
    let arr_copy = inst
        .get_typed_func::<(
            Option<Rooted<AnyRef>>,
            i32,
            Option<Rooted<AnyRef>>,
            i32,
            i32,
        ), ()>(&mut store, "arr_copy")
        .unwrap();
    let arr_get_val = inst
        .get_typed_func::<(Option<Rooted<AnyRef>>, i32), i32>(&mut store, "arr_get_val")
        .unwrap();
    let churn = inst
        .get_typed_func::<i32, i32>(&mut store, "churn")
        .unwrap();
    let live_regs = inst
        .get_typed_func::<i32, i32>(&mut store, "live_regs_across_gc")
        .unwrap();

    // Allocate parents and promote them into Old Space via Major GC.
    let old_parent = mk_node
        .call(&mut store, 1)
        .unwrap()
        .unwrap()
        .to_owned_rooted(&mut store)
        .unwrap();
    let old_host_parent = mk_node
        .call(&mut store, 2)
        .unwrap()
        .unwrap()
        .to_owned_rooted(&mut store)
        .unwrap();
    let old_arr = mk_arr
        .call(&mut store, 512)
        .unwrap()
        .unwrap()
        .to_owned_rooted(&mut store)
        .unwrap(); // 4 KiB > 8 cards
    let old_fill_arr = mk_arr
        .call(&mut store, 64)
        .unwrap()
        .unwrap()
        .to_owned_rooted(&mut store)
        .unwrap();
    let old_copy_dst = mk_arr
        .call(&mut store, 64)
        .unwrap()
        .unwrap()
        .to_owned_rooted(&mut store)
        .unwrap();
    store.gc(None).unwrap();

    let major_before = store.gc_major_count();
    let old_parent_raw = old_parent.to_raw(&mut store).unwrap();
    let old_arr_raw = old_arr.to_raw(&mut store).unwrap();

    // Mutate Old objects to point to fresh Young Nursery objects across 2 full rounds!
    for round in 0..2i32 {
        let base = round * 1000;
        {
            let mut scope = RootScope::new(&mut store);
            let y1 = mk_node.call(&mut scope, base + 111).unwrap().unwrap();
            let y2 = mk_node.call(&mut scope, base + 222).unwrap().unwrap();
            let y3 = mk_node.call(&mut scope, base + 333).unwrap().unwrap();
            let y4 = mk_node.call(&mut scope, base + 444).unwrap().unwrap();
            let y5 = mk_node.call(&mut scope, base + 555).unwrap().unwrap();
            let y_src = mk_arr.call(&mut scope, 8).unwrap().unwrap();

            let p = old_parent.to_rooted(&mut scope);
            let hp = old_host_parent.to_rooted(&mut scope);
            let a = old_arr.to_rooted(&mut scope);
            let fa = old_fill_arr.to_rooted(&mut scope);
            let ca = old_copy_dst.to_rooted(&mut scope);

            // Wasm struct.set, array.set (index 400 is > 3 KiB from array header!), array.fill, array.copy
            set_child.call(&mut scope, (Some(p), Some(y1))).unwrap();
            arr_set.call(&mut scope, (Some(a), 400, Some(y2))).unwrap();
            arr_fill
                .call(&mut scope, (Some(fa), 10, Some(y3), 20))
                .unwrap();
            arr_set
                .call(&mut scope, (Some(y_src), 3, Some(y4)))
                .unwrap();
            arr_copy
                .call(&mut scope, (Some(ca), 12, Some(y_src), 3, 1))
                .unwrap();

            // Host StructRef::set_field + ArrayRef::set write barriers!
            hp.as_struct(&scope)
                .unwrap()
                .unwrap()
                .set_field(&mut scope, 0, Val::AnyRef(Some(y5)))
                .unwrap();
            a.as_array(&scope)
                .unwrap()
                .unwrap()
                .set(&mut scope, 500, Val::AnyRef(Some(y5)))
                .unwrap();
        }

        // Churn ~400 KiB of short-lived garbage -> triggers >= 3 Minor GCs without copying Old Space!
        let _ = churn.call(&mut store, 16_000).unwrap();

        let mut scope = RootScope::new(&mut store);
        let p = old_parent.to_rooted(&mut scope);
        let hp = old_host_parent.to_rooted(&mut scope);
        let a = old_arr.to_rooted(&mut scope);
        let fa = old_fill_arr.to_rooted(&mut scope);
        let ca = old_copy_dst.to_rooted(&mut scope);

        assert_eq!(get_child_val.call(&mut scope, Some(p)).unwrap(), base + 111);
        assert_eq!(
            arr_get_val.call(&mut scope, (Some(a), 400)).unwrap(),
            base + 222
        );
        assert_eq!(
            arr_get_val.call(&mut scope, (Some(fa), 25)).unwrap(),
            base + 333
        );
        assert_eq!(
            arr_get_val.call(&mut scope, (Some(ca), 12)).unwrap(),
            base + 444
        );
        assert_eq!(
            get_child_val.call(&mut scope, Some(hp)).unwrap(),
            base + 555
        );
        assert_eq!(
            arr_get_val.call(&mut scope, (Some(a), 500)).unwrap(),
            base + 555
        );
    }

    assert!(
        store.gc_minor_count() >= 5,
        "expected >= 5 minor GCs, got {}",
        store.gc_minor_count()
    );
    assert_eq!(
        store.gc_major_count(),
        major_before,
        "Old Space should not have triggered a Major GC while headroom remained"
    );
    assert_eq!(
        old_parent.to_raw(&mut store).unwrap(),
        old_parent_raw,
        "Old object address must remain stable across Minor GCs"
    );
    assert_eq!(
        old_arr.to_raw(&mut store).unwrap(),
        old_arr_raw,
        "Old array address must remain stable across Minor GCs"
    );

    // Exercise Hill 10 register-allocated GC refs across both Minor and Major GCs!
    assert_eq!(live_regs.call(&mut store, 40_000).unwrap(), 360);
    assert!(store.gc_minor_count() >= 10);
}

#[test]
fn generational_minor_gc_weakmap_ephemeron_and_externref_semantics() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use wtime_aot_host::{
        ArrayRef, ArrayRefPre, ArrayType, ExternRef, FieldType, GcEphemeron, Mutability,
        StorageType, StructRef, StructRefPre, StructType, WeakAnyRef,
    };

    struct DropCounter(Arc<AtomicUsize>);
    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    let mut cfg = Config::new();
    cfg.collector(Collector::Copying)
        .wasm_gc(true)
        .gc_heap_initial_size(256 * 1024);
    let engine = Engine::new(&cfg).expect("engine");
    let mut store = Store::new(&engine, ());

    let st_ty = StructType::new(
        &engine,
        [
            FieldType::new(
                Mutability::Var,
                StorageType::ValType(wtime_aot_host::ValType::I32),
            ),
            FieldType::new(
                Mutability::Var,
                StorageType::ValType(wtime_aot_host::ValType::EXTERNREF),
            ),
        ],
    )
    .unwrap();
    let st_pre = StructRefPre::new(&mut store, st_ty);
    let eq_arr_ty = ArrayType::new(
        &engine,
        FieldType::new(
            Mutability::Var,
            StorageType::ValType(wtime_aot_host::ValType::ANYREF),
        ),
    );
    let eq_arr_pre = ArrayRefPre::new(&mut store, eq_arr_ty);
    let i8_arr_ty = ArrayType::new(&engine, FieldType::new(Mutability::Var, StorageType::I8));
    let i8_pre = ArrayRefPre::new(&mut store, i8_arr_ty);

    let dropped_live = Arc::new(AtomicUsize::new(0));
    let dropped_dead = Arc::new(AtomicUsize::new(0));

    // Create Old objects: weakmap table, old key, old owner, and old struct holding live ExternRef.
    let (old_wm, old_owner, old_key, old_ext_holder) = {
        let mut scope = RootScope::new(&mut store);
        let ext_live = ExternRef::new(&mut scope, DropCounter(Arc::clone(&dropped_live))).unwrap();
        let _ext_dead = ExternRef::new(&mut scope, DropCounter(Arc::clone(&dropped_dead))).unwrap();
        let holder = StructRef::new(
            &mut scope,
            &st_pre,
            &[Val::I32(99), Val::ExternRef(Some(ext_live))],
        )
        .unwrap();
        let wm = ArrayRef::new(&mut scope, &eq_arr_pre, &Val::AnyRef(None), 6).unwrap();
        let owner =
            StructRef::new(&mut scope, &st_pre, &[Val::I32(1), Val::ExternRef(None)]).unwrap();
        let key =
            StructRef::new(&mut scope, &st_pre, &[Val::I32(2), Val::ExternRef(None)]).unwrap();
        let wm_raw = wm.to_anyref().to_raw(&mut scope).unwrap();
        unsafe { GcEphemeron::register_table_raw(&mut scope, wm_raw).unwrap() };
        (
            wm.to_owned_rooted(&mut scope).unwrap(),
            owner.to_owned_rooted(&mut scope).unwrap(),
            key.to_owned_rooted(&mut scope).unwrap(),
            holder.to_owned_rooted(&mut scope).unwrap(),
        )
    };
    // Promote them to Old Space (and sweep `_ext_dead`).
    store.gc(None).unwrap();
    assert_eq!(dropped_dead.load(Ordering::SeqCst), 1);
    assert_eq!(dropped_live.load(Ordering::SeqCst), 0);
    let major_before = store.gc_major_count();

    // Populate Old WeakMap, Ephemerons, and WeakRefs with Young objects!
    let (eph_live, eph_dead, weak_live, weak_dead, young_live_key) = {
        let mut scope = RootScope::new(&mut store);
        let y_val_for_old_key =
            StructRef::new(&mut scope, &st_pre, &[Val::I32(777), Val::ExternRef(None)]).unwrap();
        let y_live_k =
            StructRef::new(&mut scope, &st_pre, &[Val::I32(888), Val::ExternRef(None)]).unwrap();
        let y_live_v =
            StructRef::new(&mut scope, &st_pre, &[Val::I32(889), Val::ExternRef(None)]).unwrap();
        let y_dead_k =
            StructRef::new(&mut scope, &st_pre, &[Val::I32(666), Val::ExternRef(None)]).unwrap();
        let y_dead_v =
            StructRef::new(&mut scope, &st_pre, &[Val::I32(667), Val::ExternRef(None)]).unwrap();
        let y_eph_v =
            StructRef::new(&mut scope, &st_pre, &[Val::I32(555), Val::ExternRef(None)]).unwrap();

        let wm = old_wm.to_rooted(&mut scope);
        let ok = old_key.to_rooted(&mut scope);
        wm.set(&mut scope, 0, Val::AnyRef(Some(ok.to_anyref())))
            .unwrap();
        wm.set(
            &mut scope,
            1,
            Val::AnyRef(Some(y_val_for_old_key.to_anyref())),
        )
        .unwrap();
        wm.set(&mut scope, 2, Val::AnyRef(Some(y_live_k.to_anyref())))
            .unwrap();
        wm.set(&mut scope, 3, Val::AnyRef(Some(y_live_v.to_anyref())))
            .unwrap();
        wm.set(&mut scope, 4, Val::AnyRef(Some(y_dead_k.to_anyref())))
            .unwrap();
        wm.set(&mut scope, 5, Val::AnyRef(Some(y_dead_v.to_anyref())))
            .unwrap();

        let e_live = GcEphemeron::new(&mut scope, &old_owner, &old_key, Some(&y_eph_v)).unwrap();
        let e_dead = GcEphemeron::new(&mut scope, &old_owner, &y_dead_k, Some(&y_dead_v)).unwrap();
        let w_live = WeakAnyRef::new(&mut scope, &old_owner, &y_live_k).unwrap();
        let w_dead = WeakAnyRef::new(&mut scope, &old_owner, &y_dead_k).unwrap();
        (
            e_live,
            e_dead,
            w_live,
            w_dead,
            y_live_k.to_owned_rooted(&mut scope).unwrap(),
        )
    };

    // Trigger several Minor GCs via short-lived byte arrays (NOT a Major GC).
    for _ in 0..12 {
        let mut scope = RootScope::new(&mut store);
        let _ = ArrayRef::new(&mut scope, &i8_pre, &Val::I32(3), 24 * 1024).unwrap();
    }
    assert!(store.gc_minor_count() >= 2);
    assert_eq!(store.gc_major_count(), major_before);
    // Live ExternRef inside Old struct MUST NOT be dropped by Minor GC!
    assert_eq!(dropped_live.load(Ordering::SeqCst), 0);

    // Check WeakMap, Ephemerons, and WeakRefs after Minor GC.
    {
        let mut scope = RootScope::new(&mut store);
        let wm = old_wm.to_rooted(&mut scope);
        let v0 = wm
            .get(&mut scope, 1)
            .unwrap()
            .unwrap_anyref()
            .expect("old_key -> young_val promoted");
        assert_eq!(
            v0.as_struct(&scope)
                .unwrap()
                .unwrap()
                .field(&mut scope, 0)
                .unwrap()
                .unwrap_i32(),
            777
        );
        let v1 = wm
            .get(&mut scope, 3)
            .unwrap()
            .unwrap_anyref()
            .expect("young_live_k -> young_live_v promoted");
        assert_eq!(
            v1.as_struct(&scope)
                .unwrap()
                .unwrap()
                .field(&mut scope, 0)
                .unwrap()
                .unwrap_i32(),
            889
        );
        assert!(
            wm.get(&mut scope, 4).unwrap().unwrap_anyref().is_none(),
            "dead young key must clear"
        );
        assert!(
            wm.get(&mut scope, 5).unwrap().unwrap_anyref().is_none(),
            "dead young val must clear"
        );

        assert!(eph_live.is_live(&scope).unwrap());
        let ev = eph_live
            .value(&mut scope)
            .unwrap()
            .expect("ephemeron young value promoted");
        assert_eq!(
            ev.as_struct(&scope)
                .unwrap()
                .unwrap()
                .field(&mut scope, 0)
                .unwrap()
                .unwrap_i32(),
            555
        );
        assert!(!eph_dead.is_live(&scope).unwrap());

        assert!(weak_live.upgrade(&mut scope).unwrap().is_some());
        assert!(weak_dead.upgrade(&mut scope).unwrap().is_none());
    }

    // Now drop `old_ext_holder` and trigger Major GC -> live ExternRef must finally be swept!
    drop(old_ext_holder);
    drop(young_live_key);
    store.gc(None).unwrap();
    assert_eq!(dropped_live.load(Ordering::SeqCst), 1);
}

#[test]
fn store_snapshot_roundtrips_gc_heap_funcrefs_globals_memory_and_weak_state() {
    use wtime_aot_host::{GcEphemeron, WeakAnyRef};

    let wasm = wat::parse_str(
        r#"(module
            (type $fn_i32 (func (param i32) (result i32)))
            (type $closure (struct (field (ref $fn_i32)) (field (mut i32)) (field (mut (ref null $closure)))))
            (type $arr (array (mut (ref null $closure))))

            (import "host" "bump" (func $host_bump (type $fn_i32)))

            (memory 1)
            (table $ftbl 2 funcref)
            (global $g_root (mut (ref null $closure)) (ref.null $closure))
            (global $g_fn (mut (ref null $fn_i32)) (ref.null $fn_i32))
            (global $g_counter (mut i32) (i32.const 0))

            (elem declare func $add_ten $host_bump)

            (func $add_ten (type $fn_i32)
                (i32.add (local.get 0) (i32.const 10)))

            (func (export "init_state") (result eqref)
                (local $leaf (ref $closure))
                (local $head (ref $closure))
                (local.set $leaf (struct.new $closure (ref.func $host_bump) (i32.const 7) (ref.null $closure)))
                (local.set $head (struct.new $closure (ref.func $add_ten) (i32.const 3) (local.get $leaf)))
                (global.set $g_root (local.get $head))
                (global.set $g_fn (ref.func $add_ten))
                (global.set $g_counter (i32.const 42))
                (table.set $ftbl (i32.const 0) (ref.func $add_ten))
                (table.set $ftbl (i32.const 1) (ref.func $host_bump))
                (i32.store (i32.const 64) (i32.const 0x12345678))
                (local.get $head))

            (func (export "invoke_snapshot_state") (param $x i32) (result i32)
                (local $head (ref $closure))
                (local $leaf (ref $closure))
                (local $r1 i32)
                (local $r2 i32)
                (local $r3 i32)
                (local.set $head (ref.as_non_null (global.get $g_root)))
                (local.set $leaf (ref.as_non_null (struct.get $closure 2 (local.get $head))))
                ;; Call Wasm funcref stored inside GC struct (`$add_ten(x)` = x + 10)
                (local.set $r1 (call_ref $fn_i32 (local.get $x) (struct.get $closure 0 (local.get $head))))
                ;; Call Host funcref stored inside nested GC struct (`$host_bump(r1)` = r1 + host_bias)
                (local.set $r2 (call_ref $fn_i32 (local.get $r1) (struct.get $closure 0 (local.get $leaf))))
                ;; Call indirect via restored table[1] (`$host_bump(100)`) + linear memory + global counter
                (local.set $r3 (call_indirect $ftbl (type $fn_i32) (i32.const 100) (i32.const 1)))
                (i32.add
                    (i32.add (local.get $r2) (local.get $r3))
                    (i32.add (i32.load (i32.const 64)) (global.get $g_counter))))

            (func (export "mutate_and_churn") (param $new_val i32) (result i32)
                (local $head (ref $closure))
                (local $fresh (ref $closure))
                (local $i i32)
                (local.set $head (ref.as_non_null (global.get $g_root)))
                (local.set $fresh (struct.new $closure (ref.func $add_ten) (local.get $new_val) (ref.null $closure)))
                ;; Write barrier on Old restored object -> Young fresh child!
                (struct.set $closure 2 (local.get $head) (local.get $fresh))
                (loop $lp
                    (drop (struct.new $closure (ref.func $add_ten) (local.get $i) (ref.null $closure)))
                    (local.set $i (i32.add (local.get $i) (i32.const 1)))
                    (br_if $lp (i32.lt_u (local.get $i) (i32.const 12000))))
                (struct.get $closure 1 (ref.as_non_null (struct.get $closure 2 (local.get $head)))))
        )"#,
    )
    .expect("wat");

    let mut cfg = Config::new();
    cfg.collector(Collector::Copying)
        .wasm_gc(true)
        .gc_heap_initial_size(256 * 1024);
    let engine = Engine::new(&cfg).expect("engine");
    let module = Module::new(&engine, &wasm).expect("module");

    let make_store = |bias: i32| {
        let mut store = Store::new(&engine, bias);
        let mut linker = Linker::new(&engine);
        linker
            .func_wrap("host", "bump", |caller: Caller<'_, i32>, v: i32| -> i32 {
                v + *caller.data()
            })
            .unwrap();
        let inst = linker.instantiate(&mut store, &module).unwrap();
        (store, inst)
    };

    // 1. Initialize Store A (host bias = 1000) and capture snapshot + extra root + ephemeron + weak ref.
    let (mut store_a, inst_a) = make_store(1000);
    let init_a = inst_a
        .get_typed_func::<(), Option<Rooted<EqRef>>>(&mut store_a, "init_state")
        .unwrap();
    let head_root = init_a
        .call(&mut store_a, ())
        .unwrap()
        .unwrap()
        .to_owned_rooted(&mut store_a)
        .unwrap();
    let eph_a = GcEphemeron::new(&mut store_a, &head_root, &head_root, Some(&head_root)).unwrap();
    let weak_a = WeakAnyRef::new(&mut store_a, &head_root, &head_root).unwrap();

    let snap = store_a
        .capture_snapshot(&[&head_root])
        .expect("capture snapshot");
    assert!(snap.heap_bytes_len() > 0);
    drop(store_a); // Destroy Store A completely before restoring into Store B & C!

    // 2. Restore into Store B (host bias = 5) and Store C (host bias = 200) — verifies host FuncRef remapping!
    for &(bias, mutate_v) in &[(5i32, 9001i32), (200i32, 7777i32)] {
        let (mut store, inst) = make_store(bias);
        let restored_roots = store.restore_snapshot(&snap).expect("restore snapshot");
        assert_eq!(restored_roots.len(), 1);

        // Verify rebound ephemeron & weak ref resolve cleanly to the restored root in this Store.
        let eph = store.rebind_ephemeron(eph_a);
        let weak = store.rebind_weak_ref(weak_a);
        assert!(eph.is_live(&store).unwrap());
        {
            let mut scope = RootScope::new(&mut store);
            let e_val = eph.value(&mut scope).unwrap().expect("ephemeron value");
            let w_val = weak.upgrade(&mut scope).unwrap().expect("weak upgrade");
            let r_val = restored_roots[0].to_rooted(&mut scope).to_anyref();
            assert!(Rooted::ref_eq(&scope, &e_val, &r_val).unwrap());
            assert!(Rooted::ref_eq(&scope, &w_val, &r_val).unwrap());
        }

        // Invoke restored GC closures (both Wasm & Host FuncRefs!), indirect table, linear memory, globals!
        let invoke = inst
            .get_typed_func::<i32, i32>(&mut store, "invoke_snapshot_state")
            .unwrap();
        // x=5 -> r1=15, r2=15+bias, r3=100+bias, mem=0x12345678, global=42
        let expected = (15 + bias) + (100 + bias) + 0x1234_5678 + 42;
        assert_eq!(invoke.call(&mut store, 5).unwrap(), expected);

        // Mutate restored Old object to point to fresh Young child + trigger Minor GCs + Major GC!
        let mutate = inst
            .get_typed_func::<i32, i32>(&mut store, "mutate_and_churn")
            .unwrap();
        assert_eq!(mutate.call(&mut store, mutate_v).unwrap(), mutate_v);
        assert!(store.gc_minor_count() >= 1);
        store.gc(None).unwrap();
        assert!(eph.is_live(&store).unwrap());
    }
}
