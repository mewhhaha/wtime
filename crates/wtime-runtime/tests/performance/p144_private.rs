use super::*;
use crate::{Config, Engine, ExecutionPolicy, External, Linker};

fn setup(adaptive: bool) -> (Store, Module, Frame) {
    let engine = if adaptive {
        Engine::with_register_entry_threshold(Config::default(), 1).unwrap()
    } else {
        Engine::with_execution_policy(Config::default(), ExecutionPolicy::Register).unwrap()
    };
    let module = engine
        .compile(include_bytes!(
            "../fixtures/performance/p138/uniform66.wasm"
        ))
        .unwrap();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .unwrap();
    let External::Func(func) = instance.export(&store, "f00").unwrap() else {
        panic!()
    };
    let frame = store.frame(func, vec![Value::I32(7)], 0).unwrap();
    assert!(frame.register.is_some());
    (store, module, frame)
}

fn independent_bytes(frame: &Frame) -> usize {
    frame.register.as_ref().unwrap().slots.capacity() * size_of::<u64>()
        + frame.labels.capacity() * size_of::<Label>()
}

#[test]
fn p144_private_capacity_tracks_reservation_not_vector_length() {
    let (store, _module, mut frame) = setup(false);
    let scope = store.register_activation.scope();
    let before = independent_bytes(&frame);
    scope.observe(std::slice::from_ref(&frame));
    assert_eq!(store.register_activation_snapshot().current_bytes, before);
    frame.register.as_mut().unwrap().slots.reserve(128);
    frame.labels.reserve(64);
    let after = independent_bytes(&frame);
    assert!(after > before);
    scope.observe(std::slice::from_ref(&frame));
    assert_eq!(store.register_activation_snapshot().peak_bytes, after);
    assert_eq!(store.register_activation_snapshot().peak_frames, 1);
    drop(scope);
    assert_eq!(store.register_activation_snapshot().current_bytes, 0);
    eprintln!(
        "P144_METRIC {{\"case\":\"capacity_oracle\",\"before\":{before},\"after\":{after},\"capacity_not_length\":true}}"
    );
}

#[test]
fn p144_private_nested_scopes_restore_outer_capacity_and_reject_reset() {
    let (mut store, _module, frame) = setup(false);
    let second = frame.clone();
    let first_bytes = independent_bytes(&frame);
    let second_bytes = independent_bytes(&second);
    let outer = store.register_activation.scope();
    outer.observe(std::slice::from_ref(&frame));
    {
        let inner = store.register_activation.scope();
        inner.observe(std::slice::from_ref(&second));
        let observed = store.register_activation_snapshot();
        assert_eq!(
            (
                observed.current_bytes,
                observed.current_frames,
                observed.active_scopes
            ),
            (first_bytes + second_bytes, 2, 2)
        );
        assert!(store.reset_register_activation_snapshot().is_err());
        assert_eq!(store.register_activation_snapshot(), observed);
    }
    assert_eq!(
        store.register_activation_snapshot().current_bytes,
        first_bytes
    );
    assert_eq!(store.register_activation_snapshot().active_scopes, 1);
    drop(outer);
    assert_eq!(store.register_activation_snapshot().current_bytes, 0);
    assert_eq!(
        store.register_activation_snapshot().peak_bytes,
        first_bytes + second_bytes
    );
    store.reset_register_activation_snapshot().unwrap();
    assert_eq!(
        store.register_activation_snapshot(),
        crate::RegisterActivationSnapshot::default()
    );
    eprintln!(
        "P144_METRIC {{\"case\":\"nested_oracle\",\"first\":{first_bytes},\"second\":{second_bytes},\"sum\":{},\"reset_preserved_active\":true}}",
        first_bytes + second_bytes
    );
}

#[test]
fn p144_private_scope_unwind_restores_zero() {
    let (mut store, _module, frame) = setup(false);
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let scope = store.register_activation.scope();
        scope.observe(std::slice::from_ref(&frame));
        panic!("test-only scope unwind");
    }));
    assert!(caught.is_err());
    let snapshot = store.register_activation_snapshot();
    assert_eq!(
        (
            snapshot.current_bytes,
            snapshot.current_frames,
            snapshot.active_scopes
        ),
        (0, 0, 0)
    );
    assert!(snapshot.peak_bytes > 0 && !snapshot.overflowed);
    store.reset_register_activation_snapshot().unwrap();
}

#[test]
fn p144_private_pinned_eviction_keeps_code_and_working_bytes_separate() {
    let (store, module, frame) = setup(true);
    let scope = store.register_activation.scope();
    scope.observe(std::slice::from_ref(&frame));
    let capacity = independent_bytes(&frame);
    let engine = store.engine.clone();
    let mut other = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut other, &module)
        .unwrap();
    for index in 1..66 {
        let External::Func(func) = instance.export(&other, &format!("f{index:02}")).unwrap() else {
            panic!()
        };
        assert_eq!(
            other.invoke(func, &[Value::I32(7)]).unwrap(),
            crate::RunOutcome::Returned(vec![Value::I32(7 + index)])
        );
    }
    let cache = module.register_cache_snapshot().unwrap();
    assert!(cache.lifecycle.evictions >= 2);
    assert!(!cache.entries.iter().any(|entry| entry.body == 0));
    assert!(cache.retained_bytes <= cache.byte_limit);
    assert_eq!(store.register_activation_snapshot().current_bytes, capacity);
    assert!(cache.retained_bytes > capacity);
    drop(frame);
    drop(scope);
    assert_eq!(store.register_activation_snapshot().current_bytes, 0);
    eprintln!(
        "P144_METRIC {{\"case\":\"pinned_eviction\",\"working_bytes\":{capacity},\"code_bytes\":{},\"evictions\":{},\"cleanup\":true}}",
        cache.retained_bytes, cache.lifecycle.evictions
    );
}
