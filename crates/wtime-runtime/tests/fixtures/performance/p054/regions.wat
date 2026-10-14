(module
  ;; PC0: wrapping signed addition (i32.add)
  (func (export "0wrap") (result i32)
    i32.const 2147483647
    i32.const 1
    i32.add)

  ;; PC1: literal AND (i32.and)
  (func (export "1and") (result i32)
    i32.const 324508639
    i32.const -1
    i32.and)

  ;; PC2: literal OR (i32.or)
  (func (export "2or") (result i32)
    i32.const -2147483648
    i32.const 7
    i32.or)

  ;; PC3: literal XOR (i32.xor)
  (func (export "3xor") (result i32)
    i32.const -1
    i32.const 324508639
    i32.xor)

  ;; source event 4: local.get, local.set, local.get, implicit end
  (func (export "4copy") (param i32) (result i32)
    local.get 0
    local.set 0
    local.get 0)

  ;; Division by zero trap control.
  (func (export "5div") (result i32)
    i32.const 17
    i32.const 0
    i32.div_s)

  ;; Floating point negative control.
  (func (export "6float") (result f32)
    f32.const 1
    f32.const 2
    f32.add)
)
