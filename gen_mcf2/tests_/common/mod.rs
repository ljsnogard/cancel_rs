//! 与具体异步运行时无关的测试场景。
//!
//! 各运行时的测试文件（如 `compio_.rs`）只负责在运行时里调用这里的 async
//! 函数并断言结果，因此以后新增运行时不需要重写场景逻辑。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use abs_cancel::{NonCancellableToken, TrCancellationToken, TrMayCancel};
use gen_mcf2::gen_may_cancel_future;

/// 测试用的取消令牌：用一个原子布尔模拟「已收到取消信号」。
#[derive(Clone, Default)]
pub(crate) struct TestToken(Arc<AtomicBool>);

impl TestToken {
    /// 创建一个尚未取消的令牌。
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// 模拟收到取消信号。
    pub(crate) fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

impl TrCancellationToken for TestToken {
    type Cancellation = core::future::Ready<()>;
    type ChildToken = Self;

    fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    fn can_be_cancelled(&self) -> bool {
        true
    }

    fn child_token(&self) -> Self::ChildToken {
        self.clone()
    }

    fn cancellation(self) -> Self::Cancellation {
        core::future::ready(())
    }
}

/// 记录自身被 drop 次数的探针，用来验证参数不会泄漏。
pub(crate) struct DropProbe(Arc<AtomicUsize>);

impl Drop for DropProbe {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// README 里的示例：多生命周期、非 `Copy` 参数、返回类型引用用户生命周期。
///
/// 函数体与 README 完全一致（包括 `Vec::new()` + `push` 的写法），因此这里
/// 放行对应的 clippy 提示。
#[allow(clippy::vec_init_then_push)]
#[gen_may_cancel_future(DoThing)]
pub(crate) async fn do_thing_async<'a, 'b, 'x, A, B, C>(
    a: &'a mut A,
    b: &'b mut B,
    l: usize,
    x: core::slice::Iter<'x, A>,
    cancel: C,
) -> Vec<<core::slice::Iter<'a, A> as Iterator>::Item>
where
    A: Send,
    B: Sync,
    C: TrCancellationToken,
{
    let _ = (b, l, x, cancel);
    let mut vec: Vec<<core::slice::Iter<'a, A> as Iterator>::Item> = Vec::new();
    vec.push(a);
    vec
}

/// 观察取消令牌是否已经被取消。
#[gen_may_cancel_future(ObserveCancel)]
#[allow(clippy::needless_lifetimes)]
pub(crate) async fn observe_cancel_async<'f, C>(hits: &'f mut usize, cancel: C) -> bool
where
    C: TrCancellationToken,
{
    if cancel.is_cancelled() {
        return true;
    }
    *hits += 1;
    false
}

/// 参数全部 `Send`，用于断言生成的 future 满足 `Send`。
#[gen_may_cancel_future(SendThing)]
#[allow(clippy::needless_lifetimes)]
pub(crate) async fn send_thing_async<'f, A, C>(a: &'f mut A, cancel: C) -> usize
where
    A: Send + Sync,
    C: TrCancellationToken,
{
    let _ = (a, cancel);
    1
}

/// 持有 `DropProbe` 的异步函数，用于验证丢弃未 poll 的 future 不会泄漏参数。
#[gen_may_cancel_future(ProbeDrop)]
#[allow(clippy::needless_lifetimes)]
pub(crate) async fn probe_drop_async<'f, C>(
    probe: DropProbe,
    hits: &'f mut usize,
    cancel: C,
) -> usize
where
    C: TrCancellationToken,
{
    // 借用一下 probe，保证它活到函数体结束；完成时它会被 drop。
    let _ = (&probe, cancel);
    *hits += 1;
    *hits
}

/// 带常量泛型参数与「只在 where 子句出现」的生命周期。
#[gen_may_cancel_future(WithConst)]
pub(crate) async fn with_const_async<'f, const N: usize, C>(arr: [u8; N], cancel: C) -> usize
where
    'f: 'static,
    C: TrCancellationToken,
{
    let _ = (arr, cancel);
    N
}

