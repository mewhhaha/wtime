use super::*;
#[derive(Debug)]
pub(super) struct TypeEnvironment {
    declarations: Vec<SubType>,
    groups: Vec<(usize, usize, usize)>,
    work: std::cell::Cell<usize>,
    limit: usize,
    exhausted: std::cell::Cell<bool>,
}
impl TypeEnvironment {
    pub(super) fn new(groups: &[TypeGroup], limit: usize) -> Self {
        let mut declarations = Vec::new();
        let mut layouts = Vec::new();
        for g in groups {
            let start = declarations.len();
            for (pos, t) in g.types.iter().enumerate() {
                declarations.push(t.clone());
                layouts.push((start, g.types.len(), pos));
            }
        }
        Self {
            declarations,
            groups: layouts,
            work: std::cell::Cell::new(0),
            limit,
            exhausted: std::cell::Cell::new(false),
        }
    }
    pub(super) fn exhausted(&self) -> bool {
        self.exhausted.get()
    }
    /// Work spent so far.
    pub(super) fn work(&self) -> usize {
        self.work.get()
    }
    /// Forgets the work spent, to measure one body's work on its own.
    pub(super) fn reset_work(&self) {
        self.work.set(0);
        self.exhausted.set(false);
    }
    /// Spends `n` units at once, as `n` spends would.
    pub(super) fn charge(&self, n: usize) {
        let total = self.work.get().saturating_add(n);
        self.work.set(total);
        if total > self.limit {
            self.exhausted.set(true);
        }
    }
    fn spend(&self) -> bool {
        let n = self.work.get().saturating_add(1);
        self.work.set(n);
        if n > self.limit {
            self.exhausted.set(true);
            false
        } else {
            true
        }
    }
}
impl std::ops::Deref for TypeEnvironment {
    type Target = [SubType];
    fn deref(&self) -> &Self::Target {
        &self.declarations
    }
}

