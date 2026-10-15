(module
  (import "env" "memory" (memory 1 1 shared))

  ;; This flag proves that every worker reached the guest immediately before
  ;; entering wait. It does not prove queue registration before the timer.
  (func (export "wait") (param $address i32) (param $timeout i64) (result i32)
    (i32.atomic.rmw.add (i32.const 4096) (i32.const 1))
    drop
    (memory.atomic.wait32 (local.get $address) (i32.const 0) (local.get $timeout)))

  ;; Notify one waiter at each address until the requested successful count is
  ;; reached. Failed zero-notify probes count as attempts too.
  (func (export "drain") (param $waiters i32) (param $addresses i32)
      (result i32 i32)
    (local $address_index i32)
    (local $notified_here i32)
    (local $total_notified i32)
    (local $attempts i32)
    (local $notify_result i32)
    (block $done
      (loop $address_loop
        (br_if $done (i32.ge_u (local.get $address_index) (local.get $addresses)))
        (local.set $notified_here (i32.const 0))
        (block $address_done
          (loop $notify_loop
            (br_if $address_done
              (i32.ge_u (local.get $notified_here)
                (i32.div_u (local.get $waiters) (local.get $addresses))))
            (br_if $done (i32.ge_u (local.get $attempts) (i32.const 1000000)))
            (local.set $attempts (i32.add (local.get $attempts) (i32.const 1)))
            (local.set $notify_result
              (memory.atomic.notify
                (i32.mul (local.get $address_index) (i32.const 4))
                (i32.const 1)))
            (local.set $notified_here
              (i32.add (local.get $notified_here) (local.get $notify_result)))
            (local.set $total_notified
              (i32.add (local.get $total_notified) (local.get $notify_result)))
            (br $notify_loop)))
        (local.set $address_index (i32.add (local.get $address_index) (i32.const 1)))
        (br $address_loop)))
    (if (i32.ne (local.get $total_notified) (local.get $waiters))
      (then (return (i32.const -1) (local.get $attempts))))
    (local.get $total_notified)
    (local.get $attempts))

  ;; Untimed cleanup wakes remaining waiters on an address.
  (func (export "notify") (param $address i32) (param $count i32) (result i32)
    (memory.atomic.notify (local.get $address) (local.get $count)))
)
