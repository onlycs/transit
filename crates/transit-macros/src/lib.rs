use heck::ToSnakeCase;
use itertools::Itertools;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{
    FieldModifiers, Token,
    parse::{Parse, ParseStream, Parser},
    punctuated::Punctuated,
    spanned::Spanned,
    token::{Brace, Colon},
};

fn ident_of(ty: &syn::Type) -> &syn::Ident {
    let syn::Type::Path(syn::TypePath { path, .. }) = &ty else {
        panic!("cant parse this type {ty:?}")
    };

    let seg = path.segments.last().unwrap();

    if let syn::PathArguments::AngleBracketed(ab) = &seg.arguments {
        let arg = ab
            .args
            .iter()
            .filter_map(|a| match a {
                syn::GenericArgument::Type(ty) => Some(ty),
                _ => None,
            })
            .next_back()
            .unwrap();

        ident_of(arg)
    } else {
        &path.segments.last().unwrap().ident
    }
}

#[proc_macro_attribute]
pub fn record(
    attr: proc_macro::TokenStream,
    tokens: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let mut tokens = syn::parse_macro_input!(tokens as syn::ItemStruct);
    let ident = syn::parse_macro_input!(attr as Option<syn::Ident>);

    let dbg = if ident.is_some_and(|i| i == "no_debug") {
        quote! {}
    } else {
        quote! { #[derive(Debug)] }
    };

    let syn::Fields::Named(named) = &mut tokens.fields else {
        panic!("Expected named struct");
    };

    named.named.iter_mut().for_each(|field| {
        if let syn::Type::Path(pth) = &field.ty
            && pth.path.segments.last().unwrap().ident == "Option"
        {
            let attrs = syn::Attribute::parse_outer.parse2(quote! {
                #[cfg_attr(target_family = "wasm", tsify(optional))]
                #[cfg_attr(target_family = "wasm", serde(default, skip_serializing_if = "Option::is_none"))]
            }).unwrap();
            field.attrs.extend(attrs)
        }
    });

    quote! {
        #[derive(::bitcode::Decode, ::bitcode::Encode, Clone)]
        #dbg
        #[cfg_attr(target_family = "wasm", derive(::serde::Serialize, ::serde::Deserialize, ::tsify::Tsify))]
        #[cfg_attr(feature = "uniffi", derive(::uniffi::Record))]
        #tokens
    }
    .into()
}

#[proc_macro_attribute]
pub fn oneof(
    attr: proc_macro::TokenStream,
    tokens: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let item = syn::parse_macro_input!(tokens as syn::ItemEnum);
    let ident = syn::parse_macro_input!(attr as Option<syn::Ident>);

    let tagging = if item
        .variants
        .iter()
        .any(|variant| matches!(variant.fields, syn::Fields::Unnamed(_)))
    {
        quote! { serde(tag = "tag", content = "value") }
    } else {
        quote! { serde(tag = "tag") }
    };

    let dbg = if ident.is_some_and(|i| i == "no_debug") {
        quote! {}
    } else {
        quote! { #[derive(Debug)] }
    };

    quote! {
        #[derive(::bitcode::Decode, ::bitcode::Encode, Clone)]
        #dbg
        #[cfg_attr(target_family = "wasm", derive(::serde::Serialize, ::serde::Deserialize, ::tsify::Tsify))]
        #[cfg_attr(target_family = "wasm", #tagging)]
        #[cfg_attr(feature = "uniffi", derive(::uniffi::Enum))]
        #item
    }
    .into()
}

#[proc_macro_attribute]
pub fn error_shard(
    attr: proc_macro::TokenStream,
    tokens1: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let tokens = TokenStream::from(tokens1.clone());
    let attr = TokenStream::from(attr);

    let record = syn::parse_macro_input!(tokens1 as syn::ItemStruct);
    let ident = &record.ident;

    quote! {
        #[derive(::bitcode::Decode, ::bitcode::Encode, ::snafu::Snafu, Debug, Clone)]
        #[cfg_attr(target_family = "wasm", derive(::serde::Serialize, ::tsify::Tsify))]
        #[cfg_attr(feature = "uniffi", derive(::uniffi::Record))]
        #[snafu(visibility(pub))]
        #[snafu(display(#attr))]
        #tokens

        #[cfg(feature = "uniffi")]
        #[::uniffi::export]
        impl #ident {
            pub fn display(&self) -> String {
                format!("{self}")
            }

            pub fn report(&self) -> String {
                snafu::Report::from_error(self).to_string()
            }
        }

        impl #ident {
            pub fn fail<T>(self) -> Result<T, #ident> {
                Err(self)
            }
        }

        impl ::snafu::IntoError<#ident> for #ident {
            type Source = ::snafu::NoneError;

            fn into_error(self, source: Self::Source) -> #ident {
                self
            }
        }
    }
    .into()
}

