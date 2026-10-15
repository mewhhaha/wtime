use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Snapshot {
    live: usize,
    payload: usize,
    field_bytes: usize,
    slot_bytes: usize,
    free_bytes: usize,
    slots: usize,
}
impl Snapshot {
    fn read(arena: &GcArena) -> Self {
        let field_bytes = arena
            .slots
            .iter()
            .filter_map(|slot| slot.object.as_ref())
            .map(|object| object.fields.capacity() * std::mem::size_of::<Value>())
            .sum();
        assert_eq!(
            arena.bytes(),
            arena.live() * std::mem::size_of::<Object>() + field_bytes
        );
        Self {
            live: arena.live(),
            payload: arena.bytes(),
            field_bytes,
            slot_bytes: arena.slots.capacity() * std::mem::size_of::<Slot>(),
            free_bytes: arena.free.capacity() * std::mem::size_of::<usize>(),
            slots: arena.slots.len(),
        }
    }
    fn json(self) -> String {
        format!(
            "{{\"live\":{},\"payload_charge_bytes\":{},\"field_capacity_bytes\":{},\"slot_capacity_bytes\":{},\"free_capacity_bytes\":{},\"slot_count\":{}}}",
            self.live, self.payload, self.field_bytes, self.slot_bytes, self.free_bytes, self.slots
        )
    }
}
fn object(fields: usize) -> Object {
    Object {
        ty: CanonicalTypeId::test(1, 0),
        kind: ObjectKind::Array,
        fields: vec![Value::I32(7); fields],
        descriptor: None,
    }
}

#[test]
fn p079_gc_large_small_reuses_bounded_metadata_and_releases_payloads() {
    let mut arena = GcArena::new(1);
    let roots_owner = Arc::downgrade(&arena.roots);
    let before = Snapshot::read(&arena);
    let roots: Vec<_> = (0..1024)
        .map(|_| arena.allocate(object(16), 1024).unwrap())
        .collect();
    let large = Snapshot::read(&arena);
    assert_eq!(large.live, 1024);
    assert_eq!(arena.collect(roots).unwrap().live, 1024);
    assert_eq!(Snapshot::read(&arena), large);
    assert_eq!(arena.collect([]).unwrap().reclaimed, 1024);
    let cleared = Snapshot::read(&arena);
    assert_eq!(
        (cleared.live, cleared.payload, cleared.field_bytes),
        (0, 0, 0)
    );
    assert!(cleared.slot_bytes >= large.slot_bytes);
    assert!(cleared.slot_bytes + cleared.free_bytes > 0);
    let small_roots: Vec<_> = (0..4)
        .map(|_| arena.allocate(object(1), 1024).unwrap())
        .collect();
    assert_eq!(arena.collect(small_roots).unwrap().live, 4);
    let small = Snapshot::read(&arena);
    assert_eq!(small.slots, large.slots);
    assert_eq!(small.slot_bytes, large.slot_bytes);
    assert_eq!(small.free_bytes, cleared.free_bytes);
    assert!(small.payload < large.payload);
    assert_eq!(arena.collect([]).unwrap().reclaimed, 4);
    let final_state = Snapshot::read(&arena);
    assert_eq!(
        (
            final_state.live,
            final_state.payload,
            final_state.field_bytes
        ),
        (0, 0, 0)
    );
    eprintln!(
        "P079_METRIC {{\"case\":\"gc_large_small\",\"before\":{},\"large\":{},\"cleared\":{},\"small\":{},\"final\":{},\"metadata_lifetime\":\"arena\",\"root_hashmap_capacity\":{}}}",
        before.json(),
        large.json(),
        cleared.json(),
        small.json(),
        final_state.json(),
        arena.roots.lock().unwrap().values.capacity()
    );
    drop(arena);
    assert!(roots_owner.upgrade().is_none());
}

#[test]
fn p079_gc_budget_failures_publish_no_partial_object() {
    let mut arena = GcArena::new(1);
    let reference = arena.allocate(object(4), 1).unwrap();
    let before = Snapshot::read(&arena);
    assert_eq!(
        arena.allocate(object(4), 1).unwrap_err().kind,
        ErrorKind::ResourceLimit
    );
    assert_eq!(Snapshot::read(&arena), before);
    arena.set_limits(before.payload, 100);
    assert_eq!(
        arena.allocate(object(512), 1024).unwrap_err().kind,
        ErrorKind::ResourceLimit
    );
    assert_eq!(Snapshot::read(&arena), before);
    assert_eq!(arena.get(reference).unwrap().fields, vec![Value::I32(7); 4]);
    assert_eq!(arena.collect([reference]).unwrap().live, 1);
    assert_eq!(arena.collect([]).unwrap().reclaimed, 1);
    let final_state = Snapshot::read(&arena);
    eprintln!(
        "P079_METRIC {{\"case\":\"gc_failure\",\"before\":{},\"final\":{},\"object_failure_atomic\":true,\"byte_failure_atomic\":true}}",
        before.json(),
        final_state.json()
    );
}
