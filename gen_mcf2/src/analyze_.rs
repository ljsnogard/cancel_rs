//! 对输入函数签名做分析与校验，产出 [`GenModel`]。
//!
//! 这一层是纯函数：不接触 `proc_macro` / `zyn` 的展开上下文，因此可以直接
//! 用单元测试覆盖。

use zyn::mark::{Diagnostic, error};
use zyn::syn;
use zyn::syn::spanned::Spanned;

use crate::model_::GenModel;
use crate::query_;

/// `{Prefix}Async` / `{Prefix}Future` 中代表 future 自身的生命周期名。
const K_LT_FUT: &str = "'lt_fut__";
/// `TrMayCancel` 的 GAT 中代表 future 自身的生命周期名。
const K_LT_MCF: &str = "'lt_mcf__";

/// `#[gen_may_cancel_future(...)]` 的参数。
pub(crate) struct MacroArgs_ {
    /// 生成类型的前缀标识符。
    pub(crate) prefix_: syn::Ident,
    /// 生成**类型**（`{Prefix}Async` / `{Prefix}Future`）的访问级别；
    /// 不指定时沿用输入函数的访问级别。
    pub(crate) vis_types_: Option<syn::Visibility>,
    /// `{Prefix}Async::new` 的访问级别；不指定时沿用 [`Self::vis_types_`]。
    ///
    /// 这一项**不参与任何校验** —— 比类型更开放或更严格都由使用者负责。
    pub(crate) vis_new_: Option<syn::Visibility>,
}

/// 解析宏参数。
///
/// 约定：`#[gen_may_cancel_future(DoThing)]`、
/// `#[gen_may_cancel_future(DoThing, pub(crate))]` 或
/// `#[gen_may_cancel_future(DoThing, pub, new(pub(crate)))]`。
///
/// 前缀之后可以按**任意顺序**给出至多一个裸访问级别（作用于生成类型）与至多一个
/// `new(<访问级别>)`（单独作用于 `{Prefix}Async::new`）；两者都可省略。
pub(crate) fn parse_args_(args: zyn::TokenStream) -> Result<MacroArgs_, Diagnostic> {
    if args.is_empty() {
        return Err(diag_(
            "`#[gen_may_cancel_future]` 需要且只需要一个标识符作为生成类型的前缀，例如 `#[gen_may_cancel_future(DoThing)]`",
            proc_macro2::Span::call_site(),
        ));
    }

    let parser = |input: syn::parse::ParseStream<'_>| -> syn::Result<MacroArgs_> {
        let prefix_: syn::Ident = input.parse()?;
        let mut vis_types_ = None;
        let mut vis_new_ = None;

        while input.peek(syn::Token![,]) {
            input.parse::<syn::Token![,]>()?;
            // 允许尾随逗号
            if input.is_empty() {
                break;
            }

            if peek_new_spec_(input) {
                if vis_new_.is_some() {
                    return Err(input.error("`new` 的访问级别重复指定"));
                }
                vis_new_ = Some(parse_new_vis_(input)?);
                continue;
            }

            let vis: syn::Visibility = input.parse()?;
            // `Visibility::parse` 在看不到 `pub` 时会静默返回 `Inherited` 且不消费 token。
            if matches!(vis, syn::Visibility::Inherited) {
                return Err(input.error(
                    "访问级别必须以 `pub` 开头，例如 `pub` / `pub(crate)` / `pub(super)` / `pub(in path)`；\
                     若要单独指定构造函数的访问级别，请写成 `new(pub(crate))`",
                ));
            }
            if vis_types_.is_some() {
                return Err(input.error("类型的访问级别重复指定"));
            }
            vis_types_ = Some(vis);
        }

        if !input.is_empty() {
            return Err(input.error(
                "参数过多：应为 `(前缀)`、`(前缀, 访问级别如 pub / pub(crate))` 或 \
                 `(前缀, 访问级别, new(访问级别))`",
            ));
        }

        Ok(MacroArgs_ {
            prefix_,
            vis_types_,
            vis_new_,
        })
    };

    syn::parse::Parser::parse2(parser, args).map_err(Diagnostic::from)
}

/// 当前位置是否是 `new(...)` 形式的构造函数访问级别说明。
///
/// `new` 不是关键字，因此只能先窥探标识符再判断 —— 不能交给
/// [`syn::Visibility::parse`]，那样它会把 `new` 当作「看不到 `pub`」而静默返回
/// `Inherited`。
fn peek_new_spec_(input: syn::parse::ParseStream<'_>) -> bool {
    input
        .fork()
        .parse::<syn::Ident>()
        .is_ok_and(|ident| ident == "new")
}

