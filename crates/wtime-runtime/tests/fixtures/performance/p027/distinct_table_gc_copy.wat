;; SPDX-License-Identifier: CC0-1.0
;; Original author: OpenAI Codex, 2026-10-11.
;; P027 semantic fixture: copy typed GC references across distinct tables and
;; across the 4096-element chunk boundary, then clear the complete source table.
(module
  (rec (type $node (struct (field i32))))
  (table $source 4097 (ref null $node))
  (table $destination 4097 (ref null $node))

  (func (export "seed")
    (table.set $source (i32.const 0)
      (struct.new $node (i32.const 73)))
    (table.set $source (i32.const 4096)
      (struct.new $node (i32.const 91))))

  (func (export "copy_all")
    (table.copy $destination $source
      (i32.const 0) (i32.const 0) (i32.const 4097)))

  (func (export "clear_source")
    (table.fill $source (i32.const 0) (ref.null $node) (i32.const 4097)))

  (func (export "clear_destination")
    (table.fill $destination (i32.const 0) (ref.null $node) (i32.const 4097)))

  (func (export "read_destination") (param $index i32) (result i32)
    (struct.get $node 0 (table.get $destination (local.get $index))))
)
