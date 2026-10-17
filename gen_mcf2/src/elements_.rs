//! 展开用的 `zyn` 元素（模板组件）。
//!
//! 每个元素负责生成一两个条目；条目之间的公共部分（泛型参数列表）再由更小
//! 的元素复用。
//!
//! 注意：`@元素(...)` 之后**不能紧跟 `{` 或 `{{ ... }}`**，否则会被
//! `zyn` 解析器当作该元素的 `children` 块。因此这里的泛型元素只产出
//! 参数序列本身，尖括号由调用方书写。

use zyn::TokenStream;

use crate::model_::GenModel;

/// 生成泛型参数**声明**序列（不含尖括号），末尾统一带逗号。
///
/// `token` 是取消令牌类型参数在末尾的名字（不需要时传空 TokenStream）。
#[zyn::element]
pub(crate) fn generic_decl_params_(model: GenModel, token: TokenStream) -> TokenStream {
    zyn::zyn! {
        @for (lt in model.user_lts_.iter()) { {{ lt }}, }
        {{ model.lt_fut_ }},
        @for (tp in model.type_params_.iter()) { {{ tp }}, }
        @for (cp in model.const_params_.iter()) { {{ cp }}, }
        {{ token }}
    }
}

/// 生成泛型参数**使用**序列（不含尖括号），末尾统一带逗号。
///
/// `lt_fut` 是代表 future 自身的那个生命周期（通常是 `'lt_fut__`，
/// `TrMayCancel` 的 GAT 里则是 `'lt_mcf__`）；`token` 含义同
/// [`generic_decl_params_`]。
#[zyn::element]
pub(crate) fn generic_use_params_(
    model: GenModel,
    lt_fut: zyn::syn::Lifetime,
    token: TokenStream,
) -> TokenStream {
    zyn::zyn! {
        @for (lt in model.user_lts_.iter()) { {{ lt }}, }
        {{ lt_fut }},
        @for (tp in model.type_params_.iter()) { {{ tp }}, }
        @for (cp in model.const_params_.iter()) { {{ cp.ident }}, }
        {{ token }}
    }
}

/// `{Prefix}Async`：持有原函数「除取消令牌外」的全部参数。
#[zyn::element]
pub(crate) fn async_struct_(model: GenModel) -> TokenStream {
    zyn::zyn! {
        {{ model.vis_types_ }} struct {{ model.async_ident_ }} <
        @generic_decl_params(model = (*model).clone(), token = zyn::quote::quote!())
        > (
            @for (ty in model.field_tys_.iter()) { {{ ty }}, }
            ::core::marker::PhantomData<&{{ model.lt_fut_ }} ()>,
            @for (lt in model.marker_lts_.iter()) { ::core::marker::PhantomData<&{{ lt }} ()>, }
            @for (tp in model.marker_tys_.iter()) { ::core::marker::PhantomData<fn() -> {{ tp }}>, }
        )
        where
            @for (pred in model.preds_common_.iter()) { {{ pred }}, }
        ;
    }
}

/// `{Prefix}Async` 的构造函数 `new`：由宏内部补齐全部标记字段，
/// 调用者只需按原函数参数顺序传入实参。
#[zyn::element]
pub(crate) fn async_new_impl_(model: GenModel) -> TokenStream {
    zyn::zyn! {
        impl <
        @generic_decl_params(model = (*model).clone(), token = zyn::quote::quote!())
        > {{ model.async_ident_ }} <
        @generic_use_params(model = (*model).clone(), lt_fut = model.lt_fut_.clone(), token = zyn::quote::quote!())
        >
        where
            @for (pred in model.preds_common_.iter()) { {{ pred }}, }
        {
            /// 构造本类型；参数顺序与原 `async fn` 的参数一致（不含取消令牌），
            /// 尾部的 `PhantomData` 标记字段由本方法自动补齐。
            #[allow(clippy::too_many_arguments)]
            #[allow(clippy::new_without_default)]
            #[inline]
            {{ model.vis_new_ }} fn new(
                @for (i in 0..model.field_tys_.len()) { {{ model.arg_idents_[i] }}: {{ model.field_tys_[i] }}, }
            ) -> Self {
                {{ model.async_ident_ }}(
                    @for (arg in model.arg_idents_.iter()) { {{ arg }}, }
                    ::core::marker::PhantomData,
                    @for (_lt in model.marker_lts_.iter()) { ::core::marker::PhantomData, }
                    @for (_tp in model.marker_tys_.iter()) { ::core::marker::PhantomData, }
                )
            }
        }
    }
}

