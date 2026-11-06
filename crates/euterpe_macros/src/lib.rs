//! Derive macros for Euterpe, re-exported by `euterpe_core`.
//!
//! - `#[derive(Params)]` turns a struct of parameter fields into a
//!   `ParamSet`, like the `params!` macro but as an ordinary struct, and with
//!   nested parameter sets.
//! - `#[derive(Chain)]` turns a struct of processors into one `Processor`
//!   that runs them in field order, with a generated parameter struct that
//!   nests each stage's parameters under the field name.
//!
//! Generated code names `::euterpe_core`, so the crate using them must
//! depend on `euterpe_core` under that name.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as Tokens;
use quote::{format_ident, quote};
use syn::spanned::Spanned;
use syn::{Data, DeriveInput, Error, Expr, Fields, Ident, LitStr, Type, Visibility};

/// Implement `Default` and `ParamSet` for a struct of parameters.
///
/// ```ignore
/// use euterpe_core::{ParamKind, Params};
///
/// #[derive(Params, Clone, Debug, PartialEq)]
/// pub struct AmpParams {
///     #[param(id = "drive", label = "Drive", kind = ParamKind::float(0.0, 36.0, 12.0).unit("dB"))]
///     pub drive_db: f32,
///     #[param(label = "Bright", kind = ParamKind::toggle(false))]
///     pub bright: bool,
///     /// Every id of `ToneParams` appears as `tone.<id>`.
///     #[param(nested)]
///     pub tone: ToneParams,
/// }
/// ```
///
/// Leaf fields are `f32` (float), `bool` (toggle) or `u8` (choice index);
/// `id` defaults to the field name. `#[param(nested)]` takes any `ParamSet`
/// and prefixes its ids with the field name, or with `nested = "name"`.
/// Duplicate ids are a compile error.
#[proc_macro_derive(Params, attributes(param))]
pub fn derive_params(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as DeriveInput);
    params_impl(&input)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

/// Implement `Processor` for a struct of processors, run in field order.
///
/// ```ignore
/// use euterpe_core::Chain;
///
/// #[derive(Chain, Default)]
/// #[chain(params = RigParams)] // optional, defaults to `<Name>Params`
/// pub struct Rig {
///     pub drive: Oversampled4<Drive>,
///     pub cab: Cab,
/// }
/// ```
///
/// Generates `pub struct RigParams { pub drive: <Oversampled4<Drive> as
/// Processor>::Params, pub cab: <Cab as Processor>::Params }` with ids
/// `drive.<id>` and `cab.<id>`, and a `Processor` whose `prepare`, `reset`
/// and `process` visit each field in order and whose `latency` is the sum.
#[proc_macro_derive(Chain, attributes(chain))]
pub fn derive_chain(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as DeriveInput);
    chain_impl(&input)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

// ---------------------------------------------------------------------------
// Params
// ---------------------------------------------------------------------------

struct Leaf {
    field: Ident,
    ty: Type,
    id: LitStr,
    label: Expr,
    kind: Expr,
}

struct Nested {
    field: Ident,
    ty: Type,
    prefix: LitStr,
}

fn named_fields(input: &DeriveInput, what: &str) -> syn::Result<Vec<syn::Field>> {
    if !input.generics.params.is_empty() {
        return Err(Error::new(
            input.generics.span(),
            format!("#[derive({what})] does not support generic structs"),
        ));
    }
    match &input.data {
        Data::Struct(s) => match &s.fields {
            Fields::Named(f) => Ok(f.named.iter().cloned().collect()),
            _ => Err(Error::new(
                input.ident.span(),
                format!("#[derive({what})] needs a struct with named fields"),
            )),
        },
        _ => Err(Error::new(
            input.ident.span(),
            format!("#[derive({what})] needs a struct"),
        )),
    }
}

