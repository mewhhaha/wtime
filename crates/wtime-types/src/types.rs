use crate::TypeIdx;

/// Half-open byte range `[start, end)` within a WebAssembly binary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Span {
    /// Inclusive start byte offset.
    pub start: usize,
    /// Exclusive end byte offset.
    pub end: usize,
}
impl Span {
    /// Creates a span from inclusive `start` and exclusive `end` byte offsets.
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
    /// Returns the length of the span in bytes, saturating at zero.
    pub const fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }
    /// Returns `true` if the span contains no bytes (`start >= end`).
    pub const fn is_empty(self) -> bool {
        self.start >= self.end
    }
}
/// Address width used to index a linear memory or table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum IndexType {
    /// 32-bit addressing (`i32`).
    #[default]
    I32,
    /// 64-bit addressing (`i64`, from `memory64`).
    I64,
}
impl IndexType {
    /// Returns the corresponding WebAssembly integer [`ValType`].
    pub const fn val_type(self) -> ValType {
        match self {
            Self::I32 => ValType::I32,
            Self::I64 => ValType::I64,
        }
    }
}
/// Minimum and optional maximum size bounds for a memory or table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Limits {
    /// Minimum required size (in pages for memories or elements for tables).
    pub min: u64,
    /// Optional maximum size bound.
    pub max: Option<u64>,
}
impl Limits {
    /// Creates size limits with the given `min` and optional `max`.
    pub const fn new(min: u64, max: Option<u64>) -> Self {
        Self { min, max }
    }
    /// Returns `true` if `max`, when present, is greater than or equal to `min`.
    pub fn is_valid(self) -> bool {
        self.max.is_none_or(|max| self.min <= max)
    }
}
/// Descriptor for a WebAssembly linear memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MemoryType {
    /// Page-count bounds for the memory.
    pub limits: Limits,
    /// Addressing index width (`i32` or `i64`).
    pub index_type: IndexType,
    /// Whether the memory is shared across threads.
    pub shared: bool,
    /// Base-2 logarithm of the memory's page size in bytes (16 for standard 64 KiB pages).
    pub page_size_log2: u8,
}
impl MemoryType {
    /// The most pages the memory's index type can address: 2^32 bytes for a
    /// 32-bit memory, 2^64 for a 64-bit one, whatever the declared maximum.
    pub fn addressable_pages(&self) -> u64 {
        let bits: u32 = if self.index_type == IndexType::I64 {
            64
        } else {
            32
        };
        1u64.checked_shl(bits - u32::from(self.page_size_log2))
            .unwrap_or(u64::MAX)
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
/// Descriptor for a WebAssembly table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TableType {
    /// Element-count bounds for the table.
    pub limits: Limits,
    /// Addressing index width (`i32` or `i64`).
    pub index_type: IndexType,
    /// Reference type stored in each table slot.
    pub element: RefType,
}
/// Descriptor for a WebAssembly global variable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlobalType {
    /// Value type stored by the global.
    pub value: ValType,
    /// Whether the global may be mutated after initialization.
    pub mutable: bool,
}
/// WebAssembly value type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ValType {
    /// 32-bit integer (`i32`).
    I32,
    /// 64-bit integer (`i64`).
    I64,
    /// 32-bit IEEE 754 floating-point number (`f32`).
    F32,
    /// 64-bit IEEE 754 floating-point number (`f64`).
    F64,
    /// 128-bit SIMD vector (`v128`).
    V128,
    /// Reference type with nullability and heap type.
    Ref(RefType),
}
impl ValType {
    /// Nullable function reference type (`funcref`).
    pub const FUNCREF: Self = Self::Ref(RefType::FUNCREF);
    /// Nullable external host reference type (`externref`).
    pub const EXTERNREF: Self = Self::Ref(RefType::EXTERNREF);
    /// Returns `true` for scalar numeric types (`I32`, `I64`, `F32`, `F64`).
    pub const fn is_numeric(self) -> bool {
        matches!(self, Self::I32 | Self::I64 | Self::F32 | Self::F64)
    }
}
/// WebAssembly reference type combining nullability and a [`HeapType`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RefType {
    /// Whether `null` is a valid inhabitant of this reference type.
    pub nullable: bool,
    /// Target heap type hierarchy.
    pub heap: HeapType,
}
impl RefType {
    /// Nullable abstract function reference (`funcref`).
    pub const FUNCREF: Self = Self {
        nullable: true,
        heap: HeapType::Func,
    };
    /// Nullable abstract external reference (`externref`).
    pub const EXTERNREF: Self = Self {
        nullable: true,
        heap: HeapType::Extern,
    };
    /// Constructs a reference type with the given nullability and heap type.
    pub const fn new(nullable: bool, heap: HeapType) -> Self {
        Self { nullable, heap }
    }
}
/// WebAssembly abstract or module-defined heap type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HeapType {
    /// Abstract function heap type (`func`).
    Func,
    /// Abstract external host heap type (`extern`).
    Extern,
    /// Top internal GC heap type (`any`).
    Any,
    /// Comparable internal GC heap type (`eq`).
    Eq,
    /// Unboxed 31-bit integer heap type (`i31`).
    I31,
    /// Abstract GC struct heap type (`struct`).
    Struct,
    /// Abstract GC array heap type (`array`).
    Array,
    /// Bottom internal GC heap type (`none`).
    None,
    /// Bottom function heap type (`nofunc`).
    NoFunc,
    /// Bottom external heap type (`noextern`).
    NoExtern,
    /// Abstract exception heap type (`exn`).
    Exn,
    /// Bottom exception heap type (`noexn`).
    NoExn,
    /// Abstract continuation heap type (`cont`).
    Cont,
    /// Bottom continuation heap type (`nocont`).
    NoCont,
    /// Module-defined concrete type index, allowing subtypes.
    Defined(TypeIdx),
    /// Module-defined concrete type index matching only that exact type (no proper subtypes).
    ExactDefined(TypeIdx),
}
/// Function signature consisting of parameter and result value types.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct FuncType {
    /// Parameter types in declaration order.
    pub params: Vec<ValType>,
    /// Result types in declaration order.
    pub results: Vec<ValType>,
}
impl FuncType {
    /// Creates a new function signature from parameter and result type vectors.
    pub fn new(params: Vec<ValType>, results: Vec<ValType>) -> Self {
        Self { params, results }
    }
}
/// Storage representation of a GC struct field or array element.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StorageType {
    /// Packed 8-bit integer storage (`i8`).
    I8,
    /// Packed 16-bit integer storage (`i16`).
    I16,
    /// Unpacked WebAssembly value type.
    Val(ValType),
}
impl StorageType {
    /// Returns the operand-stack [`ValType`] produced when unpacking this storage type.
    pub const fn unpacked(self) -> ValType {
        match self {
            Self::I8 | Self::I16 => ValType::I32,
            Self::Val(v) => v,
        }
    }
}
/// Storage type and mutability of a GC struct field or array element.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FieldType {
    /// Underlying packed or unpacked storage type.
    pub storage: StorageType,
    /// Whether the field or element may be mutated after allocation.
    pub mutable: bool,
}
/// Definition of a GC struct composite type.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct StructType {
    /// Ordered field definitions.
    pub fields: Vec<FieldType>,
}
/// Definition of a GC array composite type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ArrayType {
    /// Element storage type and mutability.
    pub field: FieldType,
}
/// Definition of a stack-switching continuation composite type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ContType {
    /// Index of the underlying function type executed by the continuation.
    pub function: TypeIdx,
}
/// Structural body of a module-defined type.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CompositeType {
    /// Function signature type.
    Func(FuncType),
    /// GC struct type.
    Struct(StructType),
    /// GC array type.
    Array(ArrayType),
    /// Stack-switching continuation type.
    Cont(ContType),
}
/// Subtype declaration inside a [`TypeGroup`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SubType {
    /// Whether further subtyping of this type is disallowed (`sub final`).
    pub final_: bool,
    /// Optional declared immediate supertype.
    pub supertype: Option<TypeIdx>,
    /// Underlying composite type definition.
    pub composite: CompositeType,
    /// Optional custom descriptor type associated with instances of this type.
    pub descriptor: Option<TypeIdx>,
    /// Optional target type described by instances of this descriptor type.
    pub describes: Option<TypeIdx>,
}
impl SubType {
    /// Creates a final subtype with no supertype or custom-descriptor clauses.
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
/// Recursion group of mutually-referencing [`SubType`] definitions.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct TypeGroup {
    /// Whether the group was encoded with an explicit `rec` prefix (`0x4e`).
    pub explicit: bool,
    /// Member subtypes in declaration order.
    pub types: Vec<SubType>,
}
/// Descriptor for an exception or continuation tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TagType {
    /// Function type index giving the tag's payload and (for stack switching) result types.
    pub function: TypeIdx,
}

/// Vectors preserve their binary little-endian lane representation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct V128(pub [u8; 16]);
impl V128 {
    /// Constructs a `v128` from a `u128` encoded in little-endian byte order.
    pub const fn from_u128(value: u128) -> Self {
        Self(value.to_le_bytes())
    }
    /// Converts the little-endian lane bytes into a `u128`.
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
