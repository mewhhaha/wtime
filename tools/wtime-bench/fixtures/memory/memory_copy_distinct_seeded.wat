;; SPDX-License-Identifier: CC0-1.0.
;; Host driver initializes both complete memories outside all invocation timers.
(module
  (memory $source (export "source") 16 16)
  (memory $destination (export "memory") 17 17)
  (func (export "run") (param $bytes i32) (result i32)
    (memory.copy $destination $source
      (i32.const 65536) (i32.const 0) (local.get $bytes))
    (local.get $bytes)))