/// 解析 `new(<访问级别>)`。
fn parse_new_vis_(input: syn::parse::ParseStream<'_>) -> syn::Result<syn::Visibility> {
    input.parse::<syn::Ident>()?;

    if !input.peek(syn::token::Paren) {
        return Err(input.error("`new` 的访问级别必须写成括号形式，例如 `new(pub(crate))`"));
    }
    let content;
    syn::parenthesized!(content in input);

    let vis: syn::Visibility = content.parse()?;
    if matches!(vis, syn::Visibility::Inherited) {
        return Err(
            content.error("`new` 的访问级别必须以 `pub` 开头，例如 `new(pub)` / `new(pub(crate))`")
        );
    }
    if !content.is_empty() {
        return Err(content.error("`new(...)` 里只能写一个访问级别，例如 `new(pub(crate))`"));
    }

    Ok(vis)
}

fn diag_(message: impl Into<String>, span: proc_macro2::Span) -> Diagnostic {
    error(message).span(span).build()
}

/// 访问级别的「开放程度」粗略排序；`None` 表示无法与其它受限级别比较。
fn vis_rank_(vis: &syn::Visibility) -> Option<u8> {
    match vis {
        syn::Visibility::Public(_) => Some(3),
        syn::Visibility::Inherited => Some(0),
        syn::Visibility::Restricted(restricted) => {
            let path = &restricted.path;
            if path.is_ident("self") {
                Some(0)
            } else if path.is_ident("super") {
                Some(1)
            } else if path.is_ident("crate") {
                Some(2)
            } else {
                None
            }
        }
    }
}

