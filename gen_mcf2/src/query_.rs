//! 供展开使用的只读语法树查询工具。

use zyn::syn;

/// 深度遍历类型中的所有子类型（含 `qself`、路径泛型实参）。
///
/// 访问者只会在本函数内部被调用，因此不对外暴露。
fn walk_ty_<F>(ty: &syn::Type, visit: &mut F)
where
    F: FnMut(&syn::Type),
{
    visit(ty);

    match ty {
        syn::Type::Reference(inner) => walk_ty_(&inner.elem, visit),
        syn::Type::Ptr(inner) => walk_ty_(&inner.elem, visit),
        syn::Type::Slice(inner) => walk_ty_(&inner.elem, visit),
        syn::Type::Array(inner) => walk_ty_(&inner.elem, visit),
        syn::Type::Paren(inner) => walk_ty_(&inner.elem, visit),
        syn::Type::Group(inner) => walk_ty_(&inner.elem, visit),
        syn::Type::Tuple(inner) => {
            for elem in &inner.elems {
                walk_ty_(elem, visit);
            }
        }
        syn::Type::Path(inner) => {
            if let Some(qself) = &inner.qself {
                walk_ty_(&qself.ty, visit);
            }
            for seg in &inner.path.segments {
                match &seg.arguments {
                    syn::PathArguments::AngleBracketed(args) => {
                        for arg in &args.args {
                            match arg {
                                syn::GenericArgument::Type(ty) => walk_ty_(ty, visit),
                                syn::GenericArgument::AssocType(assoc) => {
                                    walk_ty_(&assoc.ty, visit);
                                }
                                _ => {}
                            }
                        }
                    }
                    syn::PathArguments::Parenthesized(args) => {
                        for ty in &args.inputs {
                            walk_ty_(ty, visit);
                        }
                        if let syn::ReturnType::Type(_, ty) = &args.output {
                            walk_ty_(ty, visit);
                        }
                    }
                    syn::PathArguments::None => {}
                }
            }
        }
        _ => {}
    }
}

/// 类型中是否出现了指定的类型参数。
pub(crate) fn ty_mentions_ident_(ty: &syn::Type, target: &syn::Ident) -> bool {
    let mut found = false;
    walk_ty_(ty, &mut |ty| {
        if let syn::Type::Path(path) = ty
            && let Some(seg) = path.path.segments.last()
            && seg.ident == *target
        {
            found = true;
        }
    });
    found
}

/// 路径中是否出现了指定的类型参数（含路径泛型实参）。
fn path_mentions_ident_(path: &syn::Path, target: &syn::Ident) -> bool {
    path.segments.iter().any(|seg| {
        seg.ident == *target
            || match &seg.arguments {
                syn::PathArguments::AngleBracketed(args) => args.args.iter().any(|arg| match arg {
                    syn::GenericArgument::Type(ty) => ty_mentions_ident_(ty, target),
                    syn::GenericArgument::AssocType(assoc) => ty_mentions_ident_(&assoc.ty, target),
                    _ => false,
                }),
                _ => false,
            }
    })
}

/// 约束中是否出现了指定的类型参数。
pub(crate) fn pred_mentions_ident_(pred: &syn::WherePredicate, target: &syn::Ident) -> bool {
    match pred {
        syn::WherePredicate::Type(pred) => {
            ty_mentions_ident_(&pred.bounded_ty, target)
                || pred.bounds.iter().any(|bound| match bound {
                    syn::TypeParamBound::Trait(trait_bound) => {
                        path_mentions_ident_(&trait_bound.path, target)
                    }
                    syn::TypeParamBound::Lifetime(lt) => lt.ident == *target,
                    _ => false,
                })
        }
        _ => false,
    }
}

/// 类型中是否出现了指定的生命周期。
pub(crate) fn ty_mentions_lifetime_(ty: &syn::Type, target: &syn::Lifetime) -> bool {
    let mut found = false;
    walk_ty_(ty, &mut |ty| match ty {
        syn::Type::Reference(reference) => {
            if let Some(lt) = &reference.lifetime
                && lt.ident == target.ident
            {
                found = true;
            }
        }
        syn::Type::Path(path) => {
            for seg in &path.path.segments {
                if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
                    for arg in &args.args {
                        if let syn::GenericArgument::Lifetime(lt) = arg
                            && lt.ident == target.ident
                        {
                            found = true;
                        }
                    }
                }
            }
        }
        _ => {}
    });
    found
}

