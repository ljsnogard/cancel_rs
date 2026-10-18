//! 取消源、取消令牌与等待 future 的具体实现。

use alloc::vec::Vec;
use core::{
    fmt,
    future::Future,
    mem,
    pin::Pin,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    task::{Context, Poll, Waker},
};

use abs_cancel::TrCancellationToken;
use atomic_sync::mutex::preemptive::SpinningMutexOwned;

//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----
// CancellationSource
//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----

/// 取消信号的产生端，仿照 C# 的 `CancellationTokenSource`。
///
/// 取消源本身不是取消令牌；它只负责两件事：
///
/// - 通过 [`child_token`](CancellationSource::child_token) 派生 [`CancellationToken`]；
/// - 通过 [`cancel`](CancellationSource::cancel) 幂等地触发取消。
///
/// 取消源内联持有共享状态，不进行堆分配。派生令牌、以及创建
/// [`CancellationFuture`]，都不会额外分配内存；只有等待 future **首次被
/// poll** 时，才会在内部等待槽列表里分配一个位置。
///
/// # 生命周期
///
/// 令牌借用取消源，因此取消源必须比从它派生的所有令牌与 future 活得久；
/// 反过来，只要还有令牌存在，取消源就不能被移动。需要把令牌放进 `'static`
/// 任务时，请把取消源放进 [`Arc`](alloc::sync::Arc)，并让任务自己持有该 `Arc`。
///
/// # Examples
///
/// ```
/// use cantok_rs::CancellationSource;
///
/// let source = CancellationSource::new();
/// let token = source.child_token();
///
/// assert!(!source.is_cancelled());
/// assert!(!token.is_cancelled());
///
/// assert!(source.cancel());
/// assert!(source.is_cancelled());
/// assert!(token.is_cancelled());
/// ```
pub struct CancellationSource {
    inner_: Inner_,
}

impl CancellationSource {
    /// 创建一个尚未取消的取消源。
    ///
    /// # Examples
    ///
    /// ```
    /// use cantok_rs::CancellationSource;
    ///
    /// let source = CancellationSource::new();
    /// assert!(!source.is_cancelled());
    /// ```
    pub fn new() -> Self {
        CancellationSource {
            inner_: Inner_::new_(),
        }
    }

    /// 测试取消是否已经被触发。
    ///
    /// 该操作只读取一个原子布尔，不获取等待槽列表的锁。
    ///
    /// # Examples
    ///
    /// ```
    /// use cantok_rs::CancellationSource;
    ///
    /// let source = CancellationSource::new();
    /// assert!(!source.is_cancelled());
    /// source.cancel();
    /// assert!(source.is_cancelled());
    /// ```
    #[inline]
    pub fn is_cancelled(&self) -> bool {
        self.inner_.is_cancelled_()
    }

    /// 触发取消；幂等。
    ///
    /// 只有**第一次**调用会真正触发取消：它会置位取消标志、取走当前登记的
    /// 所有等待槽，并在释放内部锁之后逐个唤醒等待者。后续调用直接返回
    /// [`false`]，不会重复唤醒。
    ///
    /// 触发之后仍然可以调用 [`child_token`](CancellationSource::child_token)；
    /// 此时派生出的令牌一出生就处于已取消状态，不会 panic。
    ///
    /// # Returns
    ///
    /// - `true`：本次调用完成了「未取消 → 已取消」的状态转换；
    /// - `false`：取消在此前已经被触发。
    ///
    /// # Examples
    ///
    /// ```
    /// use cantok_rs::CancellationSource;
    ///
    /// let source = CancellationSource::new();
    /// assert!(source.cancel());
    /// assert!(!source.cancel());
    /// assert!(source.is_cancelled());
    /// ```
    pub fn cancel(&self) -> bool {
        self.inner_.cancel_()
    }

    /// 派生一个取消令牌。
    ///
    /// 该调用只是把取消源内部状态的引用包成令牌，**不注册等待者、不分配等待槽**。
    /// 在取消源触发之后调用也合法，返回的令牌已经处于已取消状态。
    ///
    /// # Examples
    ///
    /// ```
    /// use cantok_rs::CancellationSource;
    ///
    /// let source = CancellationSource::new();
    /// let token = source.child_token();
    ///
    /// assert!(!token.is_cancelled());
    /// source.cancel();
    /// assert!(token.is_cancelled());
    /// ```
    #[inline]
    pub fn child_token(&self) -> CancellationToken<'_> {
        CancellationToken {
            inner_: &self.inner_,
        }
    }
}

