//! `gen_mcf2`：为「可被优雅取消」的 `async fn` 生成配套类型的过程宏。
//!
//! 本 crate 是 `gen_mcf_macro::gen_may_cancel_future` 面向
//! `abs_cancel` v0.2 的重制版，内部改用 [`zyn`] 的模板、元素与诊断设施。
//!
//! 完整的使用规则、展开示例与已知限制见 crate 根目录的 `README.md`，
//! 设计决策见 `dev-notes/`。

use zyn::syn;

mod analyze_;
mod elements_;
mod model_;
mod query_;

use crate::elements_::*;
use crate::model_::GenModel;

/// 为 `async fn` 生成 `{Prefix}Async`、`{Prefix}Future` 以及相应的
/// `IntoFuture` / `TrMayCancel` / `Future` 实现。
///
/// # 使用约定
///
/// 0. 只能作用于 `async fn`；
/// 1. 显式声明所需生命周期，且不要用 `__` 结尾的标识符（宏保留该后缀）；
/// 2. 最后一个泛型**类型**参数是取消令牌类型，且在 where 子句中约束
///    `TrCancellationToken`；
/// 3. 用 where 子句声明全部约束（不要在泛型参数上写内联约束）；
/// 4. 函数的最后一个参数必须是该取消令牌类型，且按值传入。
///
/// # 生成条目的访问级别
///
/// 宏参数可以额外指定访问级别。前缀之后能按**任意顺序**给出至多一个**裸访问级别**
/// （作用于 `{Prefix}Async` / `{Prefix}Future`）与至多一个 `new(<访问级别>)`
/// （单独作用于 `{Prefix}Async::new`），两者都可省略：
///
/// - `#[gen_may_cancel_future(DoThing)]`：两者都与输入函数一致（默认）；
/// - `#[gen_may_cancel_future(DoThing, pub)]` / `pub(crate)` / `pub(super)` /
///   `pub(in path)`：类型按指定级别生成，`new` 跟随类型；
/// - `#[gen_may_cancel_future(DoThing, pub, new(pub(crate)))]`：类型是 `pub`，
///   `new` 却只在 crate 内可用 —— 「外部能命名这个类型，但只有本 crate 能构造它」；
/// - `#[gen_may_cancel_future(DoThing, new(pub(crate)))]`：只改 `new`，类型不变。
///
/// **类型**的访问级别指定得比输入函数更严格时会被拒绝（无法判定的 `pub(in path)`
/// 组合除外）；**`new`** 的访问级别不做任何校验，比类型更开放或更严格都由使用者负责。
///
/// 两处都只改变条目**自身**的可见性，条目的实际可达性还受外层模块可见性约束。
///
/// # Examples
///
/// ```ignore
/// use gen_mcf2::gen_may_cancel_future;
///
/// #[gen_may_cancel_future(DoThing, pub, new(pub(crate)))]
/// async fn do_thing_async<'a, A, C>(
///     a: &'a mut A,
///     cancel: C,
/// ) -> usize
/// where
///     A: Send,
///     C: abs_cancel::TrCancellationToken,
/// {
///     let _ = (a, cancel);
///     42
/// }
/// ```
///
/// 展开后得到 `DoThingAsync<'a, 'lt_fut__, A>` 与
/// `DoThingFuture<'a, 'lt_fut__, A, TyTok__>`：前者可 `.await`（不可取消），
/// 也可以先 `.may_cancel_with(token)` 再 `.await`。
#[zyn::attribute]
fn gen_may_cancel_future(
    #[zyn(input)] item: syn::ItemFn,
    args: zyn::TokenStream,
) -> zyn::TokenStream {
    let args = match analyze_::parse_args_(args) {
        Result::Ok(args) => args,
        // 属性体位于返回 `zyn::Output` 的闭包内，可以直接提前返回。
        Result::Err(diag) => return zyn::Output::from(diag),
    };

    expand_(&args, &item)
}

/// 纯函数形式的展开入口，便于单元测试直接调用。
pub(crate) fn expand_(args: &analyze_::MacroArgs_, item: &syn::ItemFn) -> zyn::Output {
    let model: GenModel = match analyze_::analyze_(args, item) {
        Result::Ok(model) => model,
        Result::Err(diag) => {
            return zyn::Output::from(diag);
        }
    };

    let input: zyn::Input = syn::Item::Fn(item.clone()).into();

    zyn::zyn! {
        {{ item }}
        @async_struct(model = model.clone())
        @async_new_impl(model = model.clone())
        @future_struct(model = model.clone())
        @into_future(model = model.clone())
        @may_cancel(model = model.clone())
        @future_impl(model = model.clone())
        @factory_trait(model = model.clone())
        @factory_impl(model = model.clone())
    }
}