pub(super) fn value_subtype(a: ValType, b: ValType, types: &TypeEnvironment) -> bool {
    if !types.spend() {
        return false;
    }
    if a == b {
        return true;
    }
    let (ValType::Ref(a), ValType::Ref(b)) = (a, b) else {
        return false;
    };
    if a.nullable && !b.nullable {
        return false;
    }
    heap_subtype(a.heap, b.heap, types)
}
fn heap_subtype(a: HeapType, b: HeapType, types: &TypeEnvironment) -> bool {
    use HeapType::*;
    if a == b {
        return true;
    }
    match (a, b) {
        (ExactDefined(a), Defined(b)) if a == b => true,
        (ExactDefined(a), ExactDefined(b)) => equivalent(a, b, types),
        (Defined(a) | ExactDefined(a), Defined(b)) => {
            let mut n = Some(a);
            for _ in 0..types.len() {
                if !types.spend() {
                    return false;
                }
                let Some(i) = n else { break };
                if i == b || equivalent(i, b, types) {
                    return true;
                }
                n = types.get(i.0 as usize).and_then(|t| t.supertype);
            }
            false
        }
        (Defined(i) | ExactDefined(i), b) => {
            types.get(i.0 as usize).is_some_and(|t| match t.composite {
                CompositeType::Func(_) => b == Func,
                CompositeType::Struct(_) => matches!(b, Struct | Eq | Any),
                CompositeType::Array(_) => matches!(b, Array | Eq | Any),
                CompositeType::Cont(_) => b == Cont,
            })
        }
        (None, Defined(i) | ExactDefined(i)) => types.get(i.0 as usize).is_some_and(|t| {
            matches!(
                t.composite,
                CompositeType::Struct(_) | CompositeType::Array(_)
            )
        }),
        (NoFunc, Defined(i) | ExactDefined(i)) => types
            .get(i.0 as usize)
            .is_some_and(|t| matches!(t.composite, CompositeType::Func(_))),
        (NoCont, Defined(i) | ExactDefined(i)) => types
            .get(i.0 as usize)
            .is_some_and(|t| matches!(t.composite, CompositeType::Cont(_))),
        (None, Any | Eq | Struct | Array | I31)
        | (NoFunc, Func)
        | (NoExtern, Extern)
        | (NoExn, Exn)
        | (NoCont, Cont)
        | (I31 | Struct | Array, Eq | Any)
        | (Eq, Any) => true,
        _ => false,
    }
}
fn equivalent(a: TypeIdx, b: TypeIdx, ts: &TypeEnvironment) -> bool {
    let mut todo = vec![(a, b)];
    let mut seen = HashSet::new();
    let mut seen_groups = HashSet::new();
    let mut work = 0;
    while let Some((a, b)) = todo.pop() {
        if a == b || !seen.insert((a, b)) {
            continue;
        }
        work += 1;
        if !ts.spend() || work > ts.limit {
            return false;
        }
        let (Some(&(start_a, len_a, pos_a)), Some(&(start_b, len_b, pos_b))) =
            (ts.groups.get(a.0 as usize), ts.groups.get(b.0 as usize))
        else {
            return false;
        };
        if len_a != len_b || pos_a != pos_b {
            return false;
        }
        if seen_groups.insert((start_a, start_b)) {
            for k in 0..len_a {
                todo.push((TypeIdx((start_a + k) as u32), TypeIdx((start_b + k) as u32)));
            }
        }
        let (Some(a), Some(b)) = (ts.get(a.0 as usize), ts.get(b.0 as usize)) else {
            return false;
        };
        if a.final_ != b.final_ {
            return false;
        }
        match (a.supertype, b.supertype) {
            (Some(a), Some(b)) => {
                if !equal_index(a, b, start_a, start_b, len_a, &mut todo) {
                    return false;
                }
            }
            (None, None) => {}
            _ => return false,
        }
        for (a, b) in [(a.descriptor, b.descriptor), (a.describes, b.describes)] {
            match (a, b) {
                (Some(a), Some(b)) => {
                    if !equal_index(a, b, start_a, start_b, len_a, &mut todo) {
                        return false;
                    }
                }
                (None, None) => {}
                _ => return false,
            }
        }
        let mut vals = vec![];
        match (&a.composite, &b.composite) {
            (CompositeType::Func(a), CompositeType::Func(b)) => {
                if a.params.len() != b.params.len() || a.results.len() != b.results.len() {
                    return false;
                }
                vals.extend(a.params.iter().zip(&b.params));
                vals.extend(a.results.iter().zip(&b.results));
            }
            (CompositeType::Struct(a), CompositeType::Struct(b)) => {
                if a.fields.len() != b.fields.len() {
                    return false;
                }
                for (a, b) in a.fields.iter().zip(&b.fields) {
                    if a.mutable != b.mutable {
                        return false;
                    }
                    match (a.storage, b.storage) {
                        (StorageType::Val(a), StorageType::Val(b)) => {
                            if !equal_value(a, b, start_a, start_b, len_a, &mut todo) {
                                return false;
                            }
                        }
                        (a, b) if a == b => {}
                        _ => return false,
                    }
                }
            }
            (CompositeType::Array(a), CompositeType::Array(b)) => {
                if a.field.mutable != b.field.mutable {
                    return false;
                }
                match (a.field.storage, b.field.storage) {
                    (StorageType::Val(a), StorageType::Val(b)) => {
                        if !equal_value(a, b, start_a, start_b, len_a, &mut todo) {
                            return false;
                        }
                    }
                    (a, b) if a == b => {}
                    _ => return false,
                }
            }
            (CompositeType::Cont(a), CompositeType::Cont(b)) => {
                if !equal_index(a.function, b.function, start_a, start_b, len_a, &mut todo) {
                    return false;
                }
            }
            _ => return false,
        }
        for (a, b) in vals {
            if !equal_value(*a, *b, start_a, start_b, len_a, &mut todo) {
                return false;
            }
        }
    }
    true
}
fn equal_index(
    a: TypeIdx,
    b: TypeIdx,
    start_a: usize,
    start_b: usize,
    len: usize,
    todo: &mut Vec<(TypeIdx, TypeIdx)>,
) -> bool {
    let a_internal = (start_a..start_a + len).contains(&(a.0 as usize));
    let b_internal = (start_b..start_b + len).contains(&(b.0 as usize));
    match (a_internal, b_internal) {
        (true, true) => (a.0 as usize - start_a) == (b.0 as usize - start_b),
        (false, false) => {
            if a != b {
                todo.push((a, b));
            }
            true
        }
        _ => false,
    }
}
fn equal_value(
    a: ValType,
    b: ValType,
    start_a: usize,
    start_b: usize,
    len: usize,
    todo: &mut Vec<(TypeIdx, TypeIdx)>,
) -> bool {
    match (a, b) {
        (ValType::Ref(a), ValType::Ref(b)) if a.nullable == b.nullable => match (a.heap, b.heap) {
            (HeapType::Defined(a), HeapType::Defined(b))
            | (HeapType::ExactDefined(a), HeapType::ExactDefined(b)) => {
                equal_index(a, b, start_a, start_b, len, todo)
            }
            _ => a.heap == b.heap,
        },
        _ => a == b,
    }
}
pub(super) fn validate_types(c: &Context<'_>) -> Result<()> {
    let ts = &c.types;
    let mut count = 0usize;
    let mut group_end = 0usize;
    for group in &c.m.type_groups {
        group_end += group.types.len();
        c.enabled(!group.explicit || c.features.gc)?;
        for t in &group.types {
            let mut vals = Vec::new();
            match &t.composite {
                CompositeType::Func(f) => vals.extend(f.params.iter().chain(&f.results).copied()),
                CompositeType::Struct(s) => {
                    vals.extend(s.fields.iter().map(|f| f.storage.unpacked()))
                }
                CompositeType::Array(a) => vals.push(a.field.storage.unpacked()),
                CompositeType::Cont(t) => check(
                    (t.function.0 as usize) < group_end,
                    "continuation type forward reference",
                )?,
            };
            for v in vals {
                if let ValType::Ref(RefType {
                    heap: HeapType::Defined(n) | HeapType::ExactDefined(n),
                    ..
                }) = v
                {
                    check(
                        (n.0 as usize) < group_end,
                        "type reference beyond recursion group",
                    )?;
                }
            }
        }
    }

    for (n, t) in ts.iter().enumerate() {
        for i in [t.descriptor, t.describes].into_iter().flatten() {
            check(
                (i.0 as usize) < ts.len(),
                "descriptor type index out of bounds",
            )?;
        }
        count = count.saturating_add(1);
        if let Some(p) = t.supertype {
            c.enabled(c.features.gc || c.features.function_references)?;
            check((p.0 as usize) < n, "supertype must precede subtype")?;
            let parent = &ts[p.0 as usize];
            check(
                parent.descriptor.is_some() == t.descriptor.is_some()
                    && parent.describes.is_some() == t.describes.is_some(),
                "descriptor subtype square missing edge",
            )?;
            if let (Some(a), Some(b)) = (t.describes, parent.describes) {
                check(
                    ts[a.0 as usize].supertype == Some(b),
                    "describes subtype square mismatch",
                )?;
            }
            check(!parent.final_, "cannot extend final type")?;
            match (&t.composite, &parent.composite) {
                (CompositeType::Func(a), CompositeType::Func(b)) => {
                    check(
                        a.params.len() == b.params.len() && a.results.len() == b.results.len(),
                        "function subtype arity",
                    )?;
                    for (a, b) in a.params.iter().zip(&b.params) {
                        check(value_subtype(*b, *a, ts), "function parameter variance")?
                    }
                    for (a, b) in a.results.iter().zip(&b.results) {
                        check(value_subtype(*a, *b, ts), "function result variance")?
                    }
                }
                (CompositeType::Struct(a), CompositeType::Struct(b)) => {
                    check(
                        a.fields.len() >= b.fields.len(),
                        "struct subtype field count",
                    )?;
                    for (a, b) in a.fields.iter().zip(&b.fields) {
                        field_subtype(*a, *b, ts)?
                    }
                }
                (CompositeType::Array(a), CompositeType::Array(b)) => {
                    field_subtype(a.field, b.field, ts)?
                }
                (CompositeType::Cont(a), CompositeType::Cont(b)) => check(
                    heap_subtype(
                        HeapType::Defined(a.function),
                        HeapType::Defined(b.function),
                        ts,
                    ),
                    "continuation subtype mismatch",
                )?,
                _ => return Err(invalid("supertype kind mismatch")),
            }
        }
        match &t.composite {
            CompositeType::Func(f) => {
                for v in f.params.iter().chain(&f.results) {
                    c.valtype(*v)?;
                    count = count.saturating_add(1)
                }
            }
            CompositeType::Struct(s) => {
                c.enabled(c.features.gc)?;
                for f in &s.fields {
                    c.valtype(f.storage.unpacked())?;
                    count = count.saturating_add(1)
                }
            }
            CompositeType::Array(a) => {
                c.enabled(c.features.gc)?;
                c.valtype(a.field.storage.unpacked())?
            }
            CompositeType::Cont(t) => {
                c.enabled(c.features.stack_switching)?;
                c.ty(t.function.0 as usize)?;
            }
        }
        if let Some(d) = t.descriptor {
            c.enabled(c.features.custom_descriptors)?;
            let other = ts
                .get(d.0 as usize)
                .ok_or_else(|| invalid("descriptor index out of bounds"))?;
            check(
                other.describes == Some(TypeIdx(n as u32)),
                "descriptor reciprocity mismatch",
            )?;
            check(
                matches!(t.composite, CompositeType::Struct(_))
                    && matches!(other.composite, CompositeType::Struct(_)),
                "descriptor must connect struct types",
            )?;
            check(t.final_ == other.final_, "descriptor finality mismatch")?;
            if let Some(p) = t.supertype {
                let pd = ts[p.0 as usize]
                    .descriptor
                    .ok_or_else(|| invalid("missing parent descriptor"))?;
                check(
                    other.supertype == Some(pd),
                    "descriptor subtype square mismatch",
                )?
            }
        }
        if let Some(d) = t.describes {
            c.enabled(c.features.custom_descriptors)?;
            check((d.0 as usize) < n, "describes must refer to preceding type")?;
            check(
                ts[d.0 as usize].descriptor == Some(TypeIdx(n as u32)),
                "describes reciprocity mismatch",
            )?
        }
        budget(
            count <= c.limits.max_type_work,
            "type validation work limit",
        )?;
    }
    Ok(())
}
fn field_subtype(a: FieldType, b: FieldType, ts: &TypeEnvironment) -> Result<()> {
    check(a.mutable == b.mutable, "field mutability mismatch")?;
    match (a.storage, b.storage) {
        (StorageType::Val(a), StorageType::Val(b)) => {
            check(value_subtype(a, b, ts), "field type mismatch")?
        }
        (a, b) => check(a == b, "packed storage mismatch")?,
    };
    if a.mutable
        && let (StorageType::Val(a), StorageType::Val(b)) = (a.storage, b.storage)
    {
        check(value_subtype(b, a, ts), "mutable field must be invariant")?
    }
    Ok(())
}

