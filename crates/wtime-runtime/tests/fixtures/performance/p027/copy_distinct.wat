;; Original benchmark fixture; SPDX-License-Identifier: CC0-1.0.
(module
  (memory $source (export "source") 16 16)
  (memory $destination (export "memory") 17 17)
  ;; Optional deterministic full-memory setup; call outside every timer.
  ;; The Rust host driver may instead initialize both entire images through
  ;; write_memory outside the timer to avoid millions of guest setup stores.
  (func (export "prepare")
    (local $i i32)
    (block $done
      (loop $init
        (br_if $done (i32.ge_u (local.get $i) (i32.const 1048576)))
        (i32.store8 $source (local.get $i)
          (i32.add (i32.mul (local.get $i) (i32.const 29)) (i32.const 7)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $init)))
    (local.set $i (i32.const 0))
    (block $destination_done
      (loop $destination_init
        (br_if $destination_done (i32.ge_u (local.get $i) (i32.const 1114112)))
        (i32.store8 $destination (local.get $i)
          (i32.add (i32.mul (local.get $i) (i32.const 13)) (i32.const 101)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $destination_init))))
  ;; Source and destination are distinct memories. Source stays unchanged,
  ;; making the repeated copy idempotent while changing destination bytes.
  (func (export "run") (param $bytes i32) (param $repetitions i32)
    (local $i i32)
    (block $done
      (loop $repeat
        (br_if $done (i32.ge_u (local.get $i) (local.get $repetitions)))
        (memory.copy $destination $source
          (i32.const 65536) (i32.const 0) (local.get $bytes))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $repeat)))))
