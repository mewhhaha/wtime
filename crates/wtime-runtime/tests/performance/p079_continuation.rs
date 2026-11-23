use super::*;

fn physical_state_bytes(store: &Store) -> usize {
    store
        .continuations
        .values()
        .map(|state| {
            let mut bytes = std::mem::size_of::<ContinuationState>()
                + (state.signature.params.capacity() + state.signature.results.capacity())
                    * std::mem::size_of::<ValType>()
                + (state.type_ids.0.capacity() + state.type_ids.1.capacity())
                    * std::mem::size_of::<Option<CanonicalTypeId>>()
                + state.bound.capacity() * std::mem::size_of::<Value>();
            if let Some(Body::Segment(segment)) = &state.body {
                bytes += segment.frames.capacity() * std::mem::size_of::<Frame>()
                    + segment.operands.capacity() * std::mem::size_of::<Value>()
                    + segment.prompts.capacity() * std::mem::size_of::<Prompt>();
                for frame in &segment.frames {
                    bytes += frame.locals.capacity() * std::mem::size_of::<Value>()
                        + frame.labels.capacity() * std::mem::size_of::<crate::execution::Label>()
                        + (frame.function.ty.params.capacity()
                            + frame.function.ty.results.capacity())
                            * std::mem::size_of::<ValType>();
                    bytes += frame
                        .labels
                        .iter()
                        .map(|label| {
                            label.catches.capacity() * std::mem::size_of::<wtime_validate::Catch>()
                        })
                        .sum::<usize>();
                }
                bytes += segment
                    .prompts
                    .iter()
                    .map(|prompt| {
                        prompt.handlers.capacity() * std::mem::size_of::<(Tag, Option<u32>)>()
                    })
                    .sum::<usize>();
            }
            bytes
        })
        .sum()
}

#[test]
fn p079_continuation_large_small_matches_owned_capacities_and_reclaims() {
    let engine = Engine::new(Config::default()).unwrap();
    let large = engine
        .compile(include_bytes!(
            "../fixtures/performance/bench_wasm/continuations/depth_32.wasm"
        ))
        .unwrap();
    let small = engine
        .compile(include_bytes!(
            "../fixtures/performance/bench_wasm/continuations/depth_0.wasm"
        ))
        .unwrap();
    let mut store = Store::new(&engine);
    let large = Linker::new(&engine)
        .instantiate(&mut store, &large)
        .unwrap();
    let small = Linker::new(&engine)
        .instantiate(&mut store, &small)
        .unwrap();
    let mut snapshots = vec![];
    for (instance, expected) in [(large, 1537), (small, 1009), (large, 1537), (small, 1009)] {
        let External::Func(run) = instance.export(&store, "run").unwrap() else {
            panic!()
        };
        let External::Func(resume) = instance.export(&store, "resume").unwrap() else {
            panic!()
        };
        assert_eq!(store.continuation_bytes(), 0);
        let RunOutcome::Suspended {
            continuation,
            payload,
            ..
        } = store.invoke(run, &[]).unwrap()
        else {
            panic!()
        };
        let physical = physical_state_bytes(&store);
        assert!(physical > 0);
        assert_eq!(store.continuation_bytes(), physical);
        let root = store.root(continuation).unwrap();
        let value = Value::Ref(RefValue::Gc(continuation));
        store.release_published(&payload);
        store.release_published(&[value]);
        assert_eq!(store.collect().unwrap().live, 1);
        assert_eq!(
            store.invoke(resume, &[value]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(expected)])
        );
        assert_eq!(store.continuation_bytes(), physical_state_bytes(&store));
        drop(root);
        assert_eq!(store.collect().unwrap().live, 0);
        assert_eq!(
            (
                store.continuation_bytes(),
                store.gc_payload_bytes(),
                store.continuations.len()
            ),
            (0, 0, 0)
        );
        snapshots.push(physical);
    }
    assert!(snapshots[0] > snapshots[1]);
    assert_eq!(snapshots[0], snapshots[2]);
    assert_eq!(snapshots[1], snapshots[3]);
    eprintln!(
        "P079_METRIC {{\"case\":\"continuation_large_small\",\"capture_bytes\":[{},{},{},{}],\"physical_scan_matches\":true,\"final_continuation_bytes\":0,\"final_gc_payload_bytes\":0,\"final_states\":0,\"registry_capacity\":{},\"registry_lifetime\":\"store\"}}",
        snapshots[0],
        snapshots[1],
        snapshots[2],
        snapshots[3],
        store.continuations.capacity()
    );
}
