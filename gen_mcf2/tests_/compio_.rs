//! 在 `compio` 运行时上运行与运行时无关的测试场景。
//!
//! 新增其它运行时（例如 `tokio`）时，只需再建一个同样 `mod common;` 的
//! 测试文件，把这里的 `#[compio::test]` 换成对应运行时的属性即可。
#![feature(impl_trait_in_assoc_type)]

mod common;

use common::*;

/// 测试放宽访问级别：私有函数生成的 `pub` 类型可以被别的模块命名与构造。
/// - 手段：在 crate 根模块里使用 `common::WidenedAsync::new(...)`。
/// - 判断：编译通过（若沿用原函数的私有访问级别则此处无法编译），且返回 `42`。
#[compio::test]
async fn widened_visibility_on_compio() {
    let mut n = 41usize;
    assert_eq!(common::WidenedAsync::new(&mut n).await, 42);
}

/// 测试「类型公开、构造函数仅 crate 内」的拆分访问级别能真实编译并运行。
/// - 手段：crate 根模块（`common` 之外）用 `common::SplitAsync::new(...)` 构造并 await。
/// - 判断：编译通过且返回 `40 + 2 = 42`。此处的调用点在 `common` 之外，因此若类型或
///   `new` 仍沿用原函数的私有级别，本用例都无法编译。
///
/// 注意：同一 crate 内 `pub` 与 `pub(crate)` 的可达性相同，所以「`new` 恰好是
/// `pub(crate)` 而不是 `pub`」这一条无法在本层证伪，由 `src/lib.rs` 的
/// `expands_with_split_visibility` 对展开文本做精确断言。
#[compio::test]
async fn split_visibility_on_compio() {
    let mut n = 40usize;
    assert_eq!(common::SplitAsync::new(&mut n).await, 42);
}

/// 测试不可取消路径：`DoThingAsync` 直接 `.await` 能得到原函数的返回值。
/// - 手段：调用 `scenario_into_future`，它构造 `DoThingAsync` 并 await。
/// - 判断：返回的 `Vec<u8>` 等于把原函数第一个参数（1u8）拷贝出来的结果。
#[compio::test]
async fn into_future_on_compio() {
    assert_eq!(scenario_into_future().await, vec![1u8]);
}

/// 测试可取消路径：`may_cancel_with(未取消令牌)` 后正常完成。
/// - 手段：调用 `scenario_may_cancel`，使用 `NonCancellableToken`。
/// - 判断：返回值等于原函数第一个参数（5u8）拷贝出来的结果。
#[compio::test]
async fn may_cancel_on_compio() {
    assert_eq!(scenario_may_cancel().await, vec![5u8]);
}

/// 测试令牌已取消时函数体立即观察到取消信号。
/// - 手段：先 `cancel()` 再 `may_cancel_with`。
/// - 判断：返回 `true`，且函数体没有继续执行。
#[compio::test]
async fn cancelled_token_on_compio() {
    assert!(scenario_cancelled().await);
}

/// 测试令牌未取消时函数体正常执行并累加计数。
/// - 手段：使用未取消的 `TestToken` 驱动 `ObserveCancelAsync`。
/// - 判断：返回 `(false, 1)`。
#[compio::test]
async fn not_cancelled_token_on_compio() {
    assert_eq!(scenario_not_cancelled().await, (false, 1));
}

/// 测试「从未 poll 就丢弃」时参数会被正常 drop，而不是泄漏。
/// - 手段：构造持有 `DropProbe` 的 future，构造后立即丢弃。
/// - 判断：drop 计数为 1（`MaybeUninit` 方案下会是 0）。
#[compio::test]
async fn drop_without_poll_on_compio() {
    assert_eq!(scenario_drop_without_poll().await, 1);
}

/// 测试正常 poll 完成后 drop，参数恰好被 drop 一次。
/// - 手段：await 完成后检查 `DropProbe` 的计数。
/// - 判断：返回 `(1, 1)`，即函数体执行过一次且探针只被 drop 一次。
#[compio::test]
async fn drop_after_poll_on_compio() {
    assert_eq!(scenario_drop_after_poll().await, (1, 1));
}

/// 测试常量泛型参数与 where-only 生命周期能正常展开与使用。
/// - 手段：构造 `WithConstAsync::<4>` 并 await。
/// - 判断：返回 `4`。
#[compio::test]
async fn const_generic_on_compio() {
    assert_eq!(scenario_const_generic().await, 4);
}

/// 测试生成的 future 满足 `Send`。
/// - 手段：对 `may_cancel_with` 的结果做编译期 `Send` 断言后再 await。
/// - 判断：编译通过且返回 `1`。
#[compio::test]
async fn send_future_on_compio() {
    assert_eq!(scenario_send().await, 1);
}
