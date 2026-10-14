;; SPDX-License-Identifier: CC0-1.0
;; Authored specifically for P037; static metadata only, never executed.
(module
  (func (export "edges")
    block
      loop
        i32.const 0
        br_if 1
        i32.const 0
        if
          nop
        else
          nop
        end
        i32.const 0
        br_table 0 1 2 1
      end
    end
    return))
