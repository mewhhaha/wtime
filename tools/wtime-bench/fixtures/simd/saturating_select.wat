(module
  (func (export "run") (param $n i32) (result i32)
    (local $i i32) (local $v v128)
    (local.set $v (v128.const i8x16 120 -120 10 -10 127 -128 1 -1 3 4 -5 -6 50 -50 0 0))
    (block $done
      (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (local.set $v
          (v128.bitselect
            (i8x16.add_sat_s (local.get $v)
              (v128.const i8x16 20 -20 -20 20 1 -1 2 -2 8 9 -10 -11 90 -90 127 -128))
            (local.get $v)
            (v128.const i8x16 -1 0 -1 0 -1 0 -1 0 -1 0 -1 0 -1 0 -1 0)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)))
    (i32.add
      (i32.add
        (i32.add (i8x16.extract_lane_s 0 (local.get $v)) (i8x16.extract_lane_s 1 (local.get $v)))
        (i32.add (i8x16.extract_lane_s 2 (local.get $v)) (i8x16.extract_lane_s 3 (local.get $v))))
      (i32.add
        (i32.add (i8x16.extract_lane_s 4 (local.get $v)) (i8x16.extract_lane_s 5 (local.get $v)))
        (i32.add
          (i32.add (i8x16.extract_lane_s 6 (local.get $v)) (i8x16.extract_lane_s 7 (local.get $v)))
          (i32.add
            (i32.add (i8x16.extract_lane_s 8 (local.get $v)) (i8x16.extract_lane_s 9 (local.get $v)))
            (i32.add
              (i32.add (i8x16.extract_lane_s 10 (local.get $v)) (i8x16.extract_lane_s 11 (local.get $v)))
              (i32.add
                (i32.add (i8x16.extract_lane_s 12 (local.get $v)) (i8x16.extract_lane_s 13 (local.get $v)))
                (i32.add (i8x16.extract_lane_s 14 (local.get $v)) (i8x16.extract_lane_s 15 (local.get $v)))))))))))
