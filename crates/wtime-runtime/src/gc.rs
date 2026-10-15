//! Nonmoving, generation-checked guest heap with explicit roots.
#[cfg(test)]
#[path = "../tests/performance/p079_gc.rs"]
mod p079;
use super::*;
use crate::canonical::CanonicalTypeId;
use std::sync::{Mutex, Weak};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GcRef {
    store: u64,
    index: usize,
    generation: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ObjectKind {
    Struct,
    Array,
    Continuation,
}
#[derive(Clone, Debug)]
pub(crate) struct Object {
    pub ty: CanonicalTypeId,
    pub kind: ObjectKind,
    pub fields: Vec<Value>,
    pub descriptor: Option<GcRef>,
}
struct Slot {
    generation: u64,
    marked: bool,
    object: Option<Object>,
    bytes: usize,
}
#[derive(Default)]
struct Roots {
    next: u64,
    values: HashMap<u64, GcRef>,
}
/// Clones share a registration; the final token releases it without retaining Store.
#[derive(Clone)]
pub struct RootToken {
    registration: Arc<Registration>,
}
struct Registration {
    roots: Weak<Mutex<Roots>>,
    key: u64,
    value: GcRef,
}
impl Drop for Registration {
    fn drop(&mut self) {
        if let Some(roots) = self.roots.upgrade()
            && let Ok(mut roots) = roots.lock()
        {
            roots.values.remove(&self.key);
        }
    }
}
impl RootToken {
    pub fn reference(&self) -> GcRef {
        self.registration.value
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CollectionStats {
    pub live: usize,
    pub reclaimed: usize,
}
pub(crate) struct GcArena {
    store: u64,
    slots: Vec<Slot>,
    free: Vec<usize>,
    roots: Arc<Mutex<Roots>>,
    live: usize,
    bytes: usize,
    byte_limit: usize,
    mark_limit: usize,
}
impl GcArena {
    pub fn new(store: u64) -> Self {
        Self {
            store,
            slots: vec![],
            free: vec![],
            roots: Arc::new(Mutex::new(Roots::default())),
            live: 0,
            bytes: 0,
            byte_limit: ResourceLimits::default().max_gc_bytes,
            mark_limit: ResourceLimits::default().max_gc_mark_work,
        }
    }
    pub fn set_limits(&mut self, bytes: usize, mark_work: usize) {
        self.byte_limit = bytes;
        self.mark_limit = mark_work;
    }
    pub fn check_payload(&self, count: usize) -> Result<()> {
        count
            .checked_mul(std::mem::size_of::<Value>())
            .and_then(|n| n.checked_add(std::mem::size_of::<Object>()))
            .and_then(|n| self.bytes.checked_add(n))
            .filter(|n| *n <= self.byte_limit)
            .ok_or_else(|| err(ErrorKind::ResourceLimit, "guest heap byte budget exceeded"))?;
        Ok(())
    }
    pub fn bytes(&self) -> usize {
        self.bytes
    }
    pub fn live_values(&self) -> impl Iterator<Item = &Value> {
        self.slots
            .iter()
            .filter_map(|s| s.object.as_ref())
            .flat_map(|o| o.fields.iter())
    }
    fn index(&self, r: GcRef) -> Result<usize> {
        if r.store != self.store {
            return Err(err(ErrorKind::Host, "foreign GC reference"));
        }
        let slot = self
            .slots
            .get(r.index)
            .ok_or_else(|| err(ErrorKind::Host, "invalid GC reference"))?;
        if slot.generation != r.generation || slot.object.is_none() {
            return Err(err(ErrorKind::Host, "stale GC reference"));
        }
        Ok(r.index)
    }
    pub fn get(&self, r: GcRef) -> Result<&Object> {
        Ok(self.slots[self.index(r)?].object.as_ref().unwrap())
    }
    pub fn get_mut(&mut self, r: GcRef) -> Result<&mut Object> {
        let i = self.index(r)?;
        Ok(self.slots[i].object.as_mut().unwrap())
    }
    pub fn live(&self) -> usize {
        self.live
    }
    pub fn allocate(&mut self, object: Object, limit: usize) -> Result<GcRef> {
        if self.live >= limit {
            return Err(err(
                ErrorKind::ResourceLimit,
                "guest heap object budget exceeded",
            ));
        }
        let bytes = object
            .fields
            .capacity()
            .checked_mul(std::mem::size_of::<Value>())
            .and_then(|n| n.checked_add(std::mem::size_of::<Object>()))
            .ok_or_else(|| err(ErrorKind::ResourceLimit, "guest heap size overflow"))?;
        let total = self
            .bytes
            .checked_add(bytes)
            .filter(|n| *n <= self.byte_limit)
            .ok_or_else(|| err(ErrorKind::ResourceLimit, "guest heap byte budget exceeded"))?;
        for r in object
            .fields
            .iter()
            .filter_map(reference)
            .chain(object.descriptor)
        {
            self.index(r)?;
        }
        let i = if let Some(i) = self.free.pop() {
            i
        } else {
            self.slots
                .try_reserve(1)
                .map_err(|_| err(ErrorKind::ResourceLimit, "guest heap allocation failed"))?;
            self.slots.push(Slot {
                generation: 0,
                marked: false,
                object: None,
                bytes: 0,
            });
            self.slots.len() - 1
        };
        self.slots[i].bytes = bytes;
        self.bytes = total;
        self.slots[i].object = Some(object);
        self.live += 1;
        Ok(GcRef {
            store: self.store,
            index: i,
            generation: self.slots[i].generation,
        })
    }
    pub fn root(&self, r: GcRef) -> Result<RootToken> {
        self.index(r)?;
        let mut roots = self
            .roots
            .lock()
            .map_err(|_| err(ErrorKind::Host, "poisoned root registry"))?;
        let key = roots.next;
        roots.next = key
            .checked_add(1)
            .ok_or_else(|| err(ErrorKind::ResourceLimit, "root identity exhaustion"))?;
        roots.values.insert(key, r);
        Ok(RootToken {
            registration: Arc::new(Registration {
                roots: Arc::downgrade(&self.roots),
                key,
                value: r,
            }),
        })
    }
    pub(crate) fn registered_roots(&self) -> Result<Vec<GcRef>> {
        let roots = self
            .roots
            .lock()
            .map_err(|_| err(ErrorKind::Host, "root registry poisoned"))?;
        if roots.values.len() > self.mark_limit {
            return Err(err(ErrorKind::ResourceLimit, "GC root work limit"));
        }
        let mut values = Vec::new();
        values
            .try_reserve(roots.values.len())
            .map_err(|_| err(ErrorKind::ResourceLimit, "GC root allocation failed"))?;
        values.extend(roots.values.values().copied());
        Ok(values)
    }
    fn enqueue(&mut self, r: GcRef, pending: &mut Vec<GcRef>, remaining: &mut usize) -> Result<()> {
        if *remaining == 0 {
            return Err(err(ErrorKind::ResourceLimit, "GC mark work limit"));
        }
        *remaining -= 1;
        let i = self.index(r)?;
        if !self.slots[i].marked {
            pending
                .try_reserve(1)
                .map_err(|_| err(ErrorKind::ResourceLimit, "GC mark allocation failed"))?;
            self.slots[i].marked = true;
            pending.push(r);
        }
        Ok(())
    }
    pub fn collect(&mut self, roots: impl IntoIterator<Item = GcRef>) -> Result<CollectionStats> {
        // Failure changes only temporary mark bits, never liveness or identities.
        // Mark on enqueue so duplicate edges cannot inflate the worklist.
        let registered = self.registered_roots()?;
        let mut pending = Vec::new();
        for slot in &mut self.slots {
            slot.marked = false;
        }
        let mut remaining = self.mark_limit;
        for r in roots.into_iter().chain(registered) {
            self.enqueue(r, &mut pending, &mut remaining)?;
        }
        while let Some(r) = pending.pop() {
            let i = self.index(r)?;
            let field_count = self.slots[i].object.as_ref().unwrap().fields.len();
            for j in 0..field_count {
                if remaining == 0 {
                    return Err(err(ErrorKind::ResourceLimit, "GC mark work limit"));
                }
                remaining -= 1;
                if let Some(child) = reference(&self.slots[i].object.as_ref().unwrap().fields[j]) {
                    self.enqueue(child, &mut pending, &mut remaining)?;
                }
            }
            if let Some(child) = self.slots[i].object.as_ref().unwrap().descriptor {
                self.enqueue(child, &mut pending, &mut remaining)?;
            }
        }
        let mut reclaimed = 0;
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if slot.object.is_some() && !slot.marked {
                self.bytes -= slot.bytes;
                slot.bytes = 0;
                slot.object = None;
                reclaimed += 1;
                if let Some(next) = slot.generation.checked_add(1) {
                    slot.generation = next;
                    self.free.push(i);
                }
            }
        }
        self.live -= reclaimed;
        Ok(CollectionStats {
            live: self.live,
            reclaimed,
        })
    }
}
pub(crate) fn reference(value: &Value) -> Option<GcRef> {
    match value {
        Value::Ref(RefValue::Gc(r) | RefValue::ExternGc(r)) => Some(*r),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn object() -> Object {
        Object {
            ty: CanonicalTypeId::test(1, 0),
            kind: ObjectKind::Struct,
            fields: vec![],
            descriptor: None,
        }
    }
    #[test]
    fn t112_cycles_and_stale_references() {
        let mut a = GcArena::new(1);
        let x = a.allocate(object(), 10).unwrap();
        let y = a.allocate(object(), 10).unwrap();
        a.get_mut(x)
            .unwrap()
            .fields
            .push(Value::Ref(RefValue::Gc(y)));
        a.get_mut(y)
            .unwrap()
            .fields
            .push(Value::Ref(RefValue::Gc(x)));
        assert_eq!(a.collect([x]).unwrap().live, 2);
        assert_eq!(a.collect([]).unwrap().reclaimed, 2);
        assert!(a.get(x).is_err());
        let z = a.allocate(object(), 10).unwrap();
        assert_ne!(z, y);
        assert!(a.get(y).is_err());
    }
    #[test]
    fn t113_host_token_clones_and_descriptor_edges() {
        let mut a = GcArena::new(1);
        let x = a.allocate(object(), 10).unwrap();
        let mut o = object();
        o.descriptor = Some(x);
        let y = a.allocate(o, 10).unwrap();
        let root = a.root(y).unwrap();
        let clone = root.clone();
        drop(root);
        assert_eq!(a.collect([]).unwrap().live, 2);
        drop(clone);
        assert_eq!(a.collect([]).unwrap().live, 0);
    }
    #[test]
    fn t114_foreign_refs_fail_and_budgets_preserve_heap() {
        let mut a = GcArena::new(1);
        let b = GcArena::new(2);
        let x = a.allocate(object(), 1).unwrap();
        assert!(a.allocate(object(), 1).is_err());
        assert_eq!(a.live(), 1);
        assert!(b.get(x).is_err());
        assert_eq!(a.collect([x]).unwrap().live, 1);
    }
    #[test]
    fn t114_externalized_gc_values_trace_the_same_object() {
        let mut arena = GcArena::new(1);
        let child = arena.allocate(object(), 10).unwrap();
        let mut parent = object();
        parent.fields.push(Value::Ref(RefValue::ExternGc(child)));
        let root = arena.allocate(parent, 10).unwrap();
        assert_eq!(arena.collect([root]).unwrap().live, 2);
        assert!(arena.get(child).is_ok());
        assert_eq!(arena.collect([]).unwrap().reclaimed, 2);
    }
    #[test]
    fn t114_deep_heap_chain_marks_and_reclaims_iteratively() {
        let mut arena = GcArena::new(1);
        let mut tail = None;
        for _ in 0..50_000 {
            let mut next = object();
            if let Some(r) = tail {
                next.fields.push(Value::Ref(RefValue::Gc(r)))
            }
            tail = Some(arena.allocate(next, 50_000).unwrap());
        }
        assert_eq!(arena.collect(tail).unwrap().live, 50_000);
        assert_eq!(arena.collect([]).unwrap().reclaimed, 50_000);
        assert_eq!(arena.live(), 0);
    }
    #[test]
    fn t112_payload_byte_limit_is_transactional_and_reclaimed() {
        let mut arena = GcArena::new(1);
        let mut payload = object();
        payload.fields = vec![Value::I32(7); 16];
        let expected = std::mem::size_of::<Object>() + 16 * std::mem::size_of::<Value>();
        arena.set_limits(expected, 100);
        let r = arena.allocate(payload.clone(), 10).unwrap();
        assert_eq!(arena.bytes(), expected);
        assert_eq!(
            arena.allocate(payload, 10).unwrap_err().kind,
            ErrorKind::ResourceLimit
        );
        assert_eq!(arena.live(), 1);
        assert_eq!(arena.collect([r]).unwrap().live, 1);
        assert_eq!(arena.collect([]).unwrap().reclaimed, 1);
        assert_eq!(arena.bytes(), 0);
    }
    #[test]
    fn t114_mark_budget_failure_never_sweeps_and_identity_is_stable() {
        let mut arena = GcArena::new(1);
        let child = arena.allocate(object(), 10).unwrap();
        let mut parent = object();
        parent.fields = vec![Value::Ref(RefValue::Gc(child)); 32];
        let r = arena.allocate(parent, 10).unwrap();
        arena.set_limits(usize::MAX, 2);
        assert_eq!(
            arena.collect([r]).unwrap_err().kind,
            ErrorKind::ResourceLimit
        );
        assert_eq!(arena.live(), 2);
        assert!(arena.get(child).is_ok());
        arena.set_limits(usize::MAX, 100);
        for _ in 0..10 {
            assert_eq!(arena.collect([r]).unwrap().live, 2);
            assert_eq!(
                arena.get(r).unwrap().fields[0],
                Value::Ref(RefValue::Gc(child))
            );
        }
        assert_eq!(arena.collect([]).unwrap().reclaimed, 2);
    }
    #[test]
    fn t154_shared_meta_descriptor_chain_and_cycle_trace() {
        let mut arena = GcArena::new(1);
        let meta = arena.allocate(object(), 10).unwrap();
        let mut descriptor = object();
        descriptor.descriptor = Some(meta);
        let descriptor = arena.allocate(descriptor, 10).unwrap();
        let mut instance = object();
        instance.descriptor = Some(descriptor);
        let a = arena.allocate(instance.clone(), 10).unwrap();
        let b = arena.allocate(instance, 10).unwrap();
        arena
            .get_mut(meta)
            .unwrap()
            .fields
            .push(Value::Ref(RefValue::Gc(a)));
        assert_eq!(arena.collect([a, b]).unwrap().live, 4);
        assert_eq!(
            arena.get(a).unwrap().descriptor,
            arena.get(b).unwrap().descriptor
        );
        assert_eq!(arena.collect([b]).unwrap().live, 4);
        assert_eq!(arena.collect([]).unwrap().reclaimed, 4);
    }
}
