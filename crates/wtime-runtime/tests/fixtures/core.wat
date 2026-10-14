(module
  (memory (export "memory") 1 2)
  (global (export "counter") (mut i32) (i32.const 0))
  (func (export "add") (param i32 i32) (result i32)
    local.get 0 local.get 1 i32.add)
  (func $fact (export "factorial") (param i32) (result i32)
    local.get 0 i32.eqz
    if (result i32) i32.const 1
    else local.get 0 local.get 0 i32.const 1 i32.sub call $fact i32.mul end)
  (func (export "sum") (param $n i32) (result i32) (local $sum i32)
    block $done loop $again
      local.get $n i32.eqz br_if $done
      local.get $sum local.get $n i32.add local.set $sum
      local.get $n i32.const 1 i32.sub local.set $n br $again
    end end local.get $sum)
  (func (export "store-load") (param i32 i32) (result i32)
    local.get 0 local.get 1 i32.store align=1
    local.get 0 i32.load align=1)
  (func (export "grow") (param i32) (result i32) local.get 0 memory.grow)
  (func (export "trunc") (param f64) (result i32) local.get 0 i32.trunc_f64_s)
  (func (export "unsigned-f64") (param i64) (result f64) local.get 0 f64.convert_i64_u)
  (func (export "nearest") (param f64) (result f64) local.get 0 f64.nearest)
  (func (export "min") (param f64 f64) (result f64) local.get 0 local.get 1 f64.min)
  (func (export "div") (param i32 i32) (result i32) local.get 0 local.get 1 i32.div_s)
  (func (export "count") (result i32)
    global.get 0 i32.const 1 i32.add global.set 0 global.get 0)
)
