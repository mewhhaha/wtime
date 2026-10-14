(module
  (func (export "run") (param $n i32) (result i32)
    (local $i i32) (local $acc i32)
    (block $done
      (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (block $next_case
          (block $case0
            (block $case1
              (block $case2
                (block $case3
                  (block $default
                    (br_table $case0 $case1 $case2 $case3 $default
                      (i32.and (local.get $i) (i32.const 3))))
                  (local.set $acc (i32.add (local.get $acc) (i32.const 19)))
                  (br $next_case))
                (local.set $acc (i32.add (local.get $acc) (i32.const 13)))
                (br $next_case))
              (local.set $acc (i32.add (local.get $acc) (i32.const 7)))
              (br $next_case))
            (local.set $acc (i32.add (local.get $acc) (i32.const 3)))
            (br $next_case))
          (local.set $acc (i32.add (local.get $acc) (i32.const 1))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)))
    (local.get $acc)))
