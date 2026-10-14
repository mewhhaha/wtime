//! Module-local indices; these are never runtime object handles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TypeIdx(pub u32);
impl TypeIdx {
    pub const fn as_u32(self) -> u32 {
        self.0
    }
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FuncIdx(pub u32);
impl FuncIdx {
    pub const fn as_u32(self) -> u32 {
        self.0
    }
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MemoryIdx(pub u32);
impl MemoryIdx {
    pub const fn as_u32(self) -> u32 {
        self.0
    }
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TableIdx(pub u32);
impl TableIdx {
    pub const fn as_u32(self) -> u32 {
        self.0
    }
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GlobalIdx(pub u32);
impl GlobalIdx {
    pub const fn as_u32(self) -> u32 {
        self.0
    }
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TagIdx(pub u32);
impl TagIdx {
    pub const fn as_u32(self) -> u32 {
        self.0
    }
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DataIdx(pub u32);
impl DataIdx {
    pub const fn as_u32(self) -> u32 {
        self.0
    }
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ElemIdx(pub u32);
impl ElemIdx {
    pub const fn as_u32(self) -> u32 {
        self.0
    }
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalIdx(pub u32);
impl LocalIdx {
    pub const fn as_u32(self) -> u32 {
        self.0
    }
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LabelIdx(pub u32);
impl LabelIdx {
    pub const fn as_u32(self) -> u32 {
        self.0
    }
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FieldIdx(pub u32);
impl FieldIdx {
    pub const fn as_u32(self) -> u32 {
        self.0
    }
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
