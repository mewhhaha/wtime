(module
  (func (export "run") (param $n i32) (result i32)
    (local $i i32) (local $v v128)
    (block $done
      (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (local.set $v
          (v128.xor (local.get $v)
            (i8x16.relaxed_swizzle
              (v128.const i8x16 0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15)
              (v128.const i8x16 0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)))
    (i32.add
      (i32.add (i8x16.extract_lane_u 0 (local.get $v)) (i8x16.extract_lane_u 1 (local.get $v)))
      (i32.add
        (i32.add (i8x16.extract_lane_u 2 (local.get $v)) (i8x16.extract_lane_u 3 (local.get $v)))
        (i32.add
          (i32.add (i8x16.extract_lane_u 4 (local.get $v)) (i8x16.extract_lane_u 5 (local.get $v)))
          (i32.add
            (i32.add (i8x16.extract_lane_u 6 (local.get $v)) (i8x16.extract_lane_u 7 (local.get $v)))
            (i32.add
              (i32.add (i8x16.extract_lane_u 8 (local.get $v)) (i8x16.extract_lane_u 9 (local.get $v)))
              (i32.add
                (i32.add (i8x16.extract_lane_u 10 (local.get $v)) (i8x16.extract_lane_u 11 (local.get $v)))
                (i32.add
                  (i32.add (i8x16.extract_lane_u 12 (local.get $v)) (i8x16.extract_lane_u 13 (local.get $v)))
                  (i32.add (i8x16.extract_lane_u 14 (local.get $v)) (i8x16.extract_lane_u 15 (local.get $v))))))))))))