impl Default for CancellationSource {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for CancellationSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CancellationSource")
            .field("is_cancelled", &self.is_cancelled())
            .finish_non_exhaustive()
    }
}

//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----
// CancellationToken
//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----

/// 取消令牌：取消源内部状态的一个轻量引用。
///
/// 令牌实现 [`Copy`]，从同一个取消源派生出的任意多个令牌都共享同一份取消状态：
/// 取消源触发后，它们会同时变为已取消。
///
/// 本类型实现 [`abs_cancel::TrCancellationToken`]：
///
/// - [`is_cancelled`](CancellationToken::is_cancelled)：是否已收到取消信号；
/// - [`can_be_cancelled`](CancellationToken::can_be_cancelled)：本类型恒为 `true`
///   （它始终挂在某个取消源上，而不是 `CancelledToken` /
///   `NonCancellableToken` 那样的常量令牌）；
/// - [`child_token`](CancellationToken::child_token)：再派生一个指向同一状态的令牌；
/// - [`cancellation`](CancellationToken::cancellation)：创建等待取消的 future。
///
/// 所有派生操作都只是复制引用，不注册等待者；等待槽只在 `cancellation()` 的
/// future 首次被 poll 时登记。
///
/// # Examples
///
/// ```
/// use cantok_rs::CancellationSource;
///
/// let source = CancellationSource::new();
/// let first = source.child_token();
/// let second = first.child_token();
///
/// source.cancel();
/// assert!(first.is_cancelled());
/// assert!(second.is_cancelled());
/// ```
#[derive(Clone, Copy)]
pub struct CancellationToken<'a> {
    inner_: &'a Inner_,
}

impl<'a> CancellationToken<'a> {
    /// 测试该令牌是否已经收到取消信号。
    ///
    /// # Examples
    ///
    /// ```
    /// use cantok_rs::CancellationSource;
    ///
    /// let source = CancellationSource::new();
    /// let token = source.child_token();
    /// assert!(!token.is_cancelled());
    /// source.cancel();
    /// assert!(token.is_cancelled());
    /// ```
    #[inline]
    pub fn is_cancelled(&self) -> bool {
        self.inner_.is_cancelled_()
    }

    /// 测试该令牌是否属于「可取消」类型；本类型恒为 `true`。
    ///
    /// 注意：取消已经触发之后，本方法仍然返回 `true`；是否已经收到信号请用
    /// [`is_cancelled`](CancellationToken::is_cancelled) 判断。
    ///
    /// # Examples
    ///
    /// ```
    /// use cantok_rs::CancellationSource;
    ///
    /// let source = CancellationSource::new();
    /// let token = source.child_token();
    /// assert!(token.can_be_cancelled());
    /// source.cancel();
    /// assert!(token.can_be_cancelled());
    /// ```
    #[inline]
    pub const fn can_be_cancelled(&self) -> bool {
        true
    }

    /// 再派生一个取消令牌；它共享同一个取消源的状态。
    ///
    /// 该调用只复制引用，不注册等待者、不分配。
    ///
    /// # Examples
    ///
    /// ```
    /// use cantok_rs::CancellationSource;
    ///
    /// let source = CancellationSource::new();
    /// let token = source.child_token();
    /// let child = token.child_token();
    ///
    /// source.cancel();
    /// assert!(child.is_cancelled());
    /// ```
    #[inline]
    pub const fn child_token(&self) -> Self {
        *self
    }

    /// 创建一个在取消触发时就绪的 future。
    ///
    /// 该调用本身只记录一个等待者编号，不注册等待槽、不分配内存；等待槽在
    /// future **首次被 poll** 时登记。如果取消已经触发，future 首次 poll
    /// 直接返回 [`Poll::Ready`]，也不会注册。
    ///
    /// # Examples
    ///
    /// ```
    /// use cantok_rs::CancellationSource;
    ///
    /// let source = CancellationSource::new();
    /// let token = source.child_token();
    /// let wait = token.cancellation();
    ///
    /// assert!(!token.is_cancelled());
    /// drop(wait);
    /// source.cancel();
    /// assert!(token.is_cancelled());
    /// ```
    #[must_use = "`cancellation()` 只创建等待 future，必须 poll 或 await 才会等待"]
    pub fn cancellation(self) -> CancellationFuture<'a> {
        let inner = self.inner_;
        CancellationFuture {
            inner_: inner,
            id_: inner.next_waiter_id_(),
            registered_: false,
        }
    }
}

