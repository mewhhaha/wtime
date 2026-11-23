//! Module-local indices; these are never runtime object handles.
/// Module-local index into the type index space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TypeIdx(pub u32);
impl TypeIdx {
    /// Returns the raw `u32` index value.
    pub const fn as_u32(self) -> u32 {
        self.0
    }
    /// Returns the index value widened to `usize`.
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}
impl From<u32> for TypeIdx {
    fn from(value: u32) -> Self {
        Self(value)
    }
}
impl From<TypeIdx> for u32 {
    fn from(value: TypeIdx) -> Self {
        value.0
    }
}
/// Module-local index into the function index space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FuncIdx(pub u32);
impl FuncIdx {
    /// Returns the raw `u32` index value.
    pub const fn as_u32(self) -> u32 {
        self.0
    }
    /// Returns the index value widened to `usize`.
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}
impl From<u32> for FuncIdx {
    fn from(value: u32) -> Self {
        Self(value)
    }
}
impl From<FuncIdx> for u32 {
    fn from(value: FuncIdx) -> Self {
        value.0
    }
}
/// Module-local index into the linear memory index space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MemoryIdx(pub u32);
impl MemoryIdx {
    /// Returns the raw `u32` index value.
    pub const fn as_u32(self) -> u32 {
        self.0
    }
    /// Returns the index value widened to `usize`.
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}
impl From<u32> for MemoryIdx {
    fn from(value: u32) -> Self {
        Self(value)
    }
}
impl From<MemoryIdx> for u32 {
    fn from(value: MemoryIdx) -> Self {
        value.0
    }
}
/// Module-local index into the table index space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TableIdx(pub u32);
impl TableIdx {
    /// Returns the raw `u32` index value.
    pub const fn as_u32(self) -> u32 {
        self.0
    }
    /// Returns the index value widened to `usize`.
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}
impl From<u32> for TableIdx {
    fn from(value: u32) -> Self {
        Self(value)
    }
}
impl From<TableIdx> for u32 {
    fn from(value: TableIdx) -> Self {
        value.0
    }
}
/// Module-local index into the global index space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GlobalIdx(pub u32);
impl GlobalIdx {
    /// Returns the raw `u32` index value.
    pub const fn as_u32(self) -> u32 {
        self.0
    }
    /// Returns the index value widened to `usize`.
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}
impl From<u32> for GlobalIdx {
    fn from(value: u32) -> Self {
        Self(value)
    }
}
impl From<GlobalIdx> for u32 {
    fn from(value: GlobalIdx) -> Self {
        value.0
    }
}
/// Module-local index into the tag index space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TagIdx(pub u32);
impl TagIdx {
    /// Returns the raw `u32` index value.
    pub const fn as_u32(self) -> u32 {
        self.0
    }
    /// Returns the index value widened to `usize`.
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}
impl From<u32> for TagIdx {
    fn from(value: u32) -> Self {
        Self(value)
    }
}
impl From<TagIdx> for u32 {
    fn from(value: TagIdx) -> Self {
        value.0
    }
}
/// Module-local index into the data segment index space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DataIdx(pub u32);
impl DataIdx {
    /// Returns the raw `u32` index value.
    pub const fn as_u32(self) -> u32 {
        self.0
    }
    /// Returns the index value widened to `usize`.
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}
impl From<u32> for DataIdx {
    fn from(value: u32) -> Self {
        Self(value)
    }
}
impl From<DataIdx> for u32 {
    fn from(value: DataIdx) -> Self {
        value.0
    }
}
/// Module-local index into the element segment index space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ElemIdx(pub u32);
impl ElemIdx {
    /// Returns the raw `u32` index value.
    pub const fn as_u32(self) -> u32 {
        self.0
    }
    /// Returns the index value widened to `usize`.
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}
impl From<u32> for ElemIdx {
    fn from(value: u32) -> Self {
        Self(value)
    }
}
impl From<ElemIdx> for u32 {
    fn from(value: ElemIdx) -> Self {
        value.0
    }
}
/// Function-local index into a function's parameters and local variables.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalIdx(pub u32);
impl LocalIdx {
    /// Returns the raw `u32` index value.
    pub const fn as_u32(self) -> u32 {
        self.0
    }
    /// Returns the index value widened to `usize`.
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}
impl From<u32> for LocalIdx {
    fn from(value: u32) -> Self {
        Self(value)
    }
}
impl From<LocalIdx> for u32 {
    fn from(value: LocalIdx) -> Self {
        value.0
    }
}
/// Relative control-block branch label depth within a function body.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LabelIdx(pub u32);
impl LabelIdx {
    /// Returns the raw `u32` index value.
    pub const fn as_u32(self) -> u32 {
        self.0
    }
    /// Returns the index value widened to `usize`.
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}
impl From<u32> for LabelIdx {
    fn from(value: u32) -> Self {
        Self(value)
    }
}
impl From<LabelIdx> for u32 {
    fn from(value: LabelIdx) -> Self {
        value.0
    }
}
/// Zero-based index of a field within a GC struct type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FieldIdx(pub u32);
impl FieldIdx {
    /// Returns the raw `u32` index value.
    pub const fn as_u32(self) -> u32 {
        self.0
    }
    /// Returns the index value widened to `usize`.
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}
impl From<u32> for FieldIdx {
    fn from(value: u32) -> Self {
        Self(value)
    }
}
impl From<FieldIdx> for u32 {
    fn from(value: FieldIdx) -> Self {
        value.0
    }
}
