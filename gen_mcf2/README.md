# `gen_mcf2`

`gen_mcf2` 是 `gen_mcf_macro::gen_may_cancel_future` 面向 `abs_cancel` v0.2 的重制版：
把「一个 async fn」展开成「一套可被优雅取消的 future 类型」。与旧宏不同的是，本宏
内部用 [`zyn`](https://crates.io/crates/zyn) 的模板、元素与诊断设施实现。

## 前置

### nightly only

生成的代码目前只需要如下 nightly feature：

```rust
#![feature(impl_trait_in_assoc_type)]
```

> 旧设计用到的 `unboxed_closures` / `try_trait_v2` / `min_specialization` 已经不再需要。

### 依赖

`abs_cancel` 版本 `v0.2`：

```rust
pub trait TrMayCancel<'a>
where
    Self: 'a + IntoFuture<Output = Self::MayCancelOutput>,
{
    type MayCancelFuture<'f, C>: IntoFuture<Output = Self::MayCancelOutput>
    where
        'f: 'a,
        Self: 'f,
        C: 'f + TrCancellationToken;

    type MayCancelOutput;

    fn may_cancel_with<C>(
        self,
        cancel: C,
    ) -> Self::MayCancelFuture<'a, C>
    where
        C: 'a + TrCancellationToken;
}
```

相比 `v0.1` ，传入 cancellation token 的形式不再强制为 `&mut C` ，这将更有利于简化生命周期问题。

**关于 crate 路径**：生成的代码里写的是 `abs_cancel::...`（而不是顶格的
`::abs_cancel::...`）。因此调用方需要让 `abs_cancel` 这个名字在作用域内可用。
本工作区里 `abs_cancel` 通常不会成为顶格依赖，而是由别的 crate 通过 `x_deps`
重导出（例如 `abs_buff::x_deps::abs_cancel`）；这种场合请在模块里显式
`use ...::x_deps::abs_cancel;`，或者直接以 `abs_cancel` 作为本 crate 的依赖。

## 用法

```rust
//! # Usage Rules:
//! 0. Must be an `async fn`;
//! 1. Explicitly declare all lifetimes needed, and DO NOT name a lifetime with any underscore suffix;
//! 2. The last generic type param must be for the cancellation token type and constrained with: `TrCancellationToken`;
//! 3. Use a where clause to declare ALL constrains;
//! 4. The last function parameter must be that cancellation token type, taken **by value**;
//! 5. Optionally widen the visibility of the generated items: `#[gen_may_cancel_future(Prefix, pub)]`;
//! 6. Optionally give the `new` method its own visibility: `#[gen_may_cancel_future(Prefix, pub, new(pub(crate)))]`;

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
    C: abs_cancel::TrCancellationToken,
{
    let _ = (b, l, x, cancel);
    let mut vec: Vec<<core::slice::Iter<'a, A> as Iterator>::Item> = Vec::new();
    vec.push(a);
    vec
}
```

展开出来的 `DoThingAsync` 用宏生成的 `new` 构造（尾部标记字段由它自动补齐）：

```rust
let fut = DoThingAsync::new(&mut a, &mut b, 0usize, iter);
let out = fut.await;                                   // 不可取消路径
let out = fut.may_cancel_with(token).await;            // 可取消路径（需 `use abs_cancel::TrMayCancel;`）
```

> `new` 的形参顺序与原 `async fn` 的参数一致（不含取消令牌），形参名也尽量沿用原参数名；
> `_` 之类的模式无法沿用时会回退为 `p0, p1, ...`。
> `DoThingAsync` 仍然是元组结构体（尾部是标记字段），模块内也可以直接写
> `DoThingAsync(a, b, l, x, core::marker::PhantomData)`。

## 生成条目的访问级别

宏参数可以在前缀之后指定访问级别。前缀后面能按**任意顺序**给出至多一个**裸访问级别**
（作用于生成类型 `{Prefix}Async` / `{Prefix}Future`）与至多一个 `new(<访问级别>)`
（单独作用于 `{Prefix}Async::new`），两者都可以省略：

```rust
// 类型放宽到 pub；`new` 缺省跟随类型，因此也是 pub
#[gen_may_cancel_future(ProducerWrite, pub)]

// 也可以写 pub(crate) / pub(super) / pub(in path)
#[gen_may_cancel_future(DoThing, pub(crate))]

// 类型是 pub，构造函数只在 crate 内可用 —— 本次新增的能力
#[gen_may_cancel_future(DoThing, pub, new(pub(crate)))]

// 只改构造函数，类型仍与输入函数一致；顺序无关，`new(...)` 也可以写在前面
#[gen_may_cancel_future(DoThing, new(pub(crate)))]
```

典型场景是「私有的 `async fn` + 公开的包装方法」：

```rust
#[gen_may_cancel_future(ProducerWrite, pub)]
async fn producer_write_async_<'f, /* ... */>(/* ... */) -> /* ... */ { /* ... */ }
```

- **类型**的访问级别作用于 `{Prefix}Async` 与 `{Prefix}Future`；不指定时与输入函数一致。
  指定得比输入函数**更严格**会被拒绝（宏只做保守判定：`pub` / `pub(crate)` /
  `pub(super)` / `pub(in path)` / 私有之间的可判定关系会检查，双方都是
  `pub(in path)` 这类无法比较的组合一律放行）；
- **`new`** 的访问级别只作用于 `{Prefix}Async::new`；不指定时**跟随类型**的访问级别。
  它**不做任何校验** —— 比类型更开放、比输入函数更严格都照常展开，由使用者负责。
  它的用途是把构造权收回 crate 内部：

  ```rust
  // 生成的条目（假设模块路径已被 `pub use` 重导出）
  pub struct FooAsync<'f, 'lt_fut__, /* ... */>(/* 原参数与 PhantomData 标记 */);
  impl<'f, 'lt_fut__, /* ... */> FooAsync<'f, 'lt_fut__, /* ... */> {
      pub(crate) fn new(/* ... */) -> Self { /* ... */ }
  }
  ```

  于是**外部 crate 能命名 `FooAsync`，却调不动 `FooAsync::new`**（会得到
  `error[E0624]: associated function \`new\` is private`），只有本 crate 内的代码
  能构造它 —— 对外表现为「只能消费、不能自造」；
- **原函数自身的可见性不变**；
- ⚠️ 这里只改变**条目自身**的可见性。条目对外的实际可达性是
  「自身可见性 ∩ 所有外层模块的可见性」，所以若生成代码所在模块本身是私有的
  （例如 `mod segm_;`），仅写 `pub` 对外仍然不可见，还需要 `pub use` 重导出：

  ```rust
  mod segm_;                       // 私有模块
  pub use segm_::{FooAsync, FooFuture};   // 这样外部 crate 才能命名它们
  ```

  这也是「`pub` 像没生效」最常见的原因。

## 展开结果

上面的例子展开后（本段由宏的真实输出经 `rustfmt` 得到；同一份示例函数也出现在
`tests_/common/mod.rs` 里，由集成测试真实编译并运行）：

```rust
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
    C: abs_cancel::TrCancellationToken,
{
    let _ = (b, l, x, cancel);
    let mut vec: Vec<<core::slice::Iter<'a, A> as Iterator>::Item> = Vec::new();
    vec.push(a);
    vec
}
pub(crate) struct DoThingAsync<'a, 'b, 'x, 'lt_fut__, A, B>(
    &'a mut A,
    &'b mut B,
    usize,
    core::slice::Iter<'x, A>,
    ::core::marker::PhantomData<&'lt_fut__ ()>,
)
where
    A: Send,
    B: Sync,
    'a: 'lt_fut__,
    'b: 'lt_fut__,
    'x: 'lt_fut__;
impl<'a, 'b, 'x, 'lt_fut__, A, B> DoThingAsync<'a, 'b, 'x, 'lt_fut__, A, B>
where
    A: Send,
    B: Sync,
    'a: 'lt_fut__,
    'b: 'lt_fut__,
    'x: 'lt_fut__,
{
    #[doc = " 构造本类型；参数顺序与原 `async fn` 的参数一致（不含取消令牌），"]
    #[doc = " 尾部的 `PhantomData` 标记字段由本方法自动补齐。"]
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::new_without_default)]
    #[inline]
    pub(crate) fn new(a: &'a mut A, b: &'b mut B, l: usize, x: core::slice::Iter<'x, A>) -> Self {
        DoThingAsync(a, b, l, x, ::core::marker::PhantomData)
    }
}
pub(crate) struct DoThingFuture<'a, 'b, 'x, 'lt_fut__, A, B, TyTok__>
where
    A: Send,
    B: Sync,
    'a: 'lt_fut__,
    'b: 'lt_fut__,
    'x: 'lt_fut__,
    TyTok__: abs_cancel::TrCancellationToken,
{
    params_: ::core::option::Option<DoThingAsync<'a, 'b, 'x, 'lt_fut__, A, B>>,
    cancel_: ::core::option::Option<TyTok__>,
    future_: ::core::option::Option<
        <() as TrDoThingFutureFactory<'a, 'b, 'x, 'lt_fut__, A, B, TyTok__>>::MadeFuture,
    >,
}
impl<'a, 'b, 'x, 'lt_fut__, A, B> ::core::future::IntoFuture
    for DoThingAsync<'a, 'b, 'x, 'lt_fut__, A, B>
where
    A: Send,
    B: Sync,
    'a: 'lt_fut__,
    'b: 'lt_fut__,
    'x: 'lt_fut__,
{
    type IntoFuture = DoThingFuture<'a, 'b, 'x, 'lt_fut__, A, B, abs_cancel::NonCancellableToken>;
    type Output = Vec<<core::slice::Iter<'a, A> as Iterator>::Item>;
    fn into_future(self) -> Self::IntoFuture {
        DoThingFuture {
            params_: ::core::option::Option::Some(self),
            cancel_: ::core::option::Option::Some(abs_cancel::NonCancellableToken::new()),
            future_: ::core::option::Option::None,
        }
    }
}
impl<'a, 'b, 'x, 'lt_fut__, A, B> abs_cancel::TrMayCancel<'lt_fut__>
    for DoThingAsync<'a, 'b, 'x, 'lt_fut__, A, B>
where
    A: Send,
    B: Sync,
    'a: 'lt_fut__,
    'b: 'lt_fut__,
    'x: 'lt_fut__,
{
    type MayCancelFuture<'lt_mcf__, TyTok__>
        = DoThingFuture<'a, 'b, 'x, 'lt_mcf__, A, B, TyTok__>
    where
        'lt_mcf__: 'lt_fut__,
        Self: 'lt_mcf__,
        TyTok__: 'lt_mcf__ + abs_cancel::TrCancellationToken;
    type MayCancelOutput = Vec<<core::slice::Iter<'a, A> as Iterator>::Item>;
    fn may_cancel_with<TyTok__>(self, cancel: TyTok__) -> Self::MayCancelFuture<'lt_fut__, TyTok__>
    where
        TyTok__: 'lt_fut__ + abs_cancel::TrCancellationToken,
    {
        DoThingFuture {
            params_: ::core::option::Option::Some(self),
            cancel_: ::core::option::Option::Some(cancel),
            future_: ::core::option::Option::None,
        }
    }
}
impl<'a, 'b, 'x, 'lt_fut__, A, B, TyTok__> ::core::future::Future
    for DoThingFuture<'a, 'b, 'x, 'lt_fut__, A, B, TyTok__>
where
    A: Send,
    B: Sync,
    'a: 'lt_fut__,
    'b: 'lt_fut__,
    'x: 'lt_fut__,
    TyTok__: 'lt_fut__ + abs_cancel::TrCancellationToken,
{
    type Output = Vec<<core::slice::Iter<'a, A> as Iterator>::Item>;
    fn poll(
        self: ::core::pin::Pin<&mut Self>,
        cx: &mut ::core::task::Context<'_>,
    ) -> ::core::task::Poll<Self::Output> {
        let this = unsafe { ::core::pin::Pin::get_unchecked_mut(self) };
        loop {
            if let ::core::option::Option::Some(fut) = this.future_.as_mut() {
                let fut = unsafe { ::core::pin::Pin::new_unchecked(fut) };
                break ::core::future::Future::poll(fut, cx);
            }
            let (params, cancel) = match (this.params_.take(), this.cancel_.take()) {
                (::core::option::Option::Some(params), ::core::option::Option::Some(cancel)) => {
                    (params, cancel)
                }
                _ => unreachable!("gen_may_cancel_future: future 状态机不变量被破坏"),
            };
            this.future_ = ::core::option::Option::Some(<() as TrDoThingFutureFactory<
                'a,
                'b,
                'x,
                'lt_fut__,
                A,
                B,
                TyTok__,
            >>::make_future(params, cancel));
        }
    }
}
trait TrDoThingFutureFactory<'a, 'b, 'x, 'lt_fut__, A, B, TyTok__>
where
    A: Send,
    B: Sync,
    'a: 'lt_fut__,
    'b: 'lt_fut__,
    'x: 'lt_fut__,
    A: 'a,
    B: 'b,
    TyTok__: abs_cancel::TrCancellationToken,
{
    type MadeFuture: ::core::future::Future<Output = Vec<<core::slice::Iter<'a, A> as Iterator>::Item>>;
    fn make_future(
        params: DoThingAsync<'a, 'b, 'x, 'lt_fut__, A, B>,
        cancel: TyTok__,
    ) -> Self::MadeFuture;
}
impl<'a, 'b, 'x, 'lt_fut__, A, B, TyTok__>
    TrDoThingFutureFactory<'a, 'b, 'x, 'lt_fut__, A, B, TyTok__> for ()
where
    A: Send,
    B: Sync,
    'a: 'lt_fut__,
    'b: 'lt_fut__,
    'x: 'lt_fut__,
    A: 'a,
    B: 'b,
    TyTok__: abs_cancel::TrCancellationToken,
{
    type MadeFuture =
        impl ::core::future::Future<Output = Vec<<core::slice::Iter<'a, A> as Iterator>::Item>>;
    fn make_future(
        params: DoThingAsync<'a, 'b, 'x, 'lt_fut__, A, B>,
        cancel: TyTok__,
    ) -> Self::MadeFuture {
        let DoThingAsync(p0, p1, p2, p3, ..) = params;
        self::do_thing_async(p0, p1, p2, p3, cancel)
    }
}
```

### 展开规则

除 `{Prefix}Async` 是原样复制输入函数的类型、生命周期与泛型参数外，其他类型都额外携带
一个 `'lt_fut__` 生命周期与（必要时）一个 `TyTok__` 泛型参数。

- **可见性**：`{Prefix}Async` 与 `{Prefix}Future` 取「宏参数指定的类型访问级别」或
  「输入函数的可见性」；工厂 trait `Tr{Prefix}FutureFactory` 及其实现是私有的。
- **泛型参数**：用户声明的全部生命周期（保持顺序）→ `'lt_fut__` → 除取消令牌外的
  类型参数 → 常量参数（保持原顺序）。
- **where 子句**：用户谓词去掉所有提到取消令牌类型参数的部分，再对每个用户生命周期补
  `'l: 'lt_fut__`。工厂 trait 还会补上由**字段中的引用**隐含的 outlives（例如
  `&'a mut A` ⇒ `A: 'a`），因为工厂声明处的返回类型 WF 需要它们。
- **字段**：原函数「除最后一个参数外」的参数类型**逐字照抄**（不做任何生命周期替换），
  之后依次追加：
  1. `PhantomData<&'lt_fut__ ()>`；
  2. 对每个「没有出现在任何字段里」的用户生命周期 `'l`：`PhantomData<&'l ()>`；
  3. 对每个「没有出现在任何字段里」的类型参数 `Tp`：`PhantomData<fn() -> Tp>`。
- **构造函数**：额外生成一个 `impl {Prefix}Async` 块，其中 `new(...)` 的可见性取
  「宏参数 `new(...)` 指定的级别」或「类型的访问级别」；形参就是原函数「除取消令牌外」
  的参数；尾部的全部标记字段由它自动补齐，调用者不必再手写 `PhantomData`。
- **取消令牌**：按值保存在 `{Prefix}Future::cancel_` 中，首次 `poll` 时与原参数一起交给
  原函数；不可取消路径用 `abs_cancel::NonCancellableToken`。
- **不需要 `Unpin`**：`TrCancellationToken` 只保证 `Send + Sync + Clone`，生成的类型不会
  要求 `TyTok__: Unpin`。

## 已知限制

- 只能作用于**模块级自由函数**（`syn::ItemFn`），不能用于 `impl` / `trait` 里的关联函数。
- 若**返回类型**引入了「只能由返回类型满足」的 outlives 约束（例如返回
  `core::slice::Iter<'a, T>` 而 `T` 不出现在任何参数里），宏不会自动补 `T: 'a`；
  请在 where 子句里显式写出（规则 3）。
- 泛型参数上不支持内联约束（`<'a, T: Send, C>`），请一律写在 where 子句里。
- 不支持泛型参数默认值。