impl fmt::Debug for CancellationToken<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CancellationToken")
            .field("is_cancelled", &self.is_cancelled())
            .finish_non_exhaustive()
    }
}

impl<'a> TrCancellationToken for CancellationToken<'a> {
    type Cancellation = CancellationFuture<'a>;

    type ChildToken = Self;

    #[inline]
    fn is_cancelled(&self) -> bool {
        CancellationToken::is_cancelled(self)
    }

    #[inline]
    fn can_be_cancelled(&self) -> bool {
        CancellationToken::can_be_cancelled(self)
    }

    #[inline]
    fn child_token(&self) -> Self::ChildToken {
        CancellationToken::child_token(self)
    }

    #[inline]
    fn cancellation(self) -> Self::Cancellation {
        CancellationToken::cancellation(self)
    }
}

//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----
// CancellationFuture
//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----

/// 等待取消的 future，由 [`CancellationToken::cancellation`] 创建。
///
/// # 按需注册
///
/// 创建 future 不会注册等待者；首次 poll 时：
///
/// 1. 若取消已经触发，直接返回 [`Poll::Ready`]；
/// 2. 否则分配/复用一个等待槽，登记当前 [`Waker`]；
/// 3. **登记之后再次检查**取消标志，避免「检查 → 登记」之间发生的取消丢失唤醒。
///
/// 后续 poll 只更新等待槽里的 waker。future 被丢弃时按等待者编号注销，
/// 不会把已失效的 waker 留在取消源里。
///
/// 与取消源一样，本类型不进行额外堆分配，等待槽列表的 `Vec` 由取消源持有，
/// 首次真正等待时才可能增长。
///
/// # Examples
///
/// ```
/// use core::{
///     future::Future,
///     pin::pin,
///     task::{Context, Waker},
/// };
///
/// use cantok_rs::CancellationSource;
///
/// let source = CancellationSource::new();
/// let token = source.child_token();
/// let mut wait = pin!(token.cancellation());
/// let mut cx = Context::from_waker(Waker::noop());
///
/// assert!(wait.as_mut().poll(&mut cx).is_pending());
/// source.cancel();
/// assert!(wait.as_mut().poll(&mut cx).is_ready());
/// ```
#[must_use = "等待取消的 future 必须被 poll 或 await，否则不会等待"]
pub struct CancellationFuture<'a> {
    inner_: &'a Inner_,

    /// 等待者编号，在 `cancellation()` 时由取消源单调分配。
    id_: usize,

    /// 是否已经在取消源里登记过等待槽。
    registered_: bool,
}

impl Future for CancellationFuture<'_> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();

        // 快路径：已经取消，不需要登记。
        if this.inner_.is_cancelled_() {
            return Poll::Ready(());
        }

        // 首次 poll 登记；后续 poll 更新 waker。
        this.inner_.register_or_update_(this.id_, cx.waker());
        this.registered_ = true;

        // 复检：登记与上一次检查之间发生的取消不能丢。
        if this.inner_.is_cancelled_() {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

impl Drop for CancellationFuture<'_> {
    fn drop(&mut self) {
        if self.registered_ {
            self.inner_.unregister_(self.id_);
        }
    }
}

impl fmt::Debug for CancellationFuture<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CancellationFuture")
            .field("id", &self.id_)
            .field("registered", &self.registered_)
            .field("is_cancelled", &self.inner_.is_cancelled_())
            .finish_non_exhaustive()
    }
}

//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----
// 内部共享状态
//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----

/// 取消源与令牌共享的内部状态。
///
/// 它内联在 [`CancellationSource`] 里；令牌只是对它的一层 `&'a` 引用。
struct Inner_ {
    /// 是否已经触发取消；读侧无锁。
    cancelled_: AtomicBool,

    /// 下一个等待者编号；只在创建等待 future 时自增。
    next_waiter_id_: AtomicUsize,

    /// 已登记等待槽；用 `abs_sync.rs` 的自旋锁保护，不用 `std::sync::Mutex`。
    waiters_: SpinningMutexOwned<Vec<WaitSlot_>>,
}

