;; T008/T161: execute the five assignments absent from the pinned full-suite trace.
(module
  (func (export "signed32") (result v128)
    (i32x4.relaxed_trunc_f32x4_s (v128.const f32x4 1 -2 3 -4)))
  (func (export "unsigned32") (result v128)
    (i32x4.relaxed_trunc_f32x4_u (v128.const f32x4 1 2 3 4)))
  (func (export "signed64") (result v128)
    (i32x4.relaxed_trunc_f64x2_s_zero (v128.const f64x2 1 -2)))
  (func (export "unsigned64") (result v128)
    (i32x4.relaxed_trunc_f64x2_u_zero (v128.const f64x2 1 2))))
(assert_return (invoke "signed32") (v128.const i32x4 1 -2 3 -4))
(assert_return (invoke "unsigned32") (v128.const i32x4 1 2 3 4))
(assert_return (invoke "signed64") (v128.const i32x4 1 -2 0 0))
(assert_return (invoke "unsigned64") (v128.const i32x4 1 2 0 0))
(module
  (rec
    (type $object (descriptor $descriptor) (struct (field i32)))
    (type $descriptor (describes $object) (struct)))
  (func (export "default") (result i32)
    (struct.get $object 0
      (struct.new_default_desc $object (struct.new $descriptor)))))
(assert_return (invoke "default") (i32.const 0))
