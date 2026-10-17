//! 展开所需的中间数据模型。

use zyn::syn;

/// 一次宏调用分析出来的全部信息。
///
/// 这些字段只在本 crate 内部使用，因此按仓库约定以 `_` 结尾。
#[derive(Clone)]
pub(crate) struct GenModel {
    /// 生成**类型**（`{Prefix}Async` / `{Prefix}Future`）的可见性：
    /// 宏参数里指定的、或原函数的。
    pub(crate) vis_types_: syn::Visibility,
    /// `{Prefix}Async::new` 的可见性：宏参数里 `new(...)` 指定的，否则沿用
    /// [`Self::vis_types_`]。
    pub(crate) vis_new_: syn::Visibility,
    /// 原函数名。
    pub(crate) fn_ident_: syn::Ident,
    /// `{Prefix}Async`。
    pub(crate) async_ident_: syn::Ident,
    /// `{Prefix}Future`。
    pub(crate) future_ident_: syn::Ident,
    /// `Tr{Prefix}FutureFactory`。
    pub(crate) factory_ident_: syn::Ident,
    /// future 自身的生命周期 `'lt_fut__`。
    pub(crate) lt_fut_: syn::Lifetime,
    /// `TrMayCancel` 的 GAT 中使用的生命周期 `'lt_mcf__`。
    pub(crate) lt_mcf_: syn::Lifetime,
    /// 用户声明的生命周期（保持声明顺序）。
    pub(crate) user_lts_: Vec<syn::Lifetime>,
    /// 用户声明的类型参数（已去掉取消令牌类型参数，保持顺序）。
    pub(crate) type_params_: Vec<syn::Ident>,
    /// 用户声明的常量参数（保持声明顺序）。
    pub(crate) const_params_: Vec<syn::ConstParam>,
    /// 除取消令牌外的参数类型（逐字保留）。
    pub(crate) field_tys_: Vec<syn::Type>,
    /// 原函数的返回类型（逐字保留）。
    pub(crate) output_ty_: syn::Type,
    /// 公共 where 谓词：用户谓词去掉与取消令牌相关的部分，再补
    /// `'lt: 'lt_fut__`。
    pub(crate) preds_common_: Vec<syn::WherePredicate>,
    /// 只有工厂 trait / impl 需要的 outlives 谓词（由字段类型推出）。
    pub(crate) preds_factory_: Vec<syn::WherePredicate>,
    /// 需要 `PhantomData<&'lt ()>` 标记的生命周期。
    pub(crate) marker_lts_: Vec<syn::Lifetime>,
    /// 需要 `PhantomData<fn() -> Tp>` 标记的类型参数。
    pub(crate) marker_tys_: Vec<syn::Ident>,
    /// 工厂解构 `{Prefix}Async` 时使用的绑定名 `p0, p1, ...`。
    pub(crate) pat_idents_: Vec<syn::Ident>,
    /// `{Prefix}Async::new` 的形参名：优先沿用原函数的简单参数名，
    /// 无法沿用（`_`、元组模式等）时回退为 `p0, p1, ...`。
    pub(crate) arg_idents_: Vec<syn::Ident>,
}