impl Inner_ {
    fn new_() -> Self {
        Inner_ {
            cancelled_: AtomicBool::new(false),
            next_waiter_id_: AtomicUsize::new(0usize),
            waiters_: SpinningMutexOwned::new_owned(Vec::new()),
        }
    }

    #[inline]
    fn is_cancelled_(&self) -> bool {
        self.cancelled_.load(Ordering::Acquire)
    }

    #[inline]
    fn next_waiter_id_(&self) -> usize {
        self.next_waiter_id_.fetch_add(1usize, Ordering::Relaxed)
    }

    /// 在自旋锁保护下访问等待槽列表。
    ///
    /// 这里用 `try_lock` + 自旋，而不是 `lock().wait()`，因此不需要引入
    /// `abs_sync` 的 `TrMayBreak`；临界区只做 `Vec` 的查找 / 推入 / 取出，
    /// 不会 `await`，也不会调用用户回调。
    fn with_waiters_<R>(&self, f: impl FnOnce(&mut Vec<WaitSlot_>) -> R) -> R {
        let mut session = self.waiters_.lock_session();
        loop {
            if let Ok(mut guard) = session.try_lock() {
                return f(&mut guard);
            }
            core::hint::spin_loop();
        }
    }

    /// 登记等待者；若同一编号已存在，则更新 waker。
    ///
    /// 旧 waker 在释放自旋锁**之后**才 drop，避免用户自定义 waker 的 drop
    /// 重入取消源。
    fn register_or_update_(&self, id: usize, waker: &Waker) {
        let replaced = self.with_waiters_(|waiters| {
            if let Some(slot) = waiters.iter_mut().find(|slot| slot.id_ == id) {
                if slot.waker_.will_wake(waker) {
                    None
                } else {
                    Some(mem::replace(&mut slot.waker_, waker.clone()))
                }
            } else {
                waiters.push(WaitSlot_ {
                    id_: id,
                    waker_: waker.clone(),
                });
                None
            }
        });
        drop(replaced);
    }

    /// 注销等待者；返回的等待槽在释放自旋锁之后才 drop。
    fn unregister_(&self, id: usize) {
        let removed = self.with_waiters_(|waiters| {
            waiters
                .iter()
                .position(|slot| slot.id_ == id)
                .map(|index| waiters.swap_remove(index))
        });
        drop(removed);
    }

    /// 触发取消：置位、取走全部等待槽，然后在锁外唤醒。
    ///
    /// 返回本次调用是否完成了状态转换。
    fn cancel_(&self) -> bool {
        if self.cancelled_.swap(true, Ordering::AcqRel) {
            return false;
        }

        // 先整体取走，再释放锁；`wake()` 可能同步重入 poll，绝不能在持锁时调用。
        let waiters = self.with_waiters_(mem::take);
        for slot in waiters {
            slot.waker_.wake();
        }
        true
    }
}

/// 一个已登记的等待槽。
struct WaitSlot_ {
    /// 等待者编号；用来在重复 poll 与 Drop 时定位自己。
    id_: usize,

    /// 当前登记的 waker。
    waker_: Waker,
}

//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----
// 单元测试
//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----