#[cfg(test)]
mod tests_ {
    use super::*;
    use zyn::mark::Diagnostic;
    use zyn::syn::{self, parse_quote};

    /// README 中的示例函数签名。
    const README_EXAMPLE: &str = r#"
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
    "#;

    fn args_of_(prefix: &str) -> analyze_::MacroArgs_ {
        let prefix_ = zyn::format_ident!("{}", prefix);
        analyze_::MacroArgs_ {
            prefix_,
            vis_types_: None,
            vis_new_: None,
        }
    }

    fn args_with_vis_(prefix: &str, vis: &str) -> analyze_::MacroArgs_ {
        let prefix_ = zyn::format_ident!("{}", prefix);
        let vis_: syn::Visibility = syn::parse_str(vis).unwrap();
        analyze_::MacroArgs_ {
            prefix_,
            vis_types_: Some(vis_),
            vis_new_: None,
        }
    }

    fn expand_src_with_vis_(src: &str, prefix: &str, vis: &str) -> zyn::Output {
        let item: syn::ItemFn = syn::parse_str(src).unwrap();
        expand_(&args_with_vis_(prefix, vis), &item)
    }

    /// 用完整的宏参数字面量（如 `"F, pub, new(pub(crate))"`）走「解析 + 展开」全流程。
    ///
    /// 与 [`expand_src_with_vis_`] 不同，这里经过真实的 [`analyze_::parse_args_`]，
    /// 因此同时覆盖参数解析与展开两层的配合。
    fn expand_src_with_args_(src: &str, args: &str) -> zyn::Output {
        let item: syn::ItemFn = syn::parse_str(src).unwrap();
        let parsed = analyze_::parse_args_(syn::parse_str::<zyn::TokenStream>(args).unwrap())
            .expect("宏参数应当解析成功");
        expand_(&parsed, &item)
    }

    fn analyze_src_(src: &str, prefix: &str) -> Result<GenModel, Diagnostic> {
        let item: syn::ItemFn = syn::parse_str(src).unwrap();
        analyze_::analyze_(&args_of_(prefix), &item)
    }

    fn expand_src_(src: &str, prefix: &str) -> zyn::Output {
        let item: syn::ItemFn = syn::parse_str(src).unwrap();
        expand_(&args_of_(prefix), &item)
    }

    fn text_of_(output: &zyn::Output) -> String {
        output.tokens().to_string()
    }