fn vis_text_(vis: &syn::Visibility) -> String {
    match vis {
        syn::Visibility::Inherited => "私有".to_owned(),
        other => zyn::quote::quote!(#other).to_string().replace(' ', ""),
    }
}

/// 判断生成**类型**指定的访问级别是否**可以证明**比输入函数更严格。
///
/// 生成类型只允许放宽或保持访问级别；无法判定的组合（例如双方都是
/// `pub(in path)`）一律放行，避免误报。
///
/// 注意：`{Prefix}Async::new` 的访问级别（[`MacroArgs_::vis_new_`]）不经过本函数，
/// 有意不做校验。
pub(crate) fn vis_narrowing_error_(
    over: &syn::Visibility,
    base: &syn::Visibility,
) -> Option<Diagnostic> {
    let narrower = if matches!(base, syn::Visibility::Public(_)) {
        !matches!(over, syn::Visibility::Public(_))
    } else {
        match (vis_rank_(over), vis_rank_(base)) {
            (Some(over_rank), Some(base_rank)) => over_rank < base_rank,
            _ => false,
        }
    };

    narrower.then(|| {
        diag_(
            format!(
                "生成条目的访问级别不能比输入函数更严格（输入函数为 `{}`，指定为 `{}`）；不指定则默认与输入函数一致",
                vis_text_(base),
                vis_text_(over),
            ),
            over.span(),
        )
    })
}

/// 分析函数签名，构造展开所需的全部信息。
///
/// 校验失败的场合返回带 span 的诊断，交由调用方汇入 `zyn` 的诊断流。
pub(crate) fn analyze_(args: &MacroArgs_, item: &syn::ItemFn) -> Result<GenModel, Diagnostic> {
    let prefix = &args.prefix_;
    let sig = &item.sig;

    // 生成类型只允许放宽访问级别。
    if let Some(vis) = &args.vis_types_
        && let Some(err) = vis_narrowing_error_(vis, &item.vis)
    {
        return Err(err);
    }

    // 构造函数的访问级别不做任何校验（包括不要求它比类型更严格）：使用者自负。
    let vis_types = args.vis_types_.clone().unwrap_or_else(|| item.vis.clone());
    let vis_new = args.vis_new_.clone().unwrap_or_else(|| vis_types.clone());

    if sig.asyncness.is_none() {
        return Err(diag_(
            "`#[gen_may_cancel_future]` 只能用于 `async fn`",
            sig.ident.span(),
        ));
    }

    //-- 1. 泛型参数 ---------------------------------------------------------
    let generics = &sig.generics;
    let Some(where_clause) = &generics.where_clause else {
        return Err(diag_(
            "`#[gen_may_cancel_future]` 要求函数用 where 子句声明全部约束",
            generics.span(),
        ));
    };

    let mut user_lts = Vec::new();
    let mut non_lifetime = Vec::new();
    for param in &generics.params {
        match param {
            syn::GenericParam::Lifetime(lt) => user_lts.push(lt.lifetime.clone()),
            syn::GenericParam::Type(ty) => {
                if let Some(bound) = ty.bounds.first() {
                    return Err(diag_(
                        "请把泛型约束写在 where 子句里，不要在泛型参数上写内联约束",
                        bound.span(),
                    ));
                }
                if ty.default.is_some() {
                    return Err(diag_("泛型参数不支持默认值", ty.ident.span()));
                }
                non_lifetime.push(NonLifetimeParam_::Type_(ty.ident.clone()));
            }
            syn::GenericParam::Const(c) => {
                if c.default.is_some() {
                    return Err(diag_("常量泛型参数不支持默认值", c.ident.span()));
                }
                non_lifetime.push(NonLifetimeParam_::Const_(Box::new(c.clone())));
            }
        }
    }

    if user_lts.is_empty() {
        return Err(diag_(
            "`#[gen_may_cancel_future]` 要求函数至少显式声明一个生命周期",
            generics.span(),
        ));
    }
    let Some(last_param) = non_lifetime.last() else {
        return Err(diag_(
            "`#[gen_may_cancel_future]` 要求函数至少有一个泛型类型参数作为取消令牌类型",
            generics.span(),
        ));
    };
    let NonLifetimeParam_::Type_(cancel_ident) = last_param else {
        return Err(diag_(
            "最后一个泛型参数必须是取消令牌类型参数（不能是常量参数）",
            generics.span(),
        ));
    };
    let cancel_ident = cancel_ident.clone();

    // 生成的名字统一以 `__` 结尾，用户不能占用。
    for lt in &user_lts {
        if lt.ident.to_string().ends_with("__") {
            return Err(diag_(
                "生命周期名不能以 `__` 结尾（该后缀由宏保留）",
                lt.span(),
            ));
        }
    }
    for param in &non_lifetime {
        let ident = param.ident_();
        if ident.to_string().ends_with("__") {
            return Err(diag_(
                "泛型参数名不能以 `__` 结尾（该后缀由宏保留）",
                ident.span(),
            ));
        }
    }

    //-- 2. 取消令牌类型必须被约束为 `TrCancellationToken` -------------------
    if !has_cancellable_bound_(where_clause, &cancel_ident) {
        return Err(diag_(
            "取消令牌类型参数必须在 where 子句中约束 `TrCancellationToken`",
            cancel_ident.span(),
        ));
    }

    //-- 3. 参数列表 ---------------------------------------------------------
    let inputs = &sig.inputs;
    let Some(last_arg) = inputs.last() else {
        return Err(diag_(
            "`#[gen_may_cancel_future]` 要求函数的最后一个参数是取消令牌",
            sig.paren_token.span.join(),
        ));
    };
    let syn::FnArg::Typed(last_typed) = last_arg else {
        return Err(diag_(
            "`#[gen_may_cancel_future]` 不支持方法接收者（`self`）",
            last_arg.span(),
        ));
    };
    if !query_::ty_is_ident_(&last_typed.ty, &cancel_ident) {
        return Err(diag_(
            format!("最后一个参数必须是取消令牌，其类型为裸类型参数 `{cancel_ident}`（按值传入）"),
            last_typed.ty.span(),
        ));
    }

    let mut field_tys = Vec::new();
    let mut arg_idents = Vec::new();
    for arg in inputs.iter().take(inputs.len() - 1) {
        let syn::FnArg::Typed(typed) = arg else {
            return Err(diag_(
                "`#[gen_may_cancel_future]` 不支持方法接收者（`self`）",
                arg.span(),
            ));
        };
        if query_::ty_mentions_ident_(&typed.ty, &cancel_ident) {
            return Err(diag_(
                format!("取消令牌类型参数 `{cancel_ident}` 不能出现在其它参数类型中"),
                typed.ty.span(),
            ));
        }
        // 构造函数 `new` 的形参名：能沿用原参数名就沿用，否则回退到 `p{i}`。
        let index = field_tys.len();
        arg_idents.push(match &*typed.pat {
            syn::Pat::Ident(pat) if pat.subpat.is_none() && pat.ident != "_" => pat.ident.clone(),
            _ => zyn::format_ident!("p{}", index),
        });
        field_tys.push((*typed.ty).clone());
    }

    //-- 4. 返回类型 ---------------------------------------------------------
    let output_ty = match &sig.output {
        syn::ReturnType::Type(_, ty) => (**ty).clone(),
        syn::ReturnType::Default => syn::Type::Tuple(syn::TypeTuple {
            paren_token: Default::default(),
            elems: Default::default(),
        }),
    };
    if query_::ty_mentions_ident_(&output_ty, &cancel_ident) {
        return Err(diag_(
            format!("取消令牌类型参数 `{cancel_ident}` 不能出现在返回类型中"),
            output_ty.span(),
        ));
    }

    //-- 5. where 子句拆分 ---------------------------------------------------
    let preds_base: Vec<syn::WherePredicate> = where_clause
        .predicates
        .iter()
        .filter(|pred| !query_::pred_mentions_ident_(pred, &cancel_ident))
        .cloned()
        .collect();

    let lt_fut = syn::Lifetime::new(K_LT_FUT, proc_macro2::Span::call_site());
    let lt_mcf = syn::Lifetime::new(K_LT_MCF, proc_macro2::Span::call_site());

    let mut preds_common = preds_base.clone();
    for lt in &user_lts {
        preds_common.push(query_::lifetime_bound_pred_(lt, &lt_fut));
    }

    //-- 6. 标记字段 ---------------------------------------------------------
    let type_params: Vec<syn::Ident> = non_lifetime
        .iter()
        .filter_map(|param| match param {
            NonLifetimeParam_::Type_(ident) if *ident != cancel_ident => Some(ident.clone()),
            _ => None,
        })
        .collect();
    let const_params: Vec<syn::ConstParam> = non_lifetime
        .iter()
        .filter_map(|param| match param {
            NonLifetimeParam_::Const_(param) => Some((**param).clone()),
            _ => None,
        })
        .collect();

    let marker_lts: Vec<syn::Lifetime> = user_lts
        .iter()
        .filter(|lt| {
            !field_tys
                .iter()
                .any(|ty| query_::ty_mentions_lifetime_(ty, lt))
        })
        .cloned()
        .collect();

    let marker_tys: Vec<syn::Ident> = type_params
        .iter()
        .filter(|ident| {
            !field_tys
                .iter()
                .any(|ty| query_::ty_mentions_ident_(ty, ident))
        })
        .cloned()
        .collect();

    //-- 7. 工厂 trait 需要的 outlives ---------------------------------------
    let mut pairs = Vec::new();
    for ty in &field_tys {
        query_::collect_ref_outlives_(ty, &type_params, &mut pairs);
    }
    let preds_factory: Vec<syn::WherePredicate> = pairs
        .into_iter()
        .map(|(lt, ident)| query_::type_outlives_pred_(&ident, &lt))
        .collect();

    //-- 8. 组装 -------------------------------------------------------------
    let pat_idents = (0..field_tys.len())
        .map(|i| zyn::format_ident!("p{}", i))
        .collect();

    Ok(GenModel {
        vis_types_: vis_types,
        vis_new_: vis_new,
        fn_ident_: sig.ident.clone(),
        async_ident_: format_ident_with_span_(prefix, "Async"),
        future_ident_: format_ident_with_span_(prefix, "Future"),
        factory_ident_: zyn::format_ident!("Tr{}FutureFactory", prefix),
        lt_fut_: lt_fut,
        lt_mcf_: lt_mcf,
        user_lts_: user_lts,
        type_params_: type_params,
        const_params_: const_params,
        field_tys_: field_tys,
        output_ty_: output_ty,
        preds_common_: preds_common,
        preds_factory_: preds_factory,
        marker_lts_: marker_lts,
        marker_tys_: marker_tys,
        pat_idents_: pat_idents,
        arg_idents_: arg_idents,
    })
}

/// 已解析的、非生命周期的泛型参数。
enum NonLifetimeParam_ {
    /// 类型参数。
    Type_(syn::Ident),
    /// 常量参数。
    Const_(Box<syn::ConstParam>),
}

impl NonLifetimeParam_ {
    fn ident_(&self) -> &syn::Ident {
        match self {
            Self::Type_(ident) => ident,
            Self::Const_(param) => &param.ident,
        }
    }
}

fn format_ident_with_span_(prefix: &syn::Ident, suffix: &str) -> syn::Ident {
    zyn::format_ident!("{}{}", prefix, suffix)
}

/// 取消令牌类型参数是否被约束为 `TrCancellationToken`。
///
/// 只按 trait 路径的最后一段匹配，以兼容经由 `x_deps` 重导出的路径。
fn has_cancellable_bound_(where_clause: &syn::WhereClause, cancel_ident: &syn::Ident) -> bool {
    where_clause.predicates.iter().any(|pred| {
        let syn::WherePredicate::Type(pred) = pred else {
            return false;
        };
        if !query_::ty_is_ident_(&pred.bounded_ty, cancel_ident) {
            return false;
        }
        pred.bounds.iter().any(|bound| {
            let syn::TypeParamBound::Trait(bound) = bound else {
                return false;
            };
            bound
                .path
                .segments
                .last()
                .is_some_and(|seg| seg.ident == "TrCancellationToken")
        })
    })
}