/// 收集类型中出现的、属于 `candidates` 的类型参数（去重）。
pub(crate) fn ty_idents_(ty: &syn::Type, candidates: &[syn::Ident], out: &mut Vec<syn::Ident>) {
    walk_ty_(ty, &mut |ty| {
        if let syn::Type::Path(path) = ty
            && let Some(seg) = path.path.segments.last()
            && candidates.iter().any(|c| c == &seg.ident)
            && !out.iter().any(|o| o == &seg.ident)
        {
            out.push(seg.ident.clone());
        }
    });
}

/// 收集「字段类型里的引用」所隐含的 outlives 关系 `('lt, Tp)`。
///
/// 只处理形如 `&'lt X` 的引用：`&'lt X` 的合法前提就是 `X: 'lt`，
/// 因此 `X` 中出现的每个类型参数都满足 `Tp: 'lt`。这些约束一定能由
/// 结构体字段的隐含约束推出，所以在工厂 trait 上补写它们既安全又必要。
pub(crate) fn collect_ref_outlives_(
    ty: &syn::Type,
    type_params: &[syn::Ident],
    out: &mut Vec<(syn::Lifetime, syn::Ident)>,
) {
    match ty {
        syn::Type::Reference(reference) => {
            if let Some(lt) = &reference.lifetime {
                let mut idents = Vec::new();
                ty_idents_(&reference.elem, type_params, &mut idents);
                for ident in idents {
                    if !out.iter().any(|(l, i)| l.ident == lt.ident && *i == ident) {
                        out.push((lt.clone(), ident));
                    }
                }
            }
            collect_ref_outlives_(&reference.elem, type_params, out);
        }
        syn::Type::Slice(inner) => collect_ref_outlives_(&inner.elem, type_params, out),
        syn::Type::Array(inner) => collect_ref_outlives_(&inner.elem, type_params, out),
        syn::Type::Paren(inner) => collect_ref_outlives_(&inner.elem, type_params, out),
        syn::Type::Group(inner) => collect_ref_outlives_(&inner.elem, type_params, out),
        syn::Type::Tuple(tuple) => {
            for elem in &tuple.elems {
                collect_ref_outlives_(elem, type_params, out);
            }
        }
        syn::Type::Path(path) => {
            if let Some(qself) = &path.qself {
                collect_ref_outlives_(&qself.ty, type_params, out);
            }
            for seg in &path.path.segments {
                if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
                    for arg in &args.args {
                        if let syn::GenericArgument::Type(ty) = arg {
                            collect_ref_outlives_(ty, type_params, out);
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

/// 构造 `'lt: 'bound` 形式的生命周期约束。
pub(crate) fn lifetime_bound_pred_(
    lt: &syn::Lifetime,
    bound: &syn::Lifetime,
) -> syn::WherePredicate {
    syn::WherePredicate::Lifetime(syn::PredicateLifetime {
        lifetime: lt.clone(),
        colon_token: Default::default(),
        bounds: core::iter::once(bound.clone()).collect(),
    })
}

/// 构造 `Tp: 'lt` 形式的类型 outlives 约束。
pub(crate) fn type_outlives_pred_(ident: &syn::Ident, lt: &syn::Lifetime) -> syn::WherePredicate {
    syn::WherePredicate::Type(syn::PredicateType {
        lifetimes: None,
        bounded_ty: syn::Type::Path(syn::TypePath {
            qself: None,
            path: syn::Path::from(ident.clone()),
        }),
        colon_token: Default::default(),
        bounds: core::iter::once(syn::TypeParamBound::Lifetime(lt.clone())).collect(),
    })
}

/// 判断一个类型是否是「指定的类型参数本身」。
pub(crate) fn ty_is_ident_(ty: &syn::Type, target: &syn::Ident) -> bool {
    matches!(
        ty,
        syn::Type::Path(path) if path.qself.is_none() && path.path.is_ident(target)
    )
}