fn params_impl(input: &DeriveInput) -> syn::Result<Tokens> {
    let mut leaves = Vec::new();
    let mut nested = Vec::new();
    for f in named_fields(input, "Params")? {
        let field = f
            .ident
            .clone()
            .ok_or_else(|| Error::new(f.span(), "unnamed field"))?;
        let attr = f
            .attrs
            .iter()
            .find(|a| a.path().is_ident("param"))
            .ok_or_else(|| Error::new(f.span(), "every field needs #[param(...)]"))?;
        let (mut id, mut label, mut kind, mut prefix) = (None, None, None, None);
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("id") {
                id = Some(meta.value()?.parse::<LitStr>()?);
            } else if meta.path.is_ident("label") {
                label = Some(meta.value()?.parse::<Expr>()?);
            } else if meta.path.is_ident("kind") {
                kind = Some(meta.value()?.parse::<Expr>()?);
            } else if meta.path.is_ident("nested") {
                prefix = Some(if meta.input.peek(syn::Token![=]) {
                    meta.value()?.parse::<LitStr>()?
                } else {
                    LitStr::new(&field.to_string(), field.span())
                });
            } else {
                return Err(meta.error("expected `id`, `label`, `kind` or `nested`"));
            }
            Ok(())
        })?;
        if let Some(prefix) = prefix {
            if id.is_some() || label.is_some() || kind.is_some() {
                return Err(Error::new(
                    attr.span(),
                    "`nested` takes no id, label or kind",
                ));
            }
            nested.push(Nested {
                field,
                ty: f.ty.clone(),
                prefix,
            });
        } else {
            let label = label.ok_or_else(|| Error::new(attr.span(), "missing `label = ...`"))?;
            let kind = kind.ok_or_else(|| Error::new(attr.span(), "missing `kind = ...`"))?;
            leaves.push(Leaf {
                id: id.unwrap_or_else(|| LitStr::new(&field.to_string(), field.span())),
                field,
                ty: f.ty.clone(),
                label,
                kind,
            });
        }
    }
    Ok(paramset(&input.ident, &leaves, &nested))
}