#[proc_macro]
pub fn error(tokens1: proc_macro::TokenStream) -> proc_macro::TokenStream {
    pub struct ErrorShard {
        name: syn::Ident,
        fmt: syn::LitStr,
        fields: Option<syn::FieldsNamed>,
    }

    pub struct ErrorComposed {
        name: syn::Ident,
        sum: Punctuated<syn::Type, Token![|]>,
    }

    pub enum Error {
        Shard(ErrorShard),
        Composed(ErrorComposed),
    }

    impl Parse for Error {
        fn parse(input: ParseStream) -> syn::Result<Self> {
            let name = input.parse()?;

            if input.peek(Token![=]) {
                input.parse::<Token![=]>()?;
                let mut sum = Punctuated::new();

                sum.push_value(input.parse()?);
                while input.peek(Token![|]) {
                    sum.push_punct(input.parse()?);
                    sum.push_value(input.parse()?);
                }

                Ok(Self::Composed(ErrorComposed { name, sum }))
            } else {
                let inner;
                syn::parenthesized!(inner in input);

                let lit = inner.parse()?;

                let fields = if input.peek(syn::token::Brace) {
                    let mut f = input.parse::<syn::FieldsNamed>()?;

                    f.named
                        .iter_mut()
                        .for_each(|f| f.vis = syn::Visibility::Public(Token![pub](f.span())));

                    Some(f)
                } else {
                    None
                };

                Ok(Self::Shard(ErrorShard {
                    name,
                    fmt: lit,
                    fields,
                }))
            }
        }
    }

    pub struct Errors(Punctuated<Error, Token![;]>);

    impl Parse for Errors {
        fn parse(input: ParseStream) -> syn::Result<Self> {
            let it = Punctuated::parse_terminated(input)?;

            Ok(Self(it))
        }
    }

    let tokens = TokenStream::from(tokens1.clone());
    let errors = match syn::parse2::<Errors>(tokens) {
        Ok(valid) => valid,
        Err(err) => return err.into_compile_error().into(),
    };

    let (left, right) = errors
        .0
        .into_iter()
        .partition_map::<Vec<_>, Vec<_>, _, _, _>(|a| match a {
            Error::Composed(c) => itertools::Either::Left(c),
            Error::Shard(s) => itertools::Either::Right(s),
        });

    let shard_gen = right.into_iter().map(|ErrorShard { name, fmt, fields }| {
        let tail = fields
            .map(|f| quote! { #f })
            .unwrap_or_else(|| quote! { ; });

        quote! {
            #[::transit_core::error_shard(#fmt)]
            pub struct #name #tail
        }
    });

    let composed_gen = left.into_iter().map(|ErrorComposed { name, sum }| {
        let (lasts, tys) = sum
            .into_iter()
            .map(|ty| (ident_of(&ty).clone(), ty))
            .unzip::<_, _, Vec<_>, Vec<_>>();

        let variants = lasts.iter().zip(&tys).map(|(last, ty)| quote! {
            #[snafu(display("{source}"))]
            #last { source: #ty }
        });

        let impl_from = lasts.iter().zip(&tys).map(|(last, ty)| quote! {
            impl From<#ty> for #name {
                fn from(other: #ty) -> Self {
                    Self::#last { source: other }
                }
            }
        });

        let cast_ident = quote::format_ident!("{}Cast", name);
        let cast_private = quote::format_ident!("__{}__private", name);


        let meta_impl_from = quote! {
            #[allow(non_snake_case)]
            mod #cast_private {
                pub enum #cast_ident<S> {
                    #cast_ident,
                    _Phantom(::std::convert::Infallible, ::std::marker::PhantomData<fn(S)>)
                }
            }

            pub use #cast_private::#cast_ident::#cast_ident;

            impl<E: #(From<#tys>)+* + From<::transit_core::InternalError> + ::snafu::ErrorCompat + ::std::error::Error> ::snafu::IntoError<E> for #cast_private::#cast_ident<E> {
                type Source = #name;

                #[track_caller]
                fn into_error(self, source: #name) -> E {
                    source.cast::<E>()
                }
            }

            impl #name {
                pub fn cast<S>(self) -> S
                where
                    S: #(From<#tys>)+* + From<::transit_core::InternalError>
                {
                    match self {
                        #(
                            #name::#lasts { source } => <S as From<#tys>>::from(source),
                        )*
                        Self::InternalError { source } => <S as From<::transit_core::InternalError>>::from(source),
                    }
                }
            }
        };

        quote! {
            #[derive(::bitcode::Decode, ::bitcode::Encode, ::snafu::Snafu, Debug, Clone)]
            #[cfg_attr(target_family = "wasm", derive(::serde::Serialize, ::tsify::Tsify))]
            #[cfg_attr(target_family = "wasm", serde(tag = "tag"))]
            #[cfg_attr(feature = "uniffi", derive(::uniffi::Error))]
            #[snafu(visibility(pub))]
            #[snafu(module)]
            pub enum #name {
                #[snafu(display("{source}"))]
                InternalError { source: ::transit_core::InternalError },
                #(#variants),*
            }

            impl From<::transit_core::InternalError> for #name {
                fn from(other: ::transit_core::InternalError) -> Self {
                    Self::InternalError { source: other }
                }
            }

            #[cfg(feature = "uniffi")]
            #[::uniffi::export]
            impl #name {
                pub fn display(&self) -> String {
                    format!("{self}")
                }

                pub fn report(&self) -> String {
                    snafu::Report::from_error(self).to_string()
                }
            }

            impl ::snafu::FromString for #name {
                type Source = ::snafu::NoneError;

                fn with_source(_: Self::Source, message: String) -> Self {
                    Self::InternalError { source: ::transit_core::InternalError { message } }
                }

                fn without_source(message: String) -> Self {
                    Self::InternalError { source: ::transit_core::InternalError { message } }
                }
            }

            #(#impl_from)*

            #meta_impl_from
        }
    });

    quote! {
        #(#shard_gen)*
        #(#composed_gen)*
    }
    .into()
}

