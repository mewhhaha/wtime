(module
  (memory $bytes (export "bytes") 4 16 (pagesize 1))
  (memory $wide (export "wide") i64 1 2)
  (func (export "byte-size") (result i32) memory.size $bytes)
  (func (export "byte-grow") (param i32) (result i32) local.get 0 memory.grow $bytes)
  (func (export "wide-size") (result i64) memory.size $wide)
  (func (export "wide-grow") (param i64) (result i64) local.get 0 memory.grow $wide)
  (func (export "write-read") (param i64 i64) (result i64)
    local.get 0 local.get 1 i64.store $wide align=1
    local.get 0 i64.load $wide align=1)
  (func (export "copy") (param i32 i64 i32)
    local.get 0 local.get 1 local.get 2 memory.copy $bytes $wide)
)
