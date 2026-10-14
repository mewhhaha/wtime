(module
  (memory (export "memory") 1 2)
  (data (i32.const 0) "\00\01\02\03\04\05\06\07\08\09\0a\0b\0c\0d\0e\0f\80\ff\7f\00\00\00\00\80\01\02\03\04\05\06\07\08\45\23\c1\7f\00\00\00\00\cd\ab\89\67\45\23\f9\7f\30\31\32\33\34\35\36\37\38\39\3a\3b\3c\3d\3e\3f")
  (func (export "load_i32") (param $base i32) (result i32)
    local.get $base
    i32.const 24
    i32.add
    i32.load)
  (func (export "load_i32_8_s") (param $base i32) (result i32)
    local.get $base
    i32.const 16
    i32.add
    i32.load8_s)
  (func (export "load_i32_8_u") (param $base i32) (result i32)
    local.get $base
    i32.const 16
    i32.add
    i32.load8_u)
  (func (export "load_i32_16_s") (param $base i32) (result i32)
    local.get $base
    i32.const 16
    i32.add
    i32.load16_s)
  (func (export "load_i32_16_u") (param $base i32) (result i32)
    local.get $base
    i32.const 16
    i32.add
    i32.load16_u)
  (func (export "load_i64") (param $base i32) (result i64)
    local.get $base
    i32.const 24
    i32.add
    i64.load)
  (func (export "load_i64_8_s") (param $base i32) (result i64)
    local.get $base
    i32.const 16
    i32.add
    i64.load8_s)
  (func (export "load_i64_8_u") (param $base i32) (result i64)
    local.get $base
    i32.const 16
    i32.add
    i64.load8_u)
  (func (export "load_i64_16_s") (param $base i32) (result i64)
    local.get $base
    i32.const 16
    i32.add
    i64.load16_s)
  (func (export "load_i64_16_u") (param $base i32) (result i64)
    local.get $base
    i32.const 16
    i32.add
    i64.load16_u)
  (func (export "load_i64_32_s") (param $base i32) (result i64)
    local.get $base
    i32.const 20
    i32.add
    i64.load32_s)
  (func (export "load_i64_32_u") (param $base i32) (result i64)
    local.get $base
    i32.const 20
    i32.add
    i64.load32_u)
  (func (export "load_f32") (param $base i32) (result f32)
    local.get $base
    i32.const 32
    i32.add
    f32.load)
  (func (export "load_f64") (param $base i32) (result f64)
    local.get $base
    i32.const 40
    i32.add
    f64.load)
  (func (export "store_i32") (param $base i32)
    local.get $base
    i32.const 4
    i32.add
    i32.const 287454020
    i32.store)
  (func (export "store_i32_8") (param $base i32)
    local.get $base
    i32.const 4
    i32.add
    i32.const 287454020
    i32.store8)
  (func (export "store_i32_16") (param $base i32)
    local.get $base
    i32.const 4
    i32.add
    i32.const 287454020
    i32.store16)
  (func (export "store_i64") (param $base i32)
    local.get $base
    i32.const 4
    i32.add
    i64.const 1234605616436508552
    i64.store)
  (func (export "store_i64_8") (param $base i32)
    local.get $base
    i32.const 4
    i32.add
    i64.const 1234605616436508552
    i64.store8)
  (func (export "store_i64_16") (param $base i32)
    local.get $base
    i32.const 4
    i32.add
    i64.const 1234605616436508552
    i64.store16)
  (func (export "store_i64_32") (param $base i32)
    local.get $base
    i32.const 4
    i32.add
    i64.const 1234605616436508552
    i64.store32)
  (func (export "store_f32_raw") (param $base i32)
    local.get $base
    i32.const 4
    i32.add
    f32.const nan:0x412345
    f32.store)
  (func (export "store_f64_raw") (param $base i32)
    local.get $base
    i32.const 4
    i32.add
    f64.const nan:0x923456789abcd
    f64.store)
  (func (export "load_i32_offset2") (param $base i32) (result i32)
    local.get $base
    i32.const 4
    i32.add
    i32.load offset=2)
  (func (export "load_wrapped_address") (param $base i32) (result i32)
    local.get $base
    i32.const 1
    i32.add
    i32.load)
  (func (export "load_oob_near_page_end") (param $base i32) (result i32)
    local.get $base
    i32.const 65533
    i32.add
    i32.load)
  (func (export "load_oob_max_memarg_offset") (param $base i32) (result i32)
    local.get $base
    i32.const 1
    i32.add
    i32.load offset=4294967295)
  (func (export "store_then_oob_load") (param $base i32) (result i32)
    local.get $base
    i32.const 0
    i32.add
    i32.const 42
    i32.store
    local.get $base
    i32.const 65533
    i32.add
    i32.load)
  (func (export "grow_store_then_load") (param $base i32) (result i32)
    i32.const 1
    memory.grow
    drop
    i32.const 65536
    i32.const 287454020
    i32.store
    local.get $base
    i32.const 4
    i32.add
    i32.load)
)
