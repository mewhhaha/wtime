;; SPDX-License-Identifier: Apache-2.0 OR MIT
(module
 (func (export "minimum_zero") (result i32) f32.const 0 f32.const -0 f32.min i32.reinterpret_f32)
 (func (export "maximum_zero") (result i64) f64.const -0 f64.const 0 f64.max i64.reinterpret_f64)
 (func (export "copy_nan_sign") (result i32) f32.const nan:0x412345 f32.const -0 f32.copysign i32.reinterpret_f32)
 (func (export "minimum_nan") (result i32) f32.const nan:0x412345 f32.const 1 f32.min i32.reinterpret_f32)
 (func (export "maximum_signaling_nan") (result i64) f64.const nan:0x1234 f64.const 2 f64.max i64.reinterpret_f64))
