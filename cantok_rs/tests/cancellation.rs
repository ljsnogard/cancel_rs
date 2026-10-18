//! `cantok_rs` 的集成测试：只通过公开 API 验证取消源、令牌与等待 future 的协作。
//!
//! Integration tests for `cantok_rs`: they exercise the cancellation source,
//! tokens, and wait futures through the public API only.

use core::{
    future::Future,
    pin::pin,
    task::{Context, Waker},
};
use std::{
    sync::{Arc, Barrier},
    task::Wake,
    thread,
};

use abs_cancel::TrCancellationToken;
use cantok_rs::CancellationSource;

/// 测试派生令牌、等待 future 与取消源共享同一份取消状态。
/// - 手段：从同一取消源派生令牌与 child token，把令牌传给要求
///   `TrCancellationToken` 约束的泛型函数；先 poll 等待 future 至 `Pending`，
///   再调用 `cancel()`，最后再次 poll。
/// - 判断：取消前令牌未取消；取消后令牌与 child token 都已取消；等待 future
///   从 `Pending` 变为 `Ready`。
///
/// Verifies that derived tokens, wait futures, and the source share the same
/// cancellation state.
/// - Method: derive a token and a child token, pass the token through a generic
///   function bounded by `TrCancellationToken`, poll its wait future to
///   `Pending`, cancel the source, then poll again.
/// - Criterion: the token is not cancelled before the call; after the call both
///   tokens observe cancellation and the wait future completes.
#[test]
fn child_token_and_wait_future_share_source_state() {
    fn keep_token_<C>(token: C) -> C
    where
        C: TrCancellationToken,
    {
        token
    }

    let source = CancellationSource::new();
    let token = keep_token_(source.child_token());
    let child = token.child_token();

    assert!(!token.is_cancelled());
    assert!(!child.is_cancelled());

    let mut wait = pin!(token.cancellation());
    let mut cx = Context::from_waker(Waker::noop());
    assert!(wait.as_mut().poll(&mut cx).is_pending());

    assert!(source.cancel());
    assert!(token.is_cancelled());
    assert!(child.is_cancelled());
    assert!(wait.as_mut().poll(&mut cx).is_ready());
}

/// 测试取消能唤醒其它线程上已经挂起的等待者。
/// - 手段：用 `std::thread::scope` 启动多个线程，每个线程从共享取消源派生
///   令牌、poll 一次等待 future 至 `Pending`，然后在 barrier 上汇合；主线程等
///   所有等待者登记完成后调用 `cancel()`，各线程继续 poll 至 `Ready`。
/// - 判断：`cancel()` 返回 `true`；所有线程都能结束且观察到已取消，没有线程
///   因为丢失唤醒而永久阻塞（测试本身有外部超时兜底）。
///
/// Verifies that cancellation wakes parked waiters on other threads.
/// - Method: spawn several scoped threads; each derives a token from the shared
///   source, polls its wait future once to `Pending`, and meets at a barrier.
///   The main thread cancels after all waiters have registered, and each thread
///   polls until completion.
/// - Criterion: `cancel()` returns `true`, every thread finishes and observes
///   cancellation, and no thread is left blocked by a lost wake-up.
#[test]
fn cancellation_wakes_waiters_on_other_threads() {
    const WAITERS: usize = 8;

    /// 让 `Waker::wake` 直接 unpark 所属线程。
    struct ThreadWake_ {
        thread_: thread::Thread,
    }

    impl Wake for ThreadWake_ {
        fn wake(self: Arc<Self>) {
            self.thread_.unpark();
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.thread_.unpark();
        }
    }

    let source = CancellationSource::new();
    let barrier = Barrier::new(WAITERS + 1);

    thread::scope(|scope| {
        let mut joins = Vec::with_capacity(WAITERS);

        for _ in 0..WAITERS {
            joins.push(scope.spawn(|| {
                let token = source.child_token();
                let waker = Waker::from(Arc::new(ThreadWake_ {
                    thread_: thread::current(),
                }));
                let mut cx = Context::from_waker(&waker);
                let mut wait = pin!(token.cancellation());

                assert!(wait.as_mut().poll(&mut cx).is_pending());
                barrier.wait();

                loop {
                    if wait.as_mut().poll(&mut cx).is_ready() {
                        break;
                    }
                    thread::park();
                }

                assert!(token.is_cancelled());
            }));
        }

        // 所有线程都已完成首次 poll（等待槽已登记）之后才触发取消。
        barrier.wait();
        assert!(source.cancel());

        for join in joins {
            join.join().expect("等待线程不应 panic");
        }
    });
}