/// `{Prefix}Future`：真正的 `Future`，持有参数、取消令牌与内部 future。
#[zyn::element]
pub(crate) fn future_struct_(model: GenModel) -> TokenStream {
    zyn::zyn! {
        {{ model.vis_types_ }} struct {{ model.future_ident_ }} <
        @generic_decl_params(model = (*model).clone(), token = zyn::quote::quote!(TyTok__))
        >
        where
            @for (pred in model.preds_common_.iter()) { {{ pred }}, }
            TyTok__: abs_cancel::TrCancellationToken,
        {
            params_: ::core::option::Option<
                {{ model.async_ident_ }} <
                @generic_use_params(model = (*model).clone(), lt_fut = model.lt_fut_.clone(), token = zyn::quote::quote!())
                >
            >,
            cancel_: ::core::option::Option<TyTok__>,
            future_: ::core::option::Option<
                <() as {{ model.factory_ident_ }} <
                @generic_use_params(model = (*model).clone(), lt_fut = model.lt_fut_.clone(), token = zyn::quote::quote!(TyTok__))
                >>::MadeFuture
            >,
        }
    }
}

/// `impl IntoFuture for {Prefix}Async`：不可取消路径。
#[zyn::element]
pub(crate) fn into_future_(model: GenModel) -> TokenStream {
    zyn::zyn! {
        impl <
        @generic_decl_params(model = (*model).clone(), token = zyn::quote::quote!())
        > ::core::future::IntoFuture for {{ model.async_ident_ }} <
        @generic_use_params(model = (*model).clone(), lt_fut = model.lt_fut_.clone(), token = zyn::quote::quote!())
        >
        where
            @for (pred in model.preds_common_.iter()) { {{ pred }}, }
        {
            type IntoFuture = {{ model.future_ident_ }} <
            @generic_use_params(model = (*model).clone(), lt_fut = model.lt_fut_.clone(), token = zyn::quote::quote!(abs_cancel::NonCancellableToken))
            >;

            type Output = {{ model.output_ty_ }};

            fn into_future(self) -> Self::IntoFuture {
                {{ model.future_ident_ }} {
                    params_: ::core::option::Option::Some(self),
                    cancel_: ::core::option::Option::Some(abs_cancel::NonCancellableToken::new()),
                    future_: ::core::option::Option::None,
                }
            }
        }
    }
}

/// `impl TrMayCancel<'lt_fut__> for {Prefix}Async`：可取消路径。
#[zyn::element]
pub(crate) fn may_cancel_(model: GenModel) -> TokenStream {
    zyn::zyn! {
        impl <
        @generic_decl_params(model = (*model).clone(), token = zyn::quote::quote!())
        > abs_cancel::TrMayCancel<{{ model.lt_fut_ }}> for {{ model.async_ident_ }} <
        @generic_use_params(model = (*model).clone(), lt_fut = model.lt_fut_.clone(), token = zyn::quote::quote!())
        >
        where
            @for (pred in model.preds_common_.iter()) { {{ pred }}, }
        {
            type MayCancelFuture<'lt_mcf__, TyTok__> = {{ model.future_ident_ }} <
            @generic_use_params(model = (*model).clone(), lt_fut = model.lt_mcf_.clone(), token = zyn::quote::quote!(TyTok__))
            >
            where
                {{ model.lt_mcf_ }}: {{ model.lt_fut_ }},
                Self: {{ model.lt_mcf_ }},
                TyTok__: {{ model.lt_mcf_ }} + abs_cancel::TrCancellationToken;

            type MayCancelOutput = {{ model.output_ty_ }};

            fn may_cancel_with<TyTok__>(
                self,
                cancel: TyTok__,
            ) -> Self::MayCancelFuture<{{ model.lt_fut_ }}, TyTok__>
            where
                TyTok__: {{ model.lt_fut_ }} + abs_cancel::TrCancellationToken,
            {
                {{ model.future_ident_ }} {
                    params_: ::core::option::Option::Some(self),
                    cancel_: ::core::option::Option::Some(cancel),
                    future_: ::core::option::Option::None,
                }
            }
        }
    }
}

