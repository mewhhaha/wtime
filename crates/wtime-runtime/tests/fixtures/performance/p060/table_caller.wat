;; SPDX-License-Identifier: MIT
;; P060 caller/callee control transfer fixture; callee control flow follows the P040 three-path table case.
(module
  (func $choose (param $which i32) (result i32)
    block (result i32)
      block (result i32)
        i32.const 7
        local.get $which
        br_table 0 1 2
      end
      i32.const 1000
      i32.add
    end
    i32.const 100
    i32.add
  )
  (func (export "run") (param $which i32) (result i32)
    i32.const 99
    i32.const 98
    local.get $which
    call $choose
    i32.add
    i32.add
  )
)
