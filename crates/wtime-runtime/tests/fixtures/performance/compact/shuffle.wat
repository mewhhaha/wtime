(module
  (func (export "run") (param $n i32) (result i32)
    (local $i i32) (local $v v128)
    (block $done
      (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (local.set $v
          (v128.xor (local.get $v)
            (i8x16.shuffle
              31 30 29 28 27 26 25 24 23 22 21 20 19 18 17 16
              (v128.const i8x16 0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15)
              (v128.const i8x16 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)))
    (local.set $v (i16x8.extadd_pairwise_i8x16_u (local.get $v)))
    (local.set $v (i32x4.extadd_pairwise_i16x8_u (local.get $v)))
    (i32.add
      (i32.add (i32x4.extract_lane 0 (local.get $v)) (i32x4.extract_lane 1 (local.get $v)))
      (i32.add (i32x4.extract_lane 2 (local.get $v)) (i32x4.extract_lane 3 (local.get $v))))))
