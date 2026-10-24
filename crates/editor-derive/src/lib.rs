//! `#[derive(Inspector)]`: a tuning struct draws its own editor panel.
//!
//! The macro shape follows bevy-inspector-egui's derive (`expand_struct` plus
//! one `#[inspector(..)]` per field), without reflection: each non-skipped
//! field becomes one row of a `const` table and the emitted `fn ui` hands that
//! table to the editor's draw seam (`engine_editor::inspector_fields::draw`).
//! A field the derive cannot draw is a compile error, never a runtime refusal.

use proc_macro::TokenStream;
use proc_macro2::{Literal, TokenStream as TokenStream2};
use quote::quote;
use syn::{Data, DeriveInput, Error, Field, Fields, Lit, Result, Type};

/// Derive `fn ui(&mut self, ui: &mut egui::Ui) -> bool` and its field table.
#[proc_macro_derive(Inspector, attributes(inspector))]
pub fn derive_inspector(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as DeriveInput);
    expand(&input)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

/// What the `#[inspector(..)]` attributes of one field asked for.
#[derive(Default)]
struct FieldAttrs {
    min: Option<f64>,
    max: Option<f64>,
    step: Option<f64>,
    group: Option<String>,
    readonly: bool,
    skip: bool,
}

/// The field types the panel knows how to draw.
#[derive(Clone, Copy)]
enum Kind {
    F32,
    F64,
    Bool,
}

impl Kind {
    /// Read the kind off the field type: `f32`, `f64` or `bool`.
    fn of(ty: &Type) -> Result<Self> {
        let Type::Path(path) = ty else {
            return Err(unsupported(ty));
        };
        let Some(segment) = path.path.segments.last() else {
            return Err(unsupported(ty));
        };
        if !segment.arguments.is_empty() {
            return Err(unsupported(ty));
        }
        match segment.ident.to_string().as_str() {
            "f32" => Ok(Self::F32),
            "f64" => Ok(Self::F64),
            "bool" => Ok(Self::Bool),
            _ => Err(unsupported(ty)),
        }
    }

    /// Typed accessors for one field of this kind.
    fn access(self, field: &syn::Ident) -> TokenStream2 {
        match self {
            Self::F32 => quote! {
                ::engine_editor::inspector_fields::FieldAccess::F32 {
                    get: |value| value.#field,
                    set: |value, next| value.#field = next,
                }
            },
            Self::F64 => quote! {
                ::engine_editor::inspector_fields::FieldAccess::F64 {
                    get: |value| value.#field,
                    set: |value, next| value.#field = next,
                }
            },
            Self::Bool => quote! {
                ::engine_editor::inspector_fields::FieldAccess::Bool {
                    get: |value| value.#field,
                    set: |value, next| value.#field = next,
                }
            },
        }
    }
}

fn unsupported(ty: &Type) -> Error {
    Error::new_spanned(
        ty,
        "`#[derive(Inspector)]` draws f32, f64 and bool fields; this type needs its own widget",
    )
}

fn expand(input: &DeriveInput) -> Result<TokenStream2> {
    let Data::Struct(data) = &input.data else {
        return Err(Error::new_spanned(
            &input.ident,
            "`#[derive(Inspector)]` draws a struct with named fields",
        ));
    };
    let Fields::Named(named) = &data.fields else {
        return Err(Error::new_spanned(
            &input.ident,
            "`#[derive(Inspector)]` needs named fields: a tuple field has no label",
        ));
    };
    let mut rows = Vec::with_capacity(named.named.len());
    for field in &named.named {
        if let Some(row) = spec(field)? {
            rows.push(row);
        }
    }
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    Ok(quote! {
        impl #impl_generics #name #ty_generics #where_clause {
            /// Every field the derived panel draws, in declaration order.
            pub const INSPECTOR_FIELDS:
                &'static [::engine_editor::inspector_fields::FieldSpec<Self>] = &[#(#rows),*];

            /// Draw one widget per field; `true` when any value changed.
            pub fn ui(&mut self, ui: &mut ::egui::Ui) -> bool {
                ::engine_editor::inspector_fields::draw(Self::INSPECTOR_FIELDS, self, ui).changed
            }
        }
    })
}

/// One table row, or `None` for a `#[inspector(skip)]` field.
fn spec(field: &Field) -> Result<Option<TokenStream2>> {
    let attrs = field_attrs(field)?;
    if attrs.skip {
        return Ok(None);
    }
    let name = field
        .ident
        .clone()
        .ok_or_else(|| Error::new_spanned(field, "`#[derive(Inspector)]` needs named fields"))?;
    let label = name.to_string();
    let access = Kind::of(&field.ty)?.access(&name);
    let group = attrs.group.unwrap_or_default();
    let min = optional(attrs.min);
    let max = optional(attrs.max);
    let step = optional(attrs.step);
    let readonly = attrs.readonly;
    Ok(Some(quote! {
        ::engine_editor::inspector_fields::FieldSpec {
            name: #label,
            group: #group,
            min: #min,
            max: #max,
            step: #step,
            readonly: #readonly,
            access: #access,
        }
    }))
}

/// One authored literal as an `Option<f64>` expression.
fn optional(value: Option<f64>) -> TokenStream2 {
    match value {
        Some(value) => {
            let literal = Literal::f64_unsuffixed(value);
            quote! { ::core::option::Option::Some(#literal) }
        }
        None => quote! { ::core::option::Option::None },
    }
}

fn field_attrs(field: &Field) -> Result<FieldAttrs> {
    let mut out = FieldAttrs::default();
    for attr in &field.attrs {
        if !attr.path().is_ident("inspector") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            let key = meta
                .path
                .get_ident()
                .map(ToString::to_string)
                .unwrap_or_default();
            match key.as_str() {
                "min" => out.min = Some(number(&meta)?),
                "max" => out.max = Some(number(&meta)?),
                "step" => out.step = Some(number(&meta)?),
                "group" => {
                    let group: syn::LitStr = meta.value()?.parse()?;
                    out.group = Some(group.value());
                }
                "readonly" => out.readonly = flag(&meta)?,
                "skip" => out.skip = flag(&meta)?,
                _ => {
                    return Err(meta.error(
                        "unknown key: the panel reads min, max, step, group, readonly and skip",
                    ))
                }
            }
            Ok(())
        })?;
    }
    Ok(out)
}

/// `min = 0.5` or `min = 1`: one finite literal as an `f64`.
fn number(meta: &syn::meta::ParseNestedMeta<'_>) -> Result<f64> {
    let literal: Lit = meta.value()?.parse()?;
    match &literal {
        Lit::Float(value) => value.base10_parse::<f64>(),
        Lit::Int(value) => value.base10_parse::<f64>(),
        _ => Err(Error::new_spanned(
            literal,
            "`#[inspector(..)]` range values are number literals",
        )),
    }
}

/// `readonly` or `readonly = true`: a bare key means true.
fn flag(meta: &syn::meta::ParseNestedMeta<'_>) -> Result<bool> {
    if !meta.input.peek(syn::Token![=]) {
        return Ok(true);
    }
    let value: syn::LitBool = meta.value()?.parse()?;
    Ok(value.value)
}