/// `impl Future for {Prefix}Future`。
#[zyn::element]
pub(crate) fn future_impl_(model: GenModel) -> TokenStream {
    zyn::zyn! {
        impl <
        @generic_decl_params(model = (*model).clone(), token = zyn::quote::quote!(TyTok__))
        > ::core::future::Future for {{ model.future_ident_ }} <
        @generic_use_params(model = (*model).clone(), lt_fut = model.lt_fut_.clone(), token = zyn::quote::quote!(TyTok__))
        >
        where
            @for (pred in model.preds_common_.iter()) { {{ pred }}, }
            TyTok__: {{ model.lt_fut_ }} + abs_cancel::TrCancellationToken,
        {
            type Output = {{ model.output_ty_ }};

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
                    let (params, cancel) =
                        match (this.params_.take(), this.cancel_.take()) {
                            (
                                ::core::option::Option::Some(params),
                                ::core::option::Option::Some(cancel),
                            ) => (params, cancel),
                            // 不可达：`future_` 为 `None` 时参数与令牌必然都还在。
                            _ => unreachable!("gen_may_cancel_future: future 状态机不变量被破坏"),
                        };
                    this.future_ = ::core::option::Option::Some(
                        <() as {{ model.factory_ident_ }} <
                        @generic_use_params(model = (*model).clone(), lt_fut = model.lt_fut_.clone(), token = zyn::quote::quote!(TyTok__))
                        >>::make_future(params, cancel),
                    );
                }
            }
        }
    }
}

/// 前缀独有的私有工厂 trait：为原函数返回的 opaque future 起名。
#[zyn::element]
pub(crate) fn factory_trait_(model: GenModel) -> TokenStream {
    zyn::zyn! {
        trait {{ model.factory_ident_ }} <
        @generic_decl_params(model = (*model).clone(), token = zyn::quote::quote!(TyTok__))
        >
        where
            @for (pred in model.preds_common_.iter()) { {{ pred }}, }
            @for (pred in model.preds_factory_.iter()) { {{ pred }}, }
            TyTok__: abs_cancel::TrCancellationToken,
        {
            type MadeFuture: ::core::future::Future<Output = {{ model.output_ty_ }}>;

            fn make_future(
                params: {{ model.async_ident_ }} <
                @generic_use_params(model = (*model).clone(), lt_fut = model.lt_fut_.clone(), token = zyn::quote::quote!())
                >,
                cancel: TyTok__,
            ) -> Self::MadeFuture;
        }
    }
}

/// 工厂 trait 挂在 `()` 上的实现：真正调用原函数。
#[zyn::element]
pub(crate) fn factory_impl_(model: GenModel) -> TokenStream {
    zyn::zyn! {
        impl <
        @generic_decl_params(model = (*model).clone(), token = zyn::quote::quote!(TyTok__))
        > {{ model.factory_ident_ }} <
        @generic_use_params(model = (*model).clone(), lt_fut = model.lt_fut_.clone(), token = zyn::quote::quote!(TyTok__))
        > for ()
        where
            @for (pred in model.preds_common_.iter()) { {{ pred }}, }
            @for (pred in model.preds_factory_.iter()) { {{ pred }}, }
            TyTok__: abs_cancel::TrCancellationToken,
        {
            type MadeFuture = impl ::core::future::Future<Output = {{ model.output_ty_ }}>;

            fn make_future(
                params: {{ model.async_ident_ }} <
                @generic_use_params(model = (*model).clone(), lt_fut = model.lt_fut_.clone(), token = zyn::quote::quote!())
                >,
                cancel: TyTok__,
            ) -> Self::MadeFuture {
                let {{ model.async_ident_ }}(
                    @for (pat in model.pat_idents_.iter()) { {{ pat }}, }
                    ..
                ) = params;
                self::{{ model.fn_ident_ }}(
                    @for (pat in model.pat_idents_.iter()) { {{ pat }}, }
                    cancel
                )
            }
        }
    }
}
