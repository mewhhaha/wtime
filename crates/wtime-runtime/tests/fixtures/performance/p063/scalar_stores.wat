;; Isolated store-only projection of the original P043 scalar-widths module.
(module
  (memory (export "memory") 1 1)
  (data (i32.const 0) "\80\7f\01\80\02\03\04\80\01\80\80\01\02\80\01\02\03\04\05\06\07\08\00\00\80\3f\00\00\00\00\00\00\f0\3f")
  (func (export "store_kinds") (param $v32 i32) (param $v64 i64) (param $vf32 f32) (param $vf64 f64)
    i32.const 65
    local.get $v32
    i32.store8
    i32.const 67
    local.get $v32
    i32.store16
    i32.const 71
    local.get $v32
    i32.store
    i32.const 79
    local.get $v64
    i64.store8
    i32.const 81
    local.get $v64
    i64.store16
    i32.const 85
    local.get $v64
    i64.store32
    i32.const 91
    local.get $v64
    i64.store
    i32.const 103
    local.get $vf32
    f32.store
    i32.const 109
    local.get $vf64
    f64.store)
)
