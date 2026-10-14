//! Engine-bound canonical identities. Whole recursion groups are equality keys.
use super::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CanonicalTypeId {
    engine: u64,
    index: u32,
}
impl CanonicalTypeId {
    #[cfg(feature = "compact-executor")]
    pub(crate) fn engine_id(self) -> u64 {
        self.engine
    }
    #[cfg(test)]
    pub(crate) fn test(engine: u64, index: u32) -> Self {
        Self { engine, index }
    }
}
struct Entry {
    #[cfg(test)]
    definition: SubType,
    supertype: Option<CanonicalTypeId>,
}
// Exercise real registry equality under adversarial collisions in unit tests.
#[cfg(test)]
#[derive(Default)]
struct CollisionHasher;
#[cfg(test)]
impl std::hash::Hasher for CollisionHasher {
    fn finish(&self) -> u64 {
        0
    }
    fn write(&mut self, _: &[u8]) {}
}
#[cfg(test)]
type GroupMap =
    HashMap<Vec<SubType>, Vec<CanonicalTypeId>, std::hash::BuildHasherDefault<CollisionHasher>>;
#[cfg(not(test))]
type GroupMap = HashMap<Vec<SubType>, Vec<CanonicalTypeId>>;
pub(crate) struct CanonicalTypes {
    engine: u64,
    groups: GroupMap,
    entries: Vec<Entry>,
    limit: usize,
    bytes: usize,
    byte_limit: usize,
}
impl CanonicalTypes {
    pub fn new(engine: u64) -> Self {
        Self {
            engine,
            groups: GroupMap::default(),
            entries: vec![],
            limit: ResourceLimits::default().max_canonical_types,
            bytes: 0,
            byte_limit: ResourceLimits::default().max_canonical_type_bytes,
        }
    }
    pub fn set_limit(&mut self, limit: usize) {
        self.limit = limit;
    }
    pub fn set_byte_limit(&mut self, limit: usize) {
        self.byte_limit = limit;
    }
    pub fn intern_group(
        &mut self,
        group: &TypeGroup,
        preceding: &[CanonicalTypeId],
        budget: usize,
    ) -> Result<Vec<CanonicalTypeId>> {
        if group.types.is_empty() {
            return Ok(Vec::new());
        }
        for id in preceding {
            self.check(*id)?;
        }
        let base = preceding.len();
        let end = base
            .checked_add(group.types.len())
            .ok_or_else(|| err(ErrorKind::ResourceLimit, "type index overflow"))?;
        let node_cost = group
            .types
            .iter()
            .try_fold(group.types.len(), |cost, ty| {
                let fields = match &ty.composite {
                    CompositeType::Func(t) => t.params.len().checked_add(t.results.len()),
                    CompositeType::Struct(t) => Some(t.fields.len()),
                    _ => Some(1),
                };
                cost.checked_add(fields.unwrap_or(usize::MAX))
            })
            .ok_or_else(|| err(ErrorKind::ResourceLimit, "type work overflow"))?;
        let mut remaining = budget
            .checked_sub(node_cost)
            .ok_or_else(|| err(ErrorKind::ResourceLimit, "canonical type work limit"))?;
        let mut normalized = group.types.clone();
        for ty in &mut normalized {
            visit(ty, &mut |idx| {
                if remaining == 0 {
                    return Err(err(ErrorKind::ResourceLimit, "canonical type work limit"));
                }
                remaining -= 1;
                let n = idx.0 as usize;
                if n < base {
                    Ok(TypeIdx(preceding[n].index))
                } else if n < end {
                    let local = u32::try_from(n - base)
                        .map_err(|_| err(ErrorKind::ResourceLimit, "recursion group too large"))?;
                    Ok(TypeIdx(0x80000000 | local))
                } else {
                    Err(err(
                        ErrorKind::Validation,
                        "type reference outside recursion group",
                    ))
                }
            })?;
        }
        if let Some(ids) = self.groups.get(&normalized) {
            return Ok(ids.clone());
        }
        let retained = normalized.iter().try_fold(
            normalized
                .len()
                .checked_mul(
                    std::mem::size_of::<SubType>()
                        + std::mem::size_of::<Entry>()
                        + std::mem::size_of::<CanonicalTypeId>(),
                )
                .ok_or_else(|| err(ErrorKind::ResourceLimit, "canonical size overflow"))?,
            |sum, ty| {
                let payload = match &ty.composite {
                    CompositeType::Func(f) => f
                        .params
                        .capacity()
                        .checked_add(f.results.capacity())
                        .and_then(|n| n.checked_mul(std::mem::size_of::<ValType>())),
                    CompositeType::Struct(t) => t
                        .fields
                        .capacity()
                        .checked_mul(std::mem::size_of::<FieldType>()),
                    _ => Some(0),
                };
                sum.checked_add(payload.unwrap_or(usize::MAX))
                    .ok_or_else(|| err(ErrorKind::ResourceLimit, "canonical size overflow"))
            },
        )?;
        let total_bytes = self
            .bytes
            .checked_add(retained)
            .filter(|n| *n <= self.byte_limit)
            .ok_or_else(|| err(ErrorKind::ResourceLimit, "canonical registry byte limit"))?;
        let first = self.entries.len();
        if first
            .checked_add(group.types.len())
            .is_none_or(|n| n >= 0x80000000 || n > self.limit)
        {
            return Err(err(
                ErrorKind::ResourceLimit,
                "canonical type identity limit",
            ));
        }
        let ids: Vec<_> = (0..group.types.len())
            .map(|i| CanonicalTypeId {
                engine: self.engine,
                index: (first + i) as u32,
            })
            .collect();
        let resolve = |idx: TypeIdx| -> CanonicalTypeId {
            if idx.0 & 0x80000000 != 0 {
                ids[(idx.0 & 0x7fffffff) as usize]
            } else {
                CanonicalTypeId {
                    engine: self.engine,
                    index: idx.0,
                }
            }
        };
        let mut entries = Vec::with_capacity(normalized.len());
        for ty in &normalized {
            let supertype = ty.supertype.map(resolve);
            entries.push(Entry {
                #[cfg(test)]
                definition: ty.clone(),
                supertype,
            });
        }
        // A declared supertype chain must terminate, even if field references recurse.
        for (i, _) in entries.iter().enumerate() {
            let mut current = Some(ids[i]);
            let mut seen = std::collections::HashSet::new();
            while let Some(id) = current {
                if remaining == 0 {
                    return Err(err(
                        ErrorKind::ResourceLimit,
                        "canonical supertype work limit",
                    ));
                }
                remaining -= 1;
                if !seen.insert(id) {
                    return Err(err(ErrorKind::Validation, "cyclic declared supertypes"));
                }
                let entry = if id.index as usize >= first {
                    &entries[id.index as usize - first]
                } else {
                    &self.entries[id.index as usize]
                };
                current = entry.supertype;
            }
        }
        self.entries
            .try_reserve(entries.len())
            .map_err(|_| err(ErrorKind::ResourceLimit, "canonical registry allocation"))?;
        self.groups
            .try_reserve(1)
            .map_err(|_| err(ErrorKind::ResourceLimit, "canonical registry allocation"))?;
        self.bytes = total_bytes;
        self.entries.extend(entries);
        self.groups.insert(normalized, ids.clone());
        Ok(ids)
    }
    fn check(&self, id: CanonicalTypeId) -> Result<usize> {
        if id.engine != self.engine || id.index as usize >= self.entries.len() {
            Err(err(ErrorKind::Host, "foreign or unknown canonical type"))
        } else {
            Ok(id.index as usize)
        }
    }
    #[cfg(test)]
    pub fn equivalent(&self, a: CanonicalTypeId, b: CanonicalTypeId) -> Result<bool> {
        self.check(a)?;
        self.check(b)?;
        Ok(a == b)
    }
    pub fn is_subtype(
        &self,
        actual: CanonicalTypeId,
        expected: CanonicalTypeId,
        exact: bool,
        budget: usize,
    ) -> Result<bool> {
        self.check(actual)?;
        self.check(expected)?;
        if exact {
            return Ok(actual == expected);
        }
        let mut current = Some(actual);
        let mut remaining = budget;
        while let Some(id) = current {
            if id == expected {
                return Ok(true);
            }
            if remaining == 0 {
                return Err(err(ErrorKind::ResourceLimit, "subtype work limit"));
            }
            remaining -= 1;
            current = self.entries[self.check(id)?].supertype;
        }
        Ok(false)
    }
    #[cfg(test)]
    pub fn definition(&self, id: CanonicalTypeId) -> Result<&SubType> {
        Ok(&self.entries[self.check(id)?].definition)
    }
}
fn visit(ty: &mut SubType, f: &mut impl FnMut(TypeIdx) -> Result<TypeIdx>) -> Result<()> {
    for value in [&mut ty.supertype, &mut ty.descriptor, &mut ty.describes]
        .into_iter()
        .flatten()
    {
        *value = f(*value)?;
    }
    fn val(v: &mut ValType, f: &mut impl FnMut(TypeIdx) -> Result<TypeIdx>) -> Result<()> {
        if let ValType::Ref(r) = v {
            match &mut r.heap {
                HeapType::Defined(i) | HeapType::ExactDefined(i) => *i = f(*i)?,
                _ => {}
            }
        }
        Ok(())
    }
    fn field(v: &mut FieldType, f: &mut impl FnMut(TypeIdx) -> Result<TypeIdx>) -> Result<()> {
        if let StorageType::Val(v) = &mut v.storage {
            val(v, f)?
        }
        Ok(())
    }
    match &mut ty.composite {
        CompositeType::Func(t) => {
            for v in t.params.iter_mut().chain(&mut t.results) {
                val(v, f)?
            }
        }
        CompositeType::Struct(t) => {
            for v in &mut t.fields {
                field(v, f)?
            }
        }
        CompositeType::Array(t) => field(&mut t.field, f)?,
        CompositeType::Cont(t) => t.function = f(t.function)?,
    };
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn recursive(index: u32) -> SubType {
        SubType::final_type(CompositeType::Struct(StructType {
            fields: vec![FieldType {
                storage: StorageType::Val(ValType::Ref(RefType::new(
                    true,
                    HeapType::Defined(TypeIdx(index)),
                ))),
                mutable: false,
            }],
        }))
    }
    #[test]
    fn t108_recursive_groups_normalize_absolute_indices() {
        let mut c = CanonicalTypes::new(1);
        let a = c
            .intern_group(
                &TypeGroup {
                    explicit: true,
                    types: vec![recursive(0)],
                },
                &[],
                100,
            )
            .unwrap();
        let b = c
            .intern_group(
                &TypeGroup {
                    explicit: true,
                    types: vec![recursive(1)],
                },
                &a,
                100,
            )
            .unwrap();
        assert_eq!(a, b);
        assert!(c.equivalent(a[0], b[0]).unwrap());
    }
    #[test]
    fn t154_exact_type_excludes_proper_subtypes() {
        let mut c = CanonicalTypes::new(1);
        let mut parent = SubType::final_type(CompositeType::Func(FuncType::default()));
        parent.final_ = false;
        let a = c
            .intern_group(
                &TypeGroup {
                    explicit: false,
                    types: vec![parent],
                },
                &[],
                10,
            )
            .unwrap();
        let mut child = SubType::final_type(CompositeType::Func(FuncType::default()));
        child.supertype = Some(TypeIdx(0));
        let b = c
            .intern_group(
                &TypeGroup {
                    explicit: false,
                    types: vec![child],
                },
                &a,
                10,
            )
            .unwrap();
        assert!(c.is_subtype(b[0], a[0], false, 10).unwrap());
        assert!(!c.is_subtype(b[0], a[0], true, 10).unwrap());
    }
    #[test]
    fn t108_group_boundaries_and_work_budgets() {
        let mut c = CanonicalTypes::new(1);
        assert!(
            c.intern_group(
                &TypeGroup {
                    explicit: true,
                    types: vec![recursive(0)]
                },
                &[],
                0
            )
            .is_err()
        );
        let a = c
            .intern_group(
                &TypeGroup {
                    explicit: true,
                    types: vec![recursive(0)],
                },
                &[],
                10,
            )
            .unwrap();
        assert!(c.definition(a[0]).is_ok());
        let other = CanonicalTypes::new(2);
        assert!(other.equivalent(a[0], a[0]).is_err());
    }
    #[test]
    fn t108_empty_recursion_group_has_no_canonical_members() {
        let mut registry = CanonicalTypes::new(1);
        assert!(
            registry
                .intern_group(
                    &TypeGroup {
                        explicit: true,
                        types: vec![]
                    },
                    &[],
                    0
                )
                .unwrap()
                .is_empty()
        );
        assert_eq!(registry.entries.len(), 0);
    }
    #[test]
    fn t107_deep_declared_chain_uses_bounded_iteration() {
        let mut c = CanonicalTypes::new(77);
        let definition = SubType::final_type(CompositeType::Func(FuncType::default()));
        for index in 0..50_000u32 {
            c.entries.push(Entry {
                definition: definition.clone(),
                supertype: index
                    .checked_sub(1)
                    .map(|index| CanonicalTypeId { engine: 77, index }),
            });
        }
        let leaf = CanonicalTypeId {
            engine: 77,
            index: 49_999,
        };
        let root = CanonicalTypeId {
            engine: 77,
            index: 0,
        };
        assert_eq!(
            c.is_subtype(leaf, root, false, 20).unwrap_err().kind,
            ErrorKind::ResourceLimit
        );
        assert!(c.is_subtype(leaf, root, false, 50_000).unwrap());
        assert!(!c.is_subtype(leaf, root, true, 1).unwrap());
    }
    #[test]
    fn t108_collision_and_nonequivalent_member_preserve_identity() {
        let mut registry = CanonicalTypes::new(1);
        let first = TypeGroup {
            explicit: true,
            types: vec![recursive(0)],
        };
        let a = registry.intern_group(&first, &[], 100).unwrap();
        let mut changed = first.clone();
        let CompositeType::Struct(t) = &mut changed.types[0].composite else {
            panic!()
        };
        t.fields[0].mutable = true;
        let b = registry.intern_group(&changed, &[], 100).unwrap();
        assert_ne!(a, b, "all registry hashes collide in test builds");
        assert_eq!(registry.intern_group(&first, &[], 100).unwrap(), a);
    }
    #[test]
    fn t108_aggregate_registry_limits_are_transactional() {
        let mut registry = CanonicalTypes::new(1);
        registry.set_limit(1);
        let first = TypeGroup {
            explicit: true,
            types: vec![recursive(0)],
        };
        let a = registry.intern_group(&first, &[], 100).unwrap();
        let mut changed = first.clone();
        changed.types[0].final_ = false;
        assert_eq!(
            registry.intern_group(&changed, &[], 100).unwrap_err().kind,
            ErrorKind::ResourceLimit
        );
        assert_eq!(registry.entries.len(), 1);
        assert_eq!(registry.intern_group(&first, &[], 100).unwrap(), a);
        let mut empty = CanonicalTypes::new(2);
        empty.set_byte_limit(0);
        assert_eq!(
            empty.intern_group(&first, &[], 100).unwrap_err().kind,
            ErrorKind::ResourceLimit
        );
        assert!(empty.entries.is_empty());
    }
    #[test]
    fn t151_descriptor_edges_participate_in_recursive_identity() {
        let group = |base: u32, swapped: bool| {
            let mut members =
                vec![SubType::final_type(CompositeType::Struct(StructType::default())); 4];
            let (a, b) = if swapped { (3, 2) } else { (2, 3) };
            members[0].descriptor = Some(TypeIdx(base + a));
            members[1].descriptor = Some(TypeIdx(base + b));
            members[a as usize].describes = Some(TypeIdx(base));
            members[b as usize].describes = Some(TypeIdx(base + 1));
            TypeGroup {
                explicit: true,
                types: members,
            }
        };
        let mut registry = CanonicalTypes::new(1);
        let first = registry.intern_group(&group(0, false), &[], 100).unwrap();
        let renamed = registry
            .intern_group(&group(4, false), &first, 100)
            .unwrap();
        assert_eq!(first, renamed);
        let different = registry.intern_group(&group(4, true), &first, 100).unwrap();
        assert_ne!(
            first, different,
            "descriptor identity cannot be erased to field layout"
        );
        assert!(
            !registry
                .is_subtype(different[0], first[0], false, 100)
                .unwrap()
        );
    }
}
