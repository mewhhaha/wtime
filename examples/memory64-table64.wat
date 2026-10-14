;; Original example: i64 addresses and table indices with bounded allocations.
(module
  (memory i64 1 1)
  (table i64 1 1 funcref)
  (func (export "memory_roundtrip") (result i32)
    i64.const 65535 i32.const 42 i32.store8
    i64.const 65535 i32.load8_u)
  (func (export "table_size") (result i64)
    table.size))
