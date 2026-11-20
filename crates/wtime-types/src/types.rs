use crate::TypeIdx;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}
impl Span {
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
    pub const fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }
    pub const fn is_empty(self) -> bool {
        self.start >= self.end
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum IndexType {
    #[default]
    I32,
    I64,
}
impl IndexType {
    pub const fn val_type(self) -> ValType {
        match self {
            Self::I32 => ValType::I32,
            Self::I64 => ValType::I64,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Limits {
    pub min: u64,
    pub max: Option<u64>,
}
impl Limits {
    pub const fn new(min: u64, max: Option<u64>) -> Self {
        Self { min, max }
    }
    pub fn is_valid(self) -> bool {
        self.max.is_none_or(|max| self.min <= max)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MemoryType {
    pub limits: Limits,
    pub index_type: IndexType,
    pub shared: bool,
    pub page_size_log2: u8,
}
impl MemoryType {
    /// The most pages the memory's index type can address: 2^32 bytes for a
    /// 32-bit memory, 2^64 for a 64-bit one, whatever the declared maximum.
    pub fn addressable_pages(&self) -> u64 {
        let bits: u32 = if self.index_type == IndexType::I64 { 64 } else { 32 };
        1u64.checked_shl(bits - u32::from(self.page_size_log2)).unwrap_or(u64::MAX)
    }
}
impl Default for MemoryType {
    fn default() -> Self {
        Self {
            limits: Limits::default(),
            index_type: IndexType::I32,
            shared: false,
            page_size_log2: 16,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TableType {
    pub limits: Limits,
    pub index_type: IndexType,
    pub element: RefType,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlobalType {
    pub value: ValType,
    pub mutable: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ValType {
    I32,
    I64,
    F32,
    F64,
    V128,
    Ref(RefType),
}
impl ValType {
    pub const FUNCREF: Self = Self::Ref(RefType::FUNCREF);
    pub const EXTERNREF: Self = Self::Ref(RefType::EXTERNREF);
    pub const fn is_numeric(self) -> bool {
        matches!(self, Self::I32 | Self::I64 | Self::F32 | Self::F64)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RefType {
    pub nullable: bool,
    pub heap: HeapType,
}
impl RefType {
    pub const FUNCREF: Self = Self {
        nullable: true,
        heap: HeapType::Func,
    };
    pub const EXTERNREF: Self = Self {
        nullable: true,
        heap: HeapType::Extern,
    };
    pub const fn new(nullable: bool, heap: HeapType) -> Self {
        Self { nullable, heap }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HeapType {
    Func,
    Extern,
    Any,
    Eq,
    I31,
    Struct,
    Array,
    None,
    NoFunc,
    NoExtern,
    Exn,
    NoExn,
    Cont,
    NoCont,
    Defined(TypeIdx),
    ExactDefined(TypeIdx),
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct FuncType {
    pub params: Vec<ValType>,
    pub results: Vec<ValType>,
}
impl FuncType {
    pub fn new(params: Vec<ValType>, results: Vec<ValType>) -> Self {
        Self { params, results }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StorageType {
    I8,
    I16,
    Val(ValType),
}
impl StorageType {
    pub const fn unpacked(self) -> ValType {
        match self {
            Self::I8 | Self::I16 => ValType::I32,
            Self::Val(v) => v,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FieldType {
    pub storage: StorageType,
    pub mutable: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct StructType {
    pub fields: Vec<FieldType>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ArrayType {
    pub field: FieldType,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ContType {
    pub function: TypeIdx,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CompositeType {
    Func(FuncType),
    Struct(StructType),
    Array(ArrayType),
    Cont(ContType),
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SubType {
    pub final_: bool,
    pub supertype: Option<TypeIdx>,
    pub composite: CompositeType,
    pub descriptor: Option<TypeIdx>,
    pub describes: Option<TypeIdx>,
}
impl SubType {
    pub fn final_type(composite: CompositeType) -> Self {
        Self {
            final_: true,
            supertype: None,
            composite,
            descriptor: None,
            describes: None,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct TypeGroup {
    pub explicit: bool,
    pub types: Vec<SubType>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TagType {
    pub function: TypeIdx,
}

/// Vectors preserve their binary little-endian lane representation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct V128(pub [u8; 16]);
impl V128 {
    pub const fn from_u128(value: u128) -> Self {
        Self(value.to_le_bytes())
    }
    pub const fn to_u128(self) -> u128 {
        u128::from_le_bytes(self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn t006_case_01() {
        let memory = MemoryType {
            limits: Limits::new(3, Some(9)),
            index_type: IndexType::I64,
            shared: false,
            page_size_log2: 0,
        };
        let table = TableType {
            limits: Limits::new(1u64 << 32, Some(1u64 << 33)),
            index_type: IndexType::I64,
            element: RefType::new(true, HeapType::Func),
        };
        assert_eq!(memory.page_size_log2, 0);
        assert_eq!(table.limits.min, 1u64 << 32);
        assert_eq!(table.index_type, IndexType::I64);
    }
    #[test]
    fn t006_case_02() {
        let field = FieldType {
            storage: StorageType::Val(ValType::Ref(RefType::new(
                true,
                HeapType::Defined(TypeIdx(0)),
            ))),
            mutable: true,
        };
        let group = TypeGroup {
            explicit: true,
            types: vec![SubType::final_type(CompositeType::Struct(StructType {
                fields: vec![field],
            }))],
        };
        assert!(group.explicit);
        let CompositeType::Struct(s) = &group.types[0].composite else {
            panic!()
        };
        assert_eq!(s.fields[0], field);
    }
    #[test]
    fn t006_case_03() {
        // Exact references carry a source type index, never an abstract heap type.
        let exact = HeapType::ExactDefined(TypeIdx(7));
        assert_ne!(exact, HeapType::Func);
        assert_ne!(exact, HeapType::Defined(TypeIdx(7)));
    }
}