#[cfg(test)]
mod tests_ {
    use alloc::vec::Vec;
    use core::future::Future;
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        task::{Context, Wake, Waker},
    };

    use abs_cancel::TrCancellationToken;

    use super::{CancellationFuture, CancellationSource, CancellationToken};

    /// 计数唤醒器：统计 `wake` / `wake_by_ref` 被调用的次数。
    struct CountingWake_ {
        count_: AtomicUsize,
    }

    impl Wake for CountingWake_ {
        fn wake(self: Arc<Self>) {
            self.count_.fetch_add(1usize, Ordering::SeqCst);
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.count_.fetch_add(1usize, Ordering::SeqCst);
        }
    }

    /// 构造一个带计数的 `Waker`。
    ///
    /// - 手段：把 [`CountingWake_`] 包进 `Arc` 再转成 `Waker`。
    /// - 判断：返回的 `Waker` 每次被唤醒都会让计数加一，供各用例断言。
    fn counting_waker_() -> (Waker, Arc<CountingWake_>) {
        let probe = Arc::new(CountingWake_ {
            count_: AtomicUsize::new(0),
        });
        (Waker::from(probe.clone()), probe)
    }

    /// 读取取消源当前登记的等待槽数量（测试内部状态用）。
    ///
    /// - 手段：在自旋锁保护下读取内部 `Vec` 的长度。
    /// - 判断：返回值的增减直接反映「是否注册 / 是否注销」。
    fn waiter_count_(source: &CancellationSource) -> usize {
        source.inner_.with_waiters_(|waiters| waiters.len())
    }

    /// 测试创建子令牌与未 poll 的等待 future 都不会提前注册等待槽。
    /// - 手段：创建取消源、一个令牌、若干子令牌，再创建但不 poll 一个
    ///   `cancellation()` future；每一步之后读取内部等待槽数量。
    /// - 判断：所有中间状态的等待槽数量都必须为 `0`，证明注册是惰性的。
    ///
    /// Verifies that child tokens and an unpolled cancellation future do not
    /// register a waiter early.
    /// - Method: create a source, a token, many child tokens, and an unpolled
    ///   `cancellation()` future; read the internal waiter count after each step.
    /// - Criterion: every intermediate waiter count must be `0`, proving lazy
    ///   registration.
    #[test]
    fn child_token_and_unpolled_future_do_not_register() {
        let source = CancellationSource::new();
        let token = source.child_token();

        let mut children = Vec::new();
        for _ in 0..64 {
            children.push(token.child_token());
        }
        assert_eq!(waiter_count_(&source), 0usize);

        let wait = children[0].cancellation();
        assert_eq!(waiter_count_(&source), 0usize);

        drop(wait);
        assert_eq!(waiter_count_(&source), 0usize);
    }

    /// 测试等待 future 首次 poll 才登记，Drop 时注销。
    /// - 手段：创建 future 并读取等待槽数量，poll 一次后再读，最后在作用域
    ///   结束时丢弃 future 再读。
    /// - 判断：poll 前后数量分别为 `0`、`1`；future 被丢弃后回到 `0`。
    ///
    /// Verifies that a cancellation future registers on its first poll and
    /// unregisters when dropped.
    /// - Method: read the waiter count before polling, after one poll, and after
    ///   dropping the future at the end of a scope.
    /// - Criterion: the counts are `0`, `1`, and `0` respectively.
    #[test]
    fn first_poll_registers_and_drop_unregisters() {
        let source = CancellationSource::new();
        let token = source.child_token();
        let (waker, _probe) = counting_waker_();
        let mut cx = Context::from_waker(&waker);

        {
            let mut wait = core::pin::pin!(token.cancellation());
            assert_eq!(waiter_count_(&source), 0usize);
            assert!(wait.as_mut().poll(&mut cx).is_pending());
            assert_eq!(waiter_count_(&source), 1usize);
        }

        assert_eq!(waiter_count_(&source), 0usize);
    }

    /// 测试取消会唤醒等待者，且重复取消不会重复唤醒。
    /// - 手段：poll 一个 future 至 `Pending`，调用 `cancel()` 两次，再 poll
    ///   该 future 至完成；全程统计唤醒次数。
    /// - 判断：第一次 `cancel()` 返回 `true` 且唤醒计数为 `1`；第二次返回
    ///   `false` 且计数仍为 `1`；最终 future 返回 `Ready`。
    ///
    /// Verifies that cancellation wakes the waiter once and is idempotent.
    /// - Method: poll a future to `Pending`, call `cancel()` twice, then poll the
    ///   future to completion while counting wake-ups.
    /// - Criterion: the first `cancel()` returns `true` and wakes once; the
    ///   second returns `false` and does not wake again; the future completes.
    #[test]
    fn cancel_wakes_waiter_once_and_is_idempotent() {
        let source = CancellationSource::new();
        let token = source.child_token();
        let (waker, probe) = counting_waker_();
        let mut cx = Context::from_waker(&waker);

        let mut wait = core::pin::pin!(token.cancellation());
        assert!(wait.as_mut().poll(&mut cx).is_pending());
        assert_eq!(probe.count_.load(Ordering::SeqCst), 0usize);

        assert!(source.cancel());
        assert_eq!(probe.count_.load(Ordering::SeqCst), 1usize);

        assert!(!source.cancel());
        assert_eq!(probe.count_.load(Ordering::SeqCst), 1usize);

        assert!(wait.as_mut().poll(&mut cx).is_ready());
        assert_eq!(probe.count_.load(Ordering::SeqCst), 1usize);
    }

    /// 测试「先取消、后等待」时 future 首次 poll 立即就绪且不登记等待槽。
    /// - 手段：先调用 `cancel()`，再创建并 poll 一个 `cancellation()` future。
    /// - 判断：首次 poll 返回 `Ready`；唤醒计数为 `0`；等待槽数量保持 `0`。
    ///
    /// Verifies that waiting after cancellation completes immediately without
    /// registering a waiter.
    /// - Method: call `cancel()` first, then create and poll a cancellation
    ///   future.
    /// - Criterion: the first poll is `Ready`, the waker is never called, and
    ///   the waiter count stays `0`.
    #[test]
    fn already_cancelled_future_is_ready_without_registration() {
        let source = CancellationSource::new();
        let token = source.child_token();
        assert!(source.cancel());
        assert!(token.is_cancelled());

        let (waker, probe) = counting_waker_();
        let mut cx = Context::from_waker(&waker);
        let mut wait = core::pin::pin!(token.cancellation());

        assert_eq!(waiter_count_(&source), 0usize);
        assert!(wait.as_mut().poll(&mut cx).is_ready());
        assert_eq!(probe.count_.load(Ordering::SeqCst), 0usize);
        assert_eq!(waiter_count_(&source), 0usize);
    }

    /// 测试多个等待者都会在取消时被唤醒。
    /// - 手段：从同一取消源派生两个令牌，各自 poll 至 `Pending`，再调用一次
    ///   `cancel()`。
    /// - 判断：取消后两个唤醒计数都为 `1`，两个 future 都返回 `Ready`。
    ///
    /// Verifies that every registered waiter is woken by one cancellation.
    /// - Method: derive two tokens from the same source, poll both to `Pending`,
    ///   then cancel once.
    /// - Criterion: both wake counters become `1` and both futures complete.
    #[test]
    fn multiple_waiters_are_all_woken() {
        let source = CancellationSource::new();
        let token_a = source.child_token();
        let token_b = source.child_token();
        let (waker_a, probe_a) = counting_waker_();
        let (waker_b, probe_b) = counting_waker_();
        let mut cx_a = Context::from_waker(&waker_a);
        let mut cx_b = Context::from_waker(&waker_b);

        let mut wait_a = core::pin::pin!(token_a.cancellation());
        let mut wait_b = core::pin::pin!(token_b.cancellation());
        assert!(wait_a.as_mut().poll(&mut cx_a).is_pending());
        assert!(wait_b.as_mut().poll(&mut cx_b).is_pending());
        assert_eq!(waiter_count_(&source), 2usize);

        assert!(source.cancel());
        assert_eq!(probe_a.count_.load(Ordering::SeqCst), 1usize);
        assert_eq!(probe_b.count_.load(Ordering::SeqCst), 1usize);
        assert!(wait_a.as_mut().poll(&mut cx_a).is_ready());
        assert!(wait_b.as_mut().poll(&mut cx_b).is_ready());
    }

    /// 测试重复 poll 只更新 waker，不会重复登记等待槽。
    /// - 手段：同一个 future 先用 waker A poll 一次，再用 waker B poll 一次，
    ///   然后取消并检查两个 waker 的唤醒次数。
    /// - 判断：等待槽数量始终为 `1`；取消只唤醒最新的 waker B。
    ///
    /// Verifies that re-polling updates the waker in place instead of
    /// registering a second waiter.
    /// - Method: poll one future with waker A, then with waker B, cancel, and
    ///   check both wake counters.
    /// - Criterion: the waiter count stays `1` and only waker B is woken.
    #[test]
    fn repoll_updates_waker_without_duplicate_slot() {
        let source = CancellationSource::new();
        let token = source.child_token();
        let (waker_a, probe_a) = counting_waker_();
        let (waker_b, probe_b) = counting_waker_();
        let mut cx_a = Context::from_waker(&waker_a);
        let mut cx_b = Context::from_waker(&waker_b);

        let mut wait = core::pin::pin!(token.cancellation());
        assert!(wait.as_mut().poll(&mut cx_a).is_pending());
        assert!(wait.as_mut().poll(&mut cx_b).is_pending());
        assert_eq!(waiter_count_(&source), 1usize);

        assert!(source.cancel());
        assert_eq!(probe_a.count_.load(Ordering::SeqCst), 0usize);
        assert_eq!(probe_b.count_.load(Ordering::SeqCst), 1usize);
        assert_eq!(waiter_count_(&source), 0usize);
    }

    /// 测试已取消源上新派生的令牌直接处于已取消状态。
    /// - 手段：先取消源，再调用 `child_token()`，最后 poll 该令牌的
    ///   `cancellation()` future。
    /// - 判断：令牌的 `is_cancelled` 为 `true`；future 首次 poll 即 `Ready`；
    ///   没有登记等待槽，也没有发生唤醒。
    ///
    /// Verifies that a token derived after cancellation is already cancelled.
    /// - Method: cancel the source, derive a child token, and poll its
    ///   cancellation future.
    /// - Criterion: the token is cancelled, the future is immediately ready, and
    ///   no waiter or wake-up is involved.
    #[test]
    fn child_token_after_cancel_is_already_cancelled() {
        let source = CancellationSource::new();
        assert!(source.cancel());

        let token = source.child_token();
        assert!(token.is_cancelled());

        let (waker, probe) = counting_waker_();
        let mut cx = Context::from_waker(&waker);
        let mut wait = core::pin::pin!(token.cancellation());

        assert!(wait.as_mut().poll(&mut cx).is_ready());
        assert_eq!(probe.count_.load(Ordering::SeqCst), 0usize);
        assert_eq!(waiter_count_(&source), 0usize);
    }

    /// 测试令牌是 `Copy`，且 `TrCancellationToken` 的方法与固有方法一致。
    /// - 手段：复制令牌并派生 child token；把它们传给以
    ///   `TrCancellationToken` 为约束的泛型函数；取消后通过 trait 方法检查。
    /// - 判断：复制与派生后的令牌都观察到取消；`can_be_cancelled` 恒为
    ///   `true`；编译通过本身也验证了 trait 的实现与 `Copy`。
    ///
    /// Verifies that the token is `Copy` and its `TrCancellationToken` methods
    /// agree with the inherent ones.
    /// - Method: copy the token, derive a child, pass both through a generic
    ///   function bounded by `TrCancellationToken`, then cancel and inspect them
    ///   through trait methods.
    /// - Criterion: both handles observe cancellation, `can_be_cancelled` stays
    ///   `true`, and the code compiles.
    #[test]
    fn token_is_copy_and_implements_abs_cancel_trait() {
        fn keep_token_<C>(token: C) -> C
        where
            C: TrCancellationToken,
        {
            token
        }

        let source = CancellationSource::new();
        let token = source.child_token();
        let copied = token;
        let child = token.child_token();

        let copied = keep_token_(copied);
        let child = keep_token_(child);

        assert!(TrCancellationToken::can_be_cancelled(&copied));
        assert!(TrCancellationToken::can_be_cancelled(&child));
        assert!(source.cancel());
        assert!(TrCancellationToken::is_cancelled(&copied));
        assert!(TrCancellationToken::is_cancelled(&child));
    }

    /// 测试公开类型满足 `Send + Sync`。
    /// - 手段：用两个要求 `T: Send + Sync` 的泛型断言函数实例化
    ///   `CancellationSource`、`CancellationToken` 与 `CancellationFuture`。
    /// - 判断：编译通过即说明这三个类型可以跨线程共享 / 发送。
    ///
    /// Verifies that the public types are `Send + Sync`.
    /// - Method: instantiate `CancellationSource`, `CancellationToken`, and
    ///   `CancellationFuture` through a generic assertion requiring
    ///   `T: Send + Sync`.
    /// - Criterion: successful compilation proves the bounds.
    #[test]
    fn public_types_are_send_and_sync() {
        fn assert_send_sync_<T>()
        where
            T: Send + Sync,
        {
        }

        assert_send_sync_::<CancellationSource>();
        assert_send_sync_::<CancellationToken<'static>>();
        assert_send_sync_::<CancellationFuture<'static>>();
    }

    /// 测试 `Default` 创建的是未取消的取消源。
    /// - 手段：用 `CancellationSource::default()` 创建源。
    /// - 判断：`is_cancelled()` 为 `false`。
    ///
    /// Verifies that `Default` creates a source that has not been cancelled.
    /// - Method: create a source via `CancellationSource::default()`.
    /// - Criterion: `is_cancelled()` returns `false`.
    #[test]
    fn default_source_is_not_cancelled() {
        let source = CancellationSource::default();
        assert!(!source.is_cancelled());
    }
}