    fn pred_strings_(preds: &[syn::WherePredicate]) -> Vec<String> {
        preds
            .iter()
            .map(|pred| zyn::quote::quote!(#pred).to_string())
            .collect()
    }

    /// 把「可选的访问级别」渲染成便于断言的无空格文本；`None` 渲染为 `<缺省>`。
    fn vis_text_(vis: &Option<syn::Visibility>) -> String {
        vis.as_ref().map_or_else(
            || "<缺省>".to_owned(),
            |vis| zyn::quote::quote!(#vis).to_string().replace(' ', ""),
        )
    }

    //-- 分析层 ------------------------------------------------------------

    /// 测试 README 示例的分析结果：泛型拆分、字段类型、where 子句拆分与 outlives。
    /// - 手段：解析示例函数并调用 `analyze_`，检查 `GenModel` 的各字段。
    /// - 判断：类型名/生命周期/类型参数与预期一致；取消令牌相关谓词被剔除；
    ///   生命周期约束被补上；工厂 outlives 含 `A: 'a`、`B: 'b`。
    #[test]
    fn analyze_readme_example() {
        let model = analyze_src_(README_EXAMPLE, "DoThing").unwrap();

        assert_eq!(model.fn_ident_.to_string(), "do_thing_async");
        assert_eq!(model.async_ident_.to_string(), "DoThingAsync");
        assert_eq!(model.future_ident_.to_string(), "DoThingFuture");
        assert_eq!(model.factory_ident_.to_string(), "TrDoThingFutureFactory");
        assert_eq!(model.lt_fut_.to_string(), "'lt_fut__");

        let lts: Vec<String> = model
            .user_lts_
            .iter()
            .map(|lt| lt.ident.to_string())
            .collect();
        assert_eq!(lts, ["a", "b", "x"]);

        let tps: Vec<String> = model.type_params_.iter().map(|tp| tp.to_string()).collect();
        assert_eq!(tps, ["A", "B"]);

        assert_eq!(model.field_tys_.len(), 4);
        assert!(model.const_params_.is_empty());
        assert!(model.marker_lts_.is_empty());
        assert!(model.marker_tys_.is_empty());

        let preds = pred_strings_(&model.preds_common_);
        assert!(preds.iter().any(|p| p == "A : Send"));
        assert!(preds.iter().any(|p| p == "B : Sync"));
        assert!(preds.iter().any(|p| p == "'a : 'lt_fut__"));
        assert!(preds.iter().any(|p| p == "'x : 'lt_fut__"));
        assert!(
            !preds.iter().any(|p| p.contains("TrCancellationToken")),
            "取消令牌相关谓词应被剔除，实际为 {preds:?}"
        );

        let factory = pred_strings_(&model.preds_factory_);
        assert!(factory.iter().any(|p| p == "A : 'a"), "{factory:?}");
        assert!(factory.iter().any(|p| p == "B : 'b"), "{factory:?}");
    }

    /// 测试「只在 where 子句出现」的生命周期会被补上 PhantomData 标记。
    /// - 手段：`'g` 只在 where 子句里出现，分析后检查 `marker_lts_`。
    /// - 判断：`marker_lts_` 恰好包含 `'g`。
    #[test]
    fn analyze_marks_where_only_lifetime() {
        let model = analyze_src_(
            r#"
            async fn f<'f, 'g, T, C>(x: &'f mut T, cancel: C) -> usize
            where
                'g: 'f,
                T: Send,
                C: abs_cancel::TrCancellationToken,
            { let _ = (x, cancel); 0 }
            "#,
            "F",
        )
        .unwrap();

        let marked: Vec<String> = model
            .marker_lts_
            .iter()
            .map(|lt| lt.ident.to_string())
            .collect();
        assert_eq!(marked, ["g"]);
    }

    /// 测试「只在 where 子句出现」的类型参数会被补上 PhantomData 标记。
    /// - 手段：`T` 不出现在任何参数类型里，分析后检查 `marker_tys_`。
    /// - 判断：`marker_tys_` 恰好包含 `T`，且 `T` 仍是生成类型的泛型参数。
    #[test]
    fn analyze_marks_unused_type_param() {
        let model = analyze_src_(
            r#"
            async fn f<'f, T, C>(x: &'f mut u8, cancel: C) -> usize
            where
                T: Default,
                C: abs_cancel::TrCancellationToken,
            { let _ = (x, cancel); 0 }
            "#,
            "F",
        )
        .unwrap();

        let marked: Vec<String> = model.marker_tys_.iter().map(|tp| tp.to_string()).collect();
        assert_eq!(marked, ["T"]);
    }

    /// 测试常量泛型参数被原样保留。
    /// - 手段：`const N: usize` 出现在取消令牌参数之前。
    /// - 判断：`const_params_` 长度为 1，且类型参数列表为空。
    #[test]
    fn analyze_keeps_const_param() {
        let model = analyze_src_(
            r#"
            async fn f<'f, const N: usize, C>(arr: [u8; N], cancel: C) -> usize
            where
                C: abs_cancel::TrCancellationToken,
            { let _ = (arr, cancel); N }
            "#,
            "F",
        )
        .unwrap();

        assert_eq!(model.const_params_.len(), 1);
        assert_eq!(model.const_params_[0].ident.to_string(), "N");
        assert!(model.type_params_.is_empty());
    }

    //-- 宏参数 ------------------------------------------------------------

    /// 测试宏参数解析：前缀 + 可选的访问级别。
    /// - 手段：把若干参数文本交给 `parse_args_`。
    /// - 判断：`DoThing` / `DoThing,` / `DoThing, pub(crate)` 等解析成功且字段正确；
    ///   空参数、参数过多、缺少 `pub`、缺少逗号等一律报错。
    #[test]
    fn parse_args_accepts_prefix_and_optional_vis() {
        let parse =
            |src: &str| analyze_::parse_args_(syn::parse_str::<zyn::TokenStream>(src).unwrap());

        // 只有前缀：访问级别缺省
        let args = parse("DoThing").unwrap();
        assert_eq!(args.prefix_.to_string(), "DoThing");
        assert!(args.vis_types_.is_none());
        assert!(args.vis_new_.is_none());

        // 允许尾随逗号
        let args = parse("DoThing,").unwrap();
        assert!(args.vis_types_.is_none());
        assert!(args.vis_new_.is_none());

        // 各种访问级别
        for vis in ["pub", "pub(crate)", "pub(super)", "pub(in crate::a)"] {
            let args = parse(&format!("DoThing, {vis}")).unwrap();
            assert!(args.vis_types_.is_some(), "vis = {vis}");
            assert!(args.vis_new_.is_none(), "vis = {vis}");
        }

        // 非法参数
        assert!(parse("").is_err());
        assert!(parse("DoThing, Extra, More").is_err());
        assert!(parse("DoThing DoOther").is_err());

        // 访问级别必须以 `pub` 开头（`Visibility::parse` 会静默返回私有）
        let Err(err) = parse("DoThing, crate") else {
            unreachable!("`DoThing, crate` 应当报错")
        };
        zyn::assert_diagnostic_error!(zyn::Output::from(err), "必须以 `pub` 开头");
    }

    /// 测试 `new(<访问级别>)` 宏参数的解析：单独使用、与类型级别并用、顺序无关。
    /// - 手段：把 `DoThing, pub, new(pub(crate))` 等参数文本交给 `parse_args_`。
    /// - 判断：`vis_types_` 与 `vis_new_` 分别取到对应的访问级别；两种书写顺序结果相同；
    ///   只写 `new(...)` 时 `vis_types_` 仍为 `None`；允许尾随逗号。
    #[test]
    fn parse_args_accepts_new_vis() {
        let parse =
            |src: &str| analyze_::parse_args_(syn::parse_str::<zyn::TokenStream>(src).unwrap());

        // 类型级别 + `new` 级别
        let args = parse("DoThing, pub, new(pub(crate))").unwrap();
        assert_eq!(vis_text_(&args.vis_types_), "pub");
        assert_eq!(vis_text_(&args.vis_new_), "pub(crate)");

        // 顺序无关：先写 `new(...)` 再写类型级别，结果与上面一致
        let swapped = parse("DoThing, new(pub(crate)), pub").unwrap();
        assert_eq!(vis_text_(&swapped.vis_types_), "pub");
        assert_eq!(vis_text_(&swapped.vis_new_), "pub(crate)");

        // 只写 `new(...)`：类型级别保持缺省
        let only_new = parse("DoThing, new(pub(super))").unwrap();
        assert_eq!(vis_text_(&only_new.vis_types_), "<缺省>");
        assert_eq!(vis_text_(&only_new.vis_new_), "pub(super)");

        // 允许尾随逗号
        let trailing = parse("DoThing, pub, new(pub(crate)),").unwrap();
        assert!(trailing.vis_types_.is_some() && trailing.vis_new_.is_some());
    }

    /// 测试 `new(...)` 参数的各类写法错误都会被明确拒绝。
    /// - 手段：构造重复指定、括号内不是 `pub`、缺括号、括号内多写、括号内未写完等参数文本。
    /// - 判断：每种情况都返回 `Err`，且诊断文案指出问题所在（不泄漏 syn 的英文默认信息）。
    #[test]
    fn parse_args_rejects_bad_new_vis() {
        let parse =
            |src: &str| analyze_::parse_args_(syn::parse_str::<zyn::TokenStream>(src).unwrap());

        // 重复指定 `new` 的访问级别
        let Err(err) = parse("DoThing, new(pub), new(pub(crate))") else {
            unreachable!("重复指定 `new` 的访问级别应当报错")
        };
        zyn::assert_diagnostic_error!(zyn::Output::from(err), "`new` 的访问级别重复指定");

        // 重复指定类型的访问级别（新语法下不再报「参数过多」，而是明确报重复）
        let Err(err) = parse("DoThing, pub, pub(crate)") else {
            unreachable!("重复指定类型的访问级别应当报错")
        };
        zyn::assert_diagnostic_error!(zyn::Output::from(err), "类型的访问级别重复指定");

        // 括号内的访问级别没有以 `pub` 开头
        let Err(err) = parse("DoThing, new(crate)") else {
            unreachable!("`new(crate)` 应当报错")
        };
        zyn::assert_diagnostic_error!(zyn::Output::from(err), "`new` 的访问级别必须以 `pub` 开头");

        // 缺少括号：`new` 不是关键字，必须显式写成括号形式
        let Err(err) = parse("DoThing, new pub(crate)") else {
            unreachable!("`new pub(crate)` 应当报错")
        };
        zyn::assert_diagnostic_error!(zyn::Output::from(err), "必须写成括号形式");

        // 括号内多写了内容
        let Err(err) = parse("DoThing, new(pub, pub(crate))") else {
            unreachable!("`new(pub, pub(crate))` 应当报错")
        };
        zyn::assert_diagnostic_error!(zyn::Output::from(err), "只能写一个访问级别");

        // 括号内什么都没写
        assert!(parse("DoThing, new()").is_err());
    }

    //-- 诊断 --------------------------------------------------------------

    /// 测试非 `async fn` 会被拒绝并给出明确诊断。
    /// - 手段：对同步函数调用 `expand_`。
    /// - 判断：输出包含 "只能用于 `async fn`" 的错误诊断。
    #[test]
    fn rejects_non_async_fn() {
        let out = expand_src_(
            r#"
            fn f<'f, T, C>(x: &'f mut T, cancel: C) -> usize
            where
                C: abs_cancel::TrCancellationToken,
            { let _ = (x, cancel); 0 }
            "#,
            "F",
        );
        zyn::assert_diagnostic_error!(out, "只能用于 `async fn`");
    }

    /// 测试缺少 where 子句会被拒绝。
    /// - 手段：构造没有 where 子句的异步函数。
    /// - 判断：输出包含 "where 子句" 的错误诊断。
    #[test]
    fn rejects_missing_where_clause() {
        let out = expand_src_(
            r#"async fn f<'f, T, C>(x: &'f mut T, cancel: C) -> usize { let _ = (x, cancel); 0 }"#,
            "F",
        );
        zyn::assert_diagnostic_error!(out, "where 子句");
    }

    /// 测试最后一个参数类型不是取消令牌类型参数时会被拒绝。
    /// - 手段：最后一个参数写成 `&mut C`。
    /// - 判断：输出包含 "按值传入" 的错误诊断。
    #[test]
    fn rejects_reference_cancel_param() {
        let out = expand_src_(
            r#"
            async fn f<'f, T, C>(x: &'f mut T, cancel: &'f mut C) -> usize
            where
                C: abs_cancel::TrCancellationToken,
            { let _ = (x, cancel); 0 }
            "#,
            "F",
        );
        zyn::assert_diagnostic_error!(out, "按值传入");
    }

    /// 测试取消令牌类型参数缺少 `TrCancellationToken` 约束时会被拒绝。
    /// - 手段：where 子句只约束 `C: Clone`。
    /// - 判断：输出包含 "TrCancellationToken" 的错误诊断。
    #[test]
    fn rejects_missing_cancellable_bound() {
        let out = expand_src_(
            r#"
            async fn f<'f, T, C>(x: &'f mut T, cancel: C) -> usize
            where
                C: Clone,
            { let _ = (x, cancel); 0 }
            "#,
            "F",
        );
        zyn::assert_diagnostic_error!(out, "TrCancellationToken");
    }

    /// 测试泛型参数上的内联约束会被拒绝。
    /// - 手段：写成 `<'f, T: Send, C>`。
    /// - 判断：输出包含 "内联约束" 的错误诊断。
    #[test]
    fn rejects_inline_bound() {
        let out = expand_src_(
            r#"
            async fn f<'f, T: Send, C>(x: &'f mut T, cancel: C) -> usize
            where
                C: abs_cancel::TrCancellationToken,
            { let _ = (x, cancel); 0 }
            "#,
            "F",
        );
        zyn::assert_diagnostic_error!(out, "内联约束");
    }

