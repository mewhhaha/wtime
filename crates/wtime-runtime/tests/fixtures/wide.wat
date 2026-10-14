(module
 (func (export "add") (param i64 i64 i64 i64) (result i64 i64)
  local.get 0 local.get 1 local.get 2 local.get 3 i64.add128)
 (func (export "sub") (param i64 i64 i64 i64) (result i64 i64)
  local.get 0 local.get 1 local.get 2 local.get 3 i64.sub128)
 (func (export "mul-s") (param i64 i64) (result i64 i64)
  local.get 0 local.get 1 i64.mul_wide_s)
 (func (export "mul-u") (param i64 i64) (result i64 i64)
  local.get 0 local.get 1 i64.mul_wide_u))