#[cfg(test)]
mod premise_tests {
    use super::*;

    #[test]
    fn t107_reference_nullability_and_bottom_lattice_matrix() {
        use HeapType::*;
        let heaps = [
            Any, Eq, I31, Struct, Array, None, Func, NoFunc, Extern, NoExtern, Exn, NoExn, Cont,
            NoCont,
        ];
        let edges = [
            (None, I31),
            (None, Struct),
            (None, Array),
            (I31, Eq),
            (Struct, Eq),
            (Array, Eq),
            (Eq, Any),
            (NoFunc, Func),
            (NoExtern, Extern),
            (NoExn, Exn),
            (NoCont, Cont),
        ];
        let env = TypeEnvironment::new(&[], 100_000);
        for a in heaps {
            for b in heaps {
                let mut reachable = HashSet::from([a]);
                loop {
                    let before = reachable.len();
                    for (from, to) in edges {
                        if reachable.contains(&from) {
                            reachable.insert(to);
                        }
                    }
                    if before == reachable.len() {
                        break;
                    }
                }
                for nullable_a in [false, true] {
                    for nullable_b in [false, true] {
                        let expected = reachable.contains(&b) && (!nullable_a || nullable_b);
                        assert_eq!(
                            value_subtype(
                                ValType::Ref(RefType::new(nullable_a, a)),
                                ValType::Ref(RefType::new(nullable_b, b)),
                                &env
                            ),
                            expected,
                            "{a:?}/{nullable_a} -> {b:?}/{nullable_b}"
                        );
                    }
                }
            }
        }
        let scalars = [
            ValType::I32,
            ValType::I64,
            ValType::F32,
            ValType::F64,
            ValType::V128,
        ];
        for a in scalars {
            for b in scalars {
                assert_eq!(value_subtype(a, b, &env), a == b);
            }
        }
        assert!(!env.exhausted());
    }