    /// 测试以 `__` 结尾的标识符会被拒绝（该后缀由宏保留）。
    /// - 手段：生命周期写成 `'f__`。
    /// - 判断：输出包含 "`__` 结尾" 的错误诊断。
    #[test]
    fn rejects_reserved_suffix() {
        let out = expand_src_(
            r#"
            async fn f<'f__, T, C>(x: &'f__ mut T, cancel: C) -> usize
            where
                C: abs_cancel::TrCancellationToken,
            { let _ = (x, cancel); 0 }
            "#,
            "F",
        );
        zyn::assert_diagnostic_error!(out, "`__` 结尾");
    }

    /// 测试取消令牌类型参数出现在其它参数里会被拒绝。
    /// - 手段：额外参数写成 `other: C`。
    /// - 判断：输出包含 "不能出现在其它参数类型中" 的错误诊断。
    #[test]
    fn rejects_cancel_type_in_other_param() {
        let out = expand_src_(
            r#"
            async fn f<'f, T, C>(x: &'f mut T, other: C, cancel: C) -> usize
            where
                C: abs_cancel::TrCancellationToken,
            { let _ = (x, other, cancel); 0 }
            "#,
            "F",
        );
        zyn::assert_diagnostic_error!(out, "不能出现在其它参数类型中");
    }

    //-- 展开层 ------------------------------------------------------------

    /// 测试 README 示例的展开包含全部预期条目，且不含被否决的旧设计。
    /// - 手段：展开示例并检查生成 token 文本。
    /// - 判断：含原始函数、两个结构体、三个 impl、工厂 trait/impl 与
    ///   `PhantomData` 标记；不含 `Unpin`、`MaybeUninit`。
    #[test]
    fn expands_readme_example() {
        let out = expand_src_(README_EXAMPLE, "DoThing");
        let text = text_of_(&out);

        assert!(text.contains("async fn do_thing_async"), "{text}");
        assert!(text.contains("struct DoThingAsync"), "{text}");
        assert!(text.contains("struct DoThingFuture"), "{text}");
        assert!(text.contains(":: core :: future :: IntoFuture"), "{text}");
        assert!(
            text.contains("abs_cancel :: TrMayCancel < 'lt_fut__ >"),
            "{text}"
        );
        assert!(
            text.contains(":: core :: future :: Future for DoThingFuture"),
            "{text}"
        );
        assert!(text.contains("trait TrDoThingFutureFactory"), "{text}");
        assert!(text.contains(":: core :: marker :: PhantomData"), "{text}");
        assert!(text.contains("A : 'a"), "{text}");
        assert!(text.contains("p0 , p1 , p2 , p3"), "{text}");

        assert!(!text.contains("Unpin"), "不应出现 `Unpin`：{text}");
        assert!(
            !text.contains("MaybeUninit"),
            "不应出现 `MaybeUninit`：{text}"
        );
        assert!(!text.contains("Either"), "不应出现 `Either__`：{text}");
        assert!(
            !text.contains("FutureState"),
            "不应生成 `FutureState`：{text}"
        );
    }

    /// 测试 `{Prefix}Async::new` 构造函数：可见性、形参名与标记字段补齐。
    /// - 手段：展开 README 示例，检查生成文本中的 `new` 定义。
    /// - 判断：可见性与原函数一致；形参沿用原参数名；构造体补齐 `PhantomData`。
    #[test]
    fn expands_async_new() {
        let out = expand_src_(README_EXAMPLE, "DoThing");
        let text = text_of_(&out);

        assert!(text.contains("pub (crate) fn new"), "{text}");
        assert!(
            text.contains(
                "fn new (a : & 'a mut A , b : & 'b mut B , l : usize , x : core :: slice :: Iter < 'x , A > ,) -> Self"
            ),
            "{text}"
        );
        assert!(
            text.contains("DoThingAsync (a , b , l , x , :: core :: marker :: PhantomData ,)"),
            "{text}"
        );
    }

    /// 测试无法沿用原参数名（`_` 等模式）时 `new` 的形参回退为 `p{i}`。
    /// - 手段：把第一个参数写成 `_`，展开后检查 `new` 的形参名。
    /// - 判断：出现 `fn new (p0 : & 'f mut T ,) -> Self`。
    #[test]
    fn expands_async_new_falls_back_for_wild_param() {
        let out = expand_src_(
            r#"
            async fn f<'f, T, C>(_: &'f mut T, cancel: C) -> usize
            where
                C: abs_cancel::TrCancellationToken,
            { let _ = cancel; 0 }
            "#,
            "F",
        );
        let text = text_of_(&out);
        assert!(
            text.contains("fn new (p0 : & 'f mut T ,) -> Self"),
            "{text}"
        );
    }

    /// 测试用户参数恰好命名为 `cancel` 时不会污染工厂内部的令牌绑定。
    /// - 手段：第一个参数命名为 `cancel`，最后一个参数才是取消令牌。
    /// - 判断：工厂仍用 `p0` 解构，并把生成的 `cancel` 形参作为令牌传回原函数。
    #[test]
    fn user_param_named_cancel_is_safe() {
        let out = expand_src_(
            r#"
            async fn f<'f, T, C>(cancel: &'f mut T, token: C) -> usize
            where
                C: abs_cancel::TrCancellationToken,
            { let _ = (cancel, token); 0 }
            "#,
            "F",
        );
        let text = text_of_(&out);
        assert!(text.contains("let FAsync (p0 , ..) = params"), "{text}");
        assert!(text.contains("self :: f (p0 , cancel)"), "{text}");
    }

    /// 测试不指定访问级别时，生成条目默认沿用输入函数的访问级别。
    /// - 手段：展开 `pub(crate)` 的 README 示例。
    /// - 判断：`Async` / `Future` / `new` 都是 `pub(crate)`。
    #[test]
    fn defaults_to_fn_visibility() {
        let out = expand_src_(README_EXAMPLE, "DoThing");
        let text = text_of_(&out);

        assert!(text.contains("pub (crate) struct DoThingAsync"), "{text}");
        assert!(text.contains("pub (crate) struct DoThingFuture"), "{text}");
        assert!(text.contains("pub (crate) fn new"), "{text}");
    }

    /// 测试可以用宏参数把生成条目的访问级别放宽到比输入函数更开放。
    /// - 手段：对私有 `async fn` 分别指定 `pub` 与 `pub(crate)` 展开。
    /// - 判断：生成条目带上了指定级别，而原函数本身仍然保持私有。
    #[test]
    fn expands_with_widened_visibility() {
        let src = r#"
            async fn f<'f, T, C>(x: &'f mut T, cancel: C) -> usize
            where
                C: abs_cancel::TrCancellationToken,
            { let _ = (x, cancel); 0 }
        "#;

        let text = text_of_(&expand_src_with_vis_(src, "F", "pub"));
        assert!(text.contains("pub struct FAsync"), "{text}");
        assert!(text.contains("pub struct FFuture"), "{text}");
        assert!(text.contains("pub fn new"), "{text}");
        assert!(text.contains("async fn f < 'f , T , C >"), "{text}");
        assert!(!text.contains("pub async fn f"), "{text}");

        let text = text_of_(&expand_src_with_vis_(src, "F", "pub(crate)"));
        assert!(text.contains("pub (crate) struct FAsync"), "{text}");
        assert!(text.contains("pub (crate) fn new"), "{text}");
    }

    /// 测试指定比输入函数更严格的访问级别会被拒绝。
    /// - 手段：`pub` 的函数指定 `pub(crate)`；`pub(crate)` 的函数指定 `pub(self)`。
    /// - 判断：两种情况都给出 "不能比输入函数更严格" 的错误诊断。
    #[test]
    fn rejects_narrowed_visibility() {
        let pub_fn = r#"
            pub async fn f<'f, T, C>(x: &'f mut T, cancel: C) -> usize
            where
                C: abs_cancel::TrCancellationToken,
            { let _ = (x, cancel); 0 }
        "#;
        let out = expand_src_with_vis_(pub_fn, "F", "pub(crate)");
        zyn::assert_diagnostic_error!(out, "不能比输入函数更严格");

        let crate_fn = r#"
            pub(crate) async fn f<'f, T, C>(x: &'f mut T, cancel: C) -> usize
            where
                C: abs_cancel::TrCancellationToken,
            { let _ = (x, cancel); 0 }
        "#;
        let out = expand_src_with_vis_(crate_fn, "F", "pub(self)");
        zyn::assert_diagnostic_error!(out, "不能比输入函数更严格");
    }

    /// 测试无法判定开放程度的组合（双方都是 `pub(in path)`）不会被误报。
    /// - 手段：`pub(in crate::a)` 的函数指定 `pub(in crate::b)`。
    /// - 判断：展开成功且生成条目带上指定级别。
    #[test]
    fn allows_undecidable_visibility_combinations() {
        let src = r#"
            pub(in crate::a) async fn f<'f, T, C>(x: &'f mut T, cancel: C) -> usize
            where
                C: abs_cancel::TrCancellationToken,
            { let _ = (x, cancel); 0 }
        "#;
        let out = expand_src_with_vis_(src, "F", "pub(in crate::b)");
        let text = text_of_(&out);
        assert!(text.contains("pub (in crate :: b) struct FAsync"), "{text}");
    }

    /// 测试类型与 `new` 可以取到**不同**的访问级别（本次新增能力的核心用例）。
    /// - 手段：对私有 `async fn` 用 `(F, pub, new(pub(crate)))` 走完整的解析 + 展开。
    /// - 判断：`FAsync` / `FFuture` 是 `pub`，而 `new` 是 `pub(crate)`；原函数仍私有。
    #[test]
    fn expands_with_split_visibility() {
        let src = r#"
            async fn f<'f, T, C>(x: &'f mut T, cancel: C) -> usize
            where
                C: abs_cancel::TrCancellationToken,
            { let _ = (x, cancel); 0 }
        "#;

        let out = expand_src_with_args_(src, "F, pub, new(pub(crate))");
        let text = text_of_(&out);

        assert!(text.contains("pub struct FAsync"), "{text}");
        assert!(text.contains("pub struct FFuture"), "{text}");
        assert!(text.contains("pub (crate) fn new"), "{text}");
        assert!(!text.contains("pub fn new"), "`new` 不应是 `pub`：{text}");
        assert!(text.contains("async fn f < 'f , T , C >"), "{text}");
        assert!(!text.contains("pub async fn f"), "{text}");
    }

    /// 测试只写 `new(...)` 时，类型级别仍然沿用输入函数的访问级别。
    /// - 手段：对私有 `async fn` 用 `(F, new(pub(crate)))` 展开。
    /// - 判断：`FAsync` 保持私有（无任何 `pub` 前缀），只有 `new` 是 `pub(crate)`。
    #[test]
    fn new_vis_alone_keeps_type_vis_from_fn() {
        let src = r#"
            async fn f<'f, T, C>(x: &'f mut T, cancel: C) -> usize
            where
                C: abs_cancel::TrCancellationToken,
            { let _ = (x, cancel); 0 }
        "#;

        let out = expand_src_with_args_(src, "F, new(pub(crate))");
        let text = text_of_(&out);

        assert!(text.contains("pub (crate) fn new"), "{text}");
        assert!(text.contains("struct FAsync"), "{text}");
        assert!(!text.contains("pub struct FAsync"), "{text}");
        assert!(!text.contains("pub (crate) struct FAsync"), "{text}");
    }

    /// 测试 `new` 的访问级别**不做**任何校验：比类型更开放也照常展开。
    /// - 手段：指定 `(F, pub(crate), new(pub))`，即构造函数比类型更开放。
    /// - 判断：展开成功（不报「不能比输入函数更严格」之类的错误），文本里同时出现
    ///   `pub (crate) struct FAsync` 与 `pub fn new`。
    #[test]
    fn new_vis_is_not_validated() {
        let src = r#"
            async fn f<'f, T, C>(x: &'f mut T, cancel: C) -> usize
            where
                C: abs_cancel::TrCancellationToken,
            { let _ = (x, cancel); 0 }
        "#;

        let out = expand_src_with_args_(src, "F, pub(crate), new(pub)");
        let text = text_of_(&out);

        assert!(text.contains("pub (crate) struct FAsync"), "{text}");
        assert!(text.contains("pub fn new"), "{text}");
    }

    /// 测试拆分访问级别后，类型级别「不得比输入函数更严格」的校验依然生效。
    /// - 手段：`pub` 的函数指定 `(F, pub(crate), new(pub(crate)))`。
    /// - 判断：仍然给出 "不能比输入函数更严格" 的错误诊断（`new` 不参与校验，
    ///   但类型级别照旧受约束）。
    #[test]
    fn split_visibility_still_rejects_narrowed_type() {
        let src = r#"
            pub async fn f<'f, T, C>(x: &'f mut T, cancel: C) -> usize
            where
                C: abs_cancel::TrCancellationToken,
            { let _ = (x, cancel); 0 }
        "#;
        let out = expand_src_with_args_(src, "F, pub(crate), new(pub(crate))");
        zyn::assert_diagnostic_error!(out, "不能比输入函数更严格");
    }

    /// 测试 where-only 生命周期会生成 `PhantomData<&'g ()>` 标记字段。
    /// - 手段：展开 `'g` 只在 where 子句出现的函数。
    /// - 判断：token 文本包含 `PhantomData < & 'g () >`。
    #[test]
    fn expands_where_only_lifetime_marker() {
        let out = expand_src_(
            r#"
            async fn f<'f, 'g, T, C>(x: &'f mut T, cancel: C) -> usize
            where
                'g: 'f,
                T: Send,
                C: abs_cancel::TrCancellationToken,
            { let _ = (x, cancel); 0 }
            "#,
            "F",
        );
        let text = text_of_(&out);
        assert!(text.contains("PhantomData < & 'g () >"), "{text}");
    }

    /// 测试未使用的类型参数会生成 `PhantomData<fn() -> T>` 标记字段。
    /// - 手段：展开 `T` 不出现在任何参数类型里的函数。
    /// - 判断：token 文本包含 `PhantomData < fn () -> T >`。
    #[test]
    fn expands_unused_type_param_marker() {
        let out = expand_src_(
            r#"
            async fn f<'f, T, C>(x: &'f mut u8, cancel: C) -> usize
            where
                T: Default,
                C: abs_cancel::TrCancellationToken,
            { let _ = (x, cancel); 0 }
            "#,
            "F",
        );
        let text = text_of_(&out);
        assert!(text.contains("PhantomData < fn () -> T >"), "{text}");
    }

    /// 测试泛型参数顺序：生命周期在前，随后是类型参数与常量参数。
    /// - 手段：展开 `<'f, const N: usize, C>`。
    /// - 判断：`FAsync` 的泛型参数为 `<'f, 'lt_fut__, const N: usize>`。
    #[test]
    fn expands_const_param_position() {
        let out = expand_src_(
            r#"
            async fn f<'f, const N: usize, C>(arr: [u8; N], cancel: C) -> usize
            where
                C: abs_cancel::TrCancellationToken,
            { let _ = (arr, cancel); N }
            "#,
            "F",
        );
        let text = text_of_(&out);
        assert!(
            text.contains("struct FAsync < 'f , 'lt_fut__ , const N : usize , >"),
            "{text}"
        );
    }

    /// 测试展开结果可以被重新解析为合法的 Rust 语法（粗粒度语法检查）。
    /// - 手段：把展开结果按 `syn::File` 解析。
    /// - 判断：默认形式与拆分访问级别的形式都解析成功。
    #[test]
    fn expansion_is_parsable() {
        let out = expand_src_(README_EXAMPLE, "DoThing");
        let parsed = syn::parse2::<syn::File>(out.tokens().clone());
        assert!(parsed.is_ok(), "{:?}", parsed.err());

        let out = expand_src_with_args_(README_EXAMPLE, "DoThing, pub, new(pub(crate))");
        let parsed = syn::parse2::<syn::File>(out.tokens().clone());
        assert!(parsed.is_ok(), "{:?}", parsed.err());
    }

    /// 测试 `parse_quote!` 形式的 where 谓词构造与预期一致（回归保护）。
    /// - 手段：比较 `lifetime_bound_pred_` 的 `quote` 文本。
    /// - 判断：等于 `'a : 'lt_fut__`。
    #[test]
    fn builds_lifetime_bound_predicate() {
        let lt = syn::Lifetime::new("'a", zyn::Span::call_site());
        let bound = syn::Lifetime::new("'lt_fut__", zyn::Span::call_site());
        let pred: syn::WherePredicate = parse_quote!(#lt: #bound);
        assert_eq!(
            pred_strings_(&[pred]),
            pred_strings_(&[query_::lifetime_bound_pred_(&lt, &bound)])
        );
    }
}
