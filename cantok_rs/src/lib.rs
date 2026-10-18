//! 仿照 C# `CancellationTokenSource` 的取消源与取消令牌。
//!
//! 本 crate 提供两件东西：
//!
//! - [`CancellationSource`]：取消信号的**控制端**，可幂等地触发取消；
//! - [`CancellationToken`]：取消信号的**等待端**，实现
//!   [`abs_cancel::TrCancellationToken`](https://docs.rs/abs_cancel)。
//!
//! 令牌只是 cancellation source 内部状态的一个普通引用（`Copy` 的轻量句柄）。
//! 调用 [`CancellationSource::child_token`] 或
//! [`CancellationToken::child_token`] **不会**注册等待者、也**不会**分配等待槽；
//! 只有在 `cancellation()` 返回的 future 被首次 `poll` 时，才会在源那里登记一个
//! 等待位置。这是本 crate 刻意保持的「按需注册」特性，用来减少不必要的堆分配。
//!
//! # 生命周期
//!
//! 令牌借用 [`CancellationSource`]，因此所有令牌与等待 future 都必须比源活得短；
//! 反过来，只要还有令牌存在，源就不能被移动。需要把令牌送进 `'static` 任务时，
//! 应把源放进 `Arc`，并在任务内部从该 `Arc` 派生令牌（任务自己持有 `Arc`）。
//!
//! # 同步原语
//!
//! 本 crate 不使用 `std::sync::Mutex`：等待槽列表由 `abs_sync.rs` 提供的自旋锁
//! `atomic_sync::mutex::preemptive::SpinningMutexOwned` 保护，取消状态则是原子布尔。
//!
//! # Examples
//!
//! ```
//! use cantok_rs::CancellationSource;
//!
//! let source = CancellationSource::new();
//! let token = source.child_token();
//!
//! assert!(!token.is_cancelled());
//! assert!(source.cancel());
//! assert!(token.is_cancelled());
//! // 幂等：第二次取消不会再次触发。
//! assert!(!source.cancel());
//! ```

#![no_std]

extern crate alloc;

#[cfg(test)]
extern crate std;

mod cancellation;

pub use cancellation::{CancellationFuture, CancellationSource, CancellationToken};
