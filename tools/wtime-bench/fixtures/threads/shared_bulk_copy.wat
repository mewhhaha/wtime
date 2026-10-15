;; P078 shared bulk-copy workload. The host seeds both full memories before each group.
(module
  (import "env" "src" (memory $src 4 4 shared))
  (import "env" "dst" (memory $dst 4 4 shared))
  (func (export "run")
    (param $dst_offset i32)
    (param $src_offset i32)
    (param $length i32)
    (param $repetitions i32)
    (result i32)
    (local $iteration i32)
    (block $done
      (loop $copy
        (br_if $done (i32.ge_u (local.get $iteration) (local.get $repetitions)))
        (memory.copy $dst $src
          (local.get $dst_offset)
          (local.get $src_offset)
          (local.get $length))
        (local.set $iteration (i32.add (local.get $iteration) (i32.const 1)))
        (br $copy)))
    (local.get $iteration)))
