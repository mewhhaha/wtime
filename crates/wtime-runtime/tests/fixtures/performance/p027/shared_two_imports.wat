;; SPDX-License-Identifier: CC0-1.0
;; Original author: OpenAI Codex, 2026-10-11.
;; P027 semantic fixture: two imports are expected to share one host backing.
;; The host initializes bytes before invoking these copy functions.
(module
  (import "mem" "left" (memory $left 2 2 shared))
  (import "mem" "right" (memory $right 2 2 shared))

  (func (export "copy_forward") (param $count i32)
    (memory.copy $right $left (i32.const 1) (i32.const 0)
      (local.get $count)))

  (func (export "copy_backward") (param $count i32)
    (memory.copy $right $left (i32.const 0) (i32.const 1)
      (local.get $count)))
)