/// 私有的异步函数，但把生成条目放宽成 `pub`，供 crate 根模块命名与构造。
#[allow(clippy::needless_lifetimes)]
#[gen_may_cancel_future(Widened, pub)]
async fn widened_async_<'f, C>(n: &'f mut usize, cancel: C) -> usize
where
    C: TrCancellationToken,
{
    let _ = cancel;
    *n += 1;
    *n
}

/// 私有的异步函数：生成的**类型**公开，但构造函数只在 crate 内可用。
///
/// 这是「外部可以命名与消费这个类型、但只有本 crate 能构造它」的封装模式：
/// `pub` 的类型让 `SplitAsync` 出现在对外 API 里，`pub(crate)` 的 `new` 则把
/// 构造权收回 crate 内部（crate 根模块仍然调得动，crate 外部调不动）。
#[allow(clippy::needless_lifetimes)]
#[gen_may_cancel_future(Split, pub, new(pub(crate)))]
async fn split_async_<'f, C>(n: &'f mut usize, cancel: C) -> usize
where
    C: TrCancellationToken,
{
    let _ = cancel;
    *n += 2;
    *n
}

/// 场景：`.await` 不可取消路径。
pub(crate) async fn scenario_into_future() -> Vec<u8> {
    let mut a = 1u8;
    let mut b = 2u8;
    let data = [3u8, 4u8];
    let x = data.iter();
    let fut = DoThingAsync::new(&mut a, &mut b, 0usize, x);
    fut.await.into_iter().copied().collect()
}

/// 场景：`may_cancel_with` 可取消路径（令牌未取消）。
pub(crate) async fn scenario_may_cancel() -> Vec<u8> {
    let mut a = 5u8;
    let mut b = 6u8;
    let data = [7u8];
    let x = data.iter();
    let fut = DoThingAsync::new(&mut a, &mut b, 0usize, x);
    let out = fut.may_cancel_with(NonCancellableToken::new()).await;
    out.into_iter().copied().collect()
}

/// 场景：令牌已被取消时，函数体能立即观察到取消信号。
pub(crate) async fn scenario_cancelled() -> bool {
    let token = TestToken::new();
    token.cancel();
    let mut hits = 0usize;
    let fut = ObserveCancelAsync::new(&mut hits);
    fut.may_cancel_with(token).await
}

/// 场景：令牌未取消时，函数体正常执行。
pub(crate) async fn scenario_not_cancelled() -> (bool, usize) {
    let token = TestToken::new();
    let mut hits = 0usize;
    let fut = ObserveCancelAsync::new(&mut hits);
    let cancelled = fut.may_cancel_with(token).await;
    (cancelled, hits)
}

/// 场景：丢弃一个从未 poll 的 future，参数应当被正常 drop（而不是泄漏）。
pub(crate) async fn scenario_drop_without_poll() -> usize {
    let counter = Arc::new(AtomicUsize::new(0));
    {
        let mut hits = 0usize;
        let fut = ProbeDropAsync::new(DropProbe(counter.clone()), &mut hits);
        let _unpolled = fut.may_cancel_with(NonCancellableToken::new());
    }
    counter.load(Ordering::SeqCst)
}

/// 场景：正常 poll 完成后，参数恰好被 drop 一次（不能多也不能少）。
pub(crate) async fn scenario_drop_after_poll() -> (usize, usize) {
    let counter = Arc::new(AtomicUsize::new(0));
    let mut hits = 0usize;
    let fut = ProbeDropAsync(
        DropProbe(counter.clone()),
        &mut hits,
        core::marker::PhantomData,
    );
    let seen = fut.may_cancel_with(NonCancellableToken::new()).await;
    (seen, counter.load(Ordering::SeqCst))
}

/// 场景：常量泛型参数与 where-only 生命周期可以正常使用。
pub(crate) async fn scenario_const_generic() -> usize {
    let fut = WithConstAsync::<4>::new([1u8, 2, 3, 4]);
    fut.await
}

/// 场景：生成的 future 满足 `Send`。
pub(crate) async fn scenario_send() -> usize {
    let mut a = 0u8;
    let fut = SendThingAsync::new(&mut a);
    let fut = fut.may_cancel_with(NonCancellableToken::new());
    assert_send(&fut);
    fut.await
}

/// 在编译期断言 `T: Send`。
pub(crate) fn assert_send<T: Send>(_: &T) {}