/// `Default` and `ParamSet` for `name`, made of `leaves` and `nested` sets.
fn paramset(name: &Ident, leaves: &[Leaf], nested: &[Nested]) -> Tokens {
    let core = quote!(::euterpe_core::params);
    let defaults = leaves.iter().map(|l| {
        let (field, ty, kind) = (&l.field, &l.ty, &l.kind);
        quote!(#field: <#ty as #core::ParamField>::default_for(&#kind))
    });
    let nested_defaults = nested.iter().map(|n| {
        let (field, ty) = (&n.field, &n.ty);
        quote!(#field: <#ty as ::core::default::Default>::default())
    });
    let own = leaves.iter().map(|l| {
        let (id, label, kind) = (&l.id, &l.label, &l.kind);
        quote!(#core::ParamDesc { id: #id, label: #label, kind: #kind })
    });
    let parts = nested.iter().map(|n| {
        let (ty, prefix) = (&n.ty, &n.prefix);
        quote!(#core::compose::Part {
            prefix: #prefix,
            schema: <#ty as #core::ParamSet>::SCHEMA,
        })
    });
    let get_leaf = leaves.iter().map(|l| {
        let (field, ty, id) = (&l.field, &l.ty, &l.id);
        quote!(#id => ::core::option::Option::Some(<#ty as #core::ParamField>::to_value(&self.#field)),)
    });
    let set_leaf = leaves.iter().map(|l| {
        let (field, ty, id, kind) = (&l.field, &l.ty, &l.id, &l.kind);
        quote!(#id => <#ty as #core::ParamField>::assign(&mut self.#field, &#kind, value),)
    });
    let dotted: Vec<LitStr> = nested
        .iter()
        .map(|n| LitStr::new(&format!("{}.", n.prefix.value()), n.prefix.span()))
        .collect();
    let get_nested = nested.iter().zip(&dotted).map(|(n, dot)| {
        let (field, ty) = (&n.field, &n.ty);
        quote! {
            if let ::core::option::Option::Some(rest) = id.strip_prefix(#dot) {
                return <#ty as #core::ParamSet>::get(&self.#field, rest);
            }
        }
    });
    let set_nested = nested.iter().zip(&dotted).map(|(n, dot)| {
        let (field, ty) = (&n.field, &n.ty);
        quote! {
            if let ::core::option::Option::Some(rest) = id.strip_prefix(#dot) {
                return <#ty as #core::ParamSet>::set(&mut self.#field, rest, value);
            }
        }
    });
    quote! {
        impl ::core::default::Default for #name {
            fn default() -> Self {
                Self { #(#defaults,)* #(#nested_defaults,)* }
            }
        }

        impl #core::ParamSet for #name {
            const SCHEMA: &'static [#core::ParamDesc] = {
                const PARTS: &[#core::compose::Part] = &[
                    #core::compose::Part { prefix: "", schema: &[#(#own),*] },
                    #(#parts),*
                ];
                const LEN: usize = #core::compose::count(PARTS);
                const IDS_LEN: usize = #core::compose::ids_len(PARTS);
                const IDS: &[u8; IDS_LEN] = &#core::compose::ids::<IDS_LEN>(PARTS);
                const DESCS: &[#core::ParamDesc; LEN] = &#core::compose::descs::<LEN>(PARTS, IDS);
                DESCS
            };

            #[allow(clippy::needless_return, reason = "generated: one early return per nested set")]
            fn get(&self, id: &str) -> ::core::option::Option<#core::ParamValue> {
                #(#get_nested)*
                match id {
                    #(#get_leaf)*
                    _ => ::core::option::Option::None,
                }
            }

            #[allow(clippy::needless_return, reason = "generated: one early return per nested set")]
            fn set(
                &mut self,
                id: &str,
                value: #core::ParamValue,
            ) -> ::core::result::Result<(), #core::ParamError> {
                #(#set_nested)*
                match id {
                    #(#set_leaf)*
                    _ => ::core::result::Result::Err(#core::ParamError::UnknownId),
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Chain
// ---------------------------------------------------------------------------

fn chain_impl(input: &DeriveInput) -> syn::Result<Tokens> {
    let name = &input.ident;
    let mut params_name = format_ident!("{}Params", name);
    for attr in input.attrs.iter().filter(|a| a.path().is_ident("chain")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("params") {
                params_name = meta.value()?.parse::<Ident>()?;
                Ok(())
            } else {
                Err(meta.error("expected `params = Name`"))
            }
        })?;
    }
    let fields = named_fields(input, "Chain")?;
    if fields.is_empty() {
        return Err(Error::new(name.span(), "a chain needs at least one stage"));
    }
    let vis: &Visibility = &input.vis;
    let proc_ = quote!(::euterpe_core::processor::Processor);

    let mut nested = Vec::new();
    let mut idents = Vec::new();
    for f in &fields {
        let field = f
            .ident
            .clone()
            .ok_or_else(|| Error::new(f.span(), "unnamed field"))?;
        let ty = &f.ty;
        nested.push(Nested {
            prefix: LitStr::new(&field.to_string(), field.span()),
            field: field.clone(),
            ty: syn::parse_quote!(<#ty as #proc_>::Params),
        });
        idents.push(field);
    }
    let param_fields = nested.iter().map(|n| {
        let (field, ty) = (&n.field, &n.ty);
        quote!(pub #field: #ty)
    });
    let paramset = paramset(&params_name, &[], &nested);
    let doc = format!(
        "Parameters of [`{name}`], one nested set per stage. Generated by `#[derive(Chain)]`."
    );

    Ok(quote! {
        #[doc = #doc]
        #[derive(Clone)]
        #vis struct #params_name {
            #(#param_fields,)*
        }

        #paramset

        impl #proc_ for #name {
            type Params = #params_name;

            fn prepare(&mut self, ctx: &::euterpe_core::processor::Ctx) {
                #(#proc_::prepare(&mut self.#idents, ctx);)*
            }

            fn reset(&mut self) {
                #(#proc_::reset(&mut self.#idents);)*
            }

            #[inline]
            fn process(&mut self, buf: &mut ::euterpe_core::processor::Block, params: &Self::Params) {
                #(#proc_::process(&mut self.#idents, buf, &params.#idents);)*
            }

            fn latency(&self) -> usize {
                0 #(+ #proc_::latency(&self.#idents))*
            }
        }
    })
}