#[proc_macro]
pub fn route(tokens: proc_macro::TokenStream) -> proc_macro::TokenStream {
    struct RouteDefinition {
        name: syn::Ident,
        request: syn::Type,
        response_t: syn::Type,
        response_e: syn::Type,
    }

    impl Parse for RouteDefinition {
        fn parse(input: ParseStream) -> syn::Result<Self> {
            let name = input.parse()?;
            let inner;
            syn::parenthesized!(inner in input);
            let request = inner.parse()?;
            if !inner.is_empty() {
                return Err(inner.error("expected one request type"));
            }
            input.parse::<Token![->]>()?;

            let result_ident = input.parse::<syn::Ident>()?;
            if result_ident != "Result" {
                return Err(input.error("Expected `Result`"));
            }

            input.parse::<Token![<]>()?;
            let response_t = input.parse::<syn::Type>()?;
            input.parse::<Token![,]>()?;
            let response_e = input.parse::<syn::Type>()?;
            input.parse::<Token![>]>()?;

            Ok(Self {
                name,
                request,
                response_t,
                response_e,
            })
        }
    }

    struct Routes(Punctuated<RouteDefinition, Token![;]>);

    impl Parse for Routes {
        fn parse(input: ParseStream) -> syn::Result<Self> {
            Ok(Self(Punctuated::parse_terminated(input)?))
        }
    }

    let Routes(routes) = syn::parse_macro_input!(tokens as Routes);
    let routes = routes.into_iter().map(|RouteDefinition { name, request, response_t, response_e }| {
        let function = format_ident!("route_{}", name.to_string().to_snake_case());
        let result_type = format_ident!("{}Result", name);
        let error_type = format_ident!("{}FfiError", name);
        let js_return_type = result_type.to_string();

        let req_ts = quote!(#request).to_string();
        let promise_ts = format!("Promise<{js_return_type}>");

        quote! {
            pub struct #name;
            impl ::transit_core::Route for #name {
                const ID: ::transit_core::frame::RouteId = ::xxhash_rust::const_xxh3::xxh3_64(stringify!(#name).as_bytes());
                type Request = #request;
                type Response = Result<#response_t, #response_e>;
            }

            #[cfg(feature = "uniffi")]
            #[derive(Debug, ::uniffi::Error)]
            pub enum #error_type {
                Protocol { source: #response_e },
                Transit { source: ::transit_ffi::TransitError },
            }

            #[cfg(target_family = "wasm")]
            #[derive(Debug, ::serde::Serialize)]
            pub enum #error_type {
                Protocol { source: #response_e },
                Transit {
                    #[serde(serialize_with = "::transit_core::wbg_util::serialize_wrapper")]
                    source: ::transit_wasm::TransitError
                },
            }

            #[cfg(any(feature = "uniffi", target_family = "wasm"))]
            impl ::std::fmt::Display for #error_type {
                fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                    match self {
                        Self::Protocol { source } => ::std::fmt::Display::fmt(source, f),
                        Self::Transit { source } => ::std::fmt::Display::fmt(source, f),
                    }
                }
            }

            #[cfg(any(feature = "uniffi", target_family = "wasm"))]
            impl ::std::error::Error for #error_type {
                fn source(&self) -> Option<&(dyn ::std::error::Error + 'static)> {
                    match self {
                        Self::Protocol { source } => Some(source),
                        Self::Transit { source } => None,
                    }
                }
            }

            #[cfg(feature = "uniffi")]
            #[::uniffi::export]
            impl #error_type {
                pub fn display(&self) -> String {
                    format!("{self}")
                }

                pub fn report(&self) -> String {
                    match self {
                        Self::Protocol { source } => ::snafu::Report::from_error(&source).to_string(),
                        Self::Transit { source } => source.report.clone(),
                    }
                }
            }

            #[cfg(feature = "uniffi")]
            #[::uniffi::export]
            pub async fn #function(t: &::transit_ffi::Transit, req: #request) -> Result<#response_t, #error_type>
            {
                t.route::<#name>(req).await
                    .map_err(|e| #error_type::Transit { source: e })
                    .and_then(|r| r.map_err(|e| #error_type::Protocol { source: e }))
            }

            #[cfg(target_family = "wasm")]
            #[::tsify::declare]
            pub type #result_type = Result<#response_t, #error_type>;

            #[cfg(target_family = "wasm")]
            #[::wasm_bindgen::prelude::wasm_bindgen(unchecked_return_type = #promise_ts)]
            pub fn #function(
                t: &::transit_wasm::Transit,
                #[wasm_bindgen(unchecked_param_type = #req_ts)] req: ::wasm_bindgen::JsValue,
            ) -> ::js_sys::Promise {
                let client = t.client();
                ::wasm_bindgen_futures::future_to_promise(async move {
                    let req: #request = ::serde_wasm_bindgen::from_value(req)?;
                    let result: #result_type = client
                        .route::<#name>(req)
                        .await
                        .map_err(|e| #error_type::Transit { source: ::transit_wasm::TransitError::from_err(&e) })
                        .and_then(|r| r.map_err(|e| #error_type::Protocol { source: e }));
                    Ok(::serde_wasm_bindgen::to_value(&result)?)
                })
            }
        }
    });
    quote! { #(#routes)* }.into()
}

#[proc_macro_attribute]
pub fn core_error(
    _attr: proc_macro::TokenStream,
    item1: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let mut item = syn::parse_macro_input!(item1 as syn::ItemEnum);

    item.variants.iter_mut().for_each(|var| {
        let sp = var.span();

        if let syn::Fields::Unnamed(_) = var.fields {
            panic!("Only named fields")
        };

        if let syn::Fields::Unit = var.fields {
            var.fields = syn::Fields::Named(syn::FieldsNamed {
                brace_token: Brace(var.span()),
                named: Punctuated::new(),
            });
        }

        let syn::Fields::Named(named) = &mut var.fields else {
            panic!("what the fuck");
        };

        named.named.push(syn::Field {
            attrs: syn::Attribute::parse_outer
                .parse2(quote! {
                    #[cfg_attr(feature = "nightly", snafu(provide))]
                    #[snafu(implicit)]
                })
                .unwrap(),
            vis: syn::Visibility::Inherited,
            modifiers: FieldModifiers::default(),
            ident: Some(syn::Ident::new("location", sp)),
            ty: syn::parse2(quote!(::snafu::Location)).unwrap(),
            colon_token: Some(Colon(sp)),
            default: None,
        });
    });

    quote! {
        #[derive(Debug, ::snafu::Snafu)]
        #item
    }
    .into()
}

#[proc_macro_attribute]
pub fn constvar(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let item = syn::parse_macro_input!(item as syn::ItemConst);
    let cast = if attr.is_empty() {
        None
    } else {
        Some(syn::parse_macro_input!(attr as syn::ExprCast))
    };

    let ident = &item.ident;
    let snake = syn::Ident::new(&ident.to_string().to_snake_case(), ident.span());
    let ty = &item.ty;
    let docs = item.attrs.iter().filter(|a| a.path().is_ident("doc"));

    let (ret_ty, body) = if let Some(syn::ExprCast { expr, ty, .. }) = cast {
        (quote! { #ty }, quote! { #expr(#ident) })
    } else if let syn::Type::Reference(syn::TypeReference { elem, .. }) = &**ty
        && let syn::Type::Path(syn::TypePath { path, .. }) = &**elem
        && path.is_ident("str")
    {
        (
            quote! { ::std::string::String },
            quote! { ::std::string::ToString::to_string(#ident) },
        )
    } else {
        (quote! { #ty }, quote! { #ident })
    };

    quote! {
        #item

        #[cfg(any(feature = "uniffi", target_family = "wasm"))]
        #[cfg_attr(target_family = "wasm", ::wasm_bindgen::prelude::wasm_bindgen)]
        #[cfg_attr(feature = "uniffi", ::uniffi::export)]
        #(#docs)*
        pub fn #snake() -> #ret_ty {
            #body
        }
    }
    .into()
}