    #[test]
    fn t107_array_storage_covariance_and_mutable_invariance() {
        let env = TypeEnvironment::new(&[], 1000);
        let field = |heap, mutable| FieldType {
            storage: StorageType::Val(ValType::Ref(RefType::new(true, heap))),
            mutable,
        };
        assert!(
            field_subtype(
                field(HeapType::Eq, false),
                field(HeapType::Any, false),
                &env
            )
            .is_ok()
        );
        assert!(
            field_subtype(
                field(HeapType::Any, false),
                field(HeapType::Eq, false),
                &env
            )
            .is_err()
        );
        assert!(field_subtype(field(HeapType::Eq, true), field(HeapType::Eq, true), &env).is_ok());
        assert!(
            field_subtype(field(HeapType::Eq, true), field(HeapType::Any, true), &env).is_err()
        );
        assert!(
            field_subtype(field(HeapType::Any, true), field(HeapType::Eq, true), &env).is_err()
        );
        assert!(
            field_subtype(field(HeapType::Eq, true), field(HeapType::Any, false), &env).is_err()
        );
        let packed = |storage| FieldType {
            storage,
            mutable: false,
        };
        assert!(field_subtype(packed(StorageType::I8), packed(StorageType::I16), &env).is_err());
        assert!(
            field_subtype(
                packed(StorageType::I8),
                packed(StorageType::Val(ValType::I32)),
                &env
            )
            .is_err()
        );
    }
}
