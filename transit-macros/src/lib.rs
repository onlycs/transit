use heck::ToSnakeCase;
use itertools::Itertools;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{
    Token,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
};

#[proc_macro_attribute]
pub fn record(
    _attr: proc_macro::TokenStream,
    tokens: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let tokens = TokenStream::from(tokens);

    quote! {
        #[derive(::bitcode::Decode, ::bitcode::Encode, Clone)]
        #[cfg_attr(target_family = "wasm", derive(::serde::Serialize, ::serde::Deserialize, ::tsify::Tsify))]
        #[cfg_attr(feature = "uniffi", derive(::uniffi::Record))]
        #tokens
    }
    .into()
}

#[proc_macro_attribute]
pub fn oneof(
    _attr: proc_macro::TokenStream,
    tokens: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let item = syn::parse_macro_input!(tokens as syn::ItemEnum);

    let tagging = if item
        .variants
        .iter()
        .any(|variant| matches!(variant.fields, syn::Fields::Unnamed(_)))
    {
        quote! { serde(tag = "tag", content = "value") }
    } else {
        quote! { serde(tag = "tag") }
    };

    quote! {
        #[derive(::bitcode::Decode, ::bitcode::Encode, Clone)]
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

    let ident_display = syn::Ident::new(
        &format!("{}_display", ident.to_string().to_snake_case()),
        ident.span(),
    );

    let ident_report = syn::Ident::new(
        &format!("{}_report", ident.to_string().to_snake_case()),
        ident.span(),
    );

    quote! {
        #[derive(::bitcode::Decode, ::bitcode::Encode, ::snafu::Snafu, Debug, Clone)]
        #[cfg_attr(target_family = "wasm", derive(::serde::Serialize, ::serde::Deserialize, ::tsify::Tsify))]
        #[cfg_attr(feature = "uniffi", derive(::uniffi::Record))]
        #[snafu(visibility(pub))]
        #[snafu(display(#attr))]
        #tokens

        #[cfg(feature = "uniffi")]
        #[::uniffi::export]
        pub fn #ident_display (error: #ident) -> String {
            format!("{error}")
        }

        #[cfg(feature = "uniffi")]
        #[::uniffi::export]
        pub fn #ident_report (error: #ident) -> String {
            snafu::Report::from_error(&error).to_string()
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
                    Some(input.parse()?)
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
        let (variants, impl_from) = sum
            .into_iter()
            .map(|ty| {
                let syn::Type::Path(syn::TypePath { path, .. }) = &ty else {
                    panic!("cant parse this type {ty:?}")
                };

                let last = &path.segments.last().unwrap().ident;

                (
                    quote! {
                        #[snafu(display("{source}"))]
                        #last { source: #ty }
                    },
                    quote! {
                        impl From<#ty> for #name {
                            fn from(other: #ty) -> Self {
                                Self::#last { source: other }
                            }
                        }
                    },
                )
            })
            .unzip::<_, _, Vec<_>, Vec<_>>();

        quote! {
            #[derive(::bitcode::Decode, ::bitcode::Encode, ::snafu::Snafu, Debug, Clone)]
            #[cfg_attr(target_family = "wasm", derive(::serde::Serialize, ::serde::Deserialize, ::tsify::Tsify))]
            #[cfg_attr(target_family = "wasm", serde(tag = "tag"))]
            #[cfg_attr(feature = "uniffi", derive(::uniffi::Error))]
            #[snafu(visibility(pub))]
            #[snafu(module)]
            pub enum #name {
                #[snafu(display("Internal server error"))]
                InternalError { source: transit_core::InternalError },
                #(#variants),*
            }

            impl From<::transit_core::InternalError> for #name {
                fn from(other: ::transit_core::InternalError) -> Self {
                    Self::InternalError { source: other }
                }
            }

            #(#impl_from)*
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
        let error_binding_type = format_ident!("_{}FfiErrorAlias", name);
        let js_return_type = result_type.to_string();

        quote! {
            pub struct #name;
            impl ::transit_core::Route for #name {
                const ID: ::transit_core::frame::RouteId = ::xxhash_rust::const_xxh3::xxh3_64(stringify!(#name).as_bytes());
                type Request = #request;
                type Response = Result<#response_t, #response_e>;
            }

            #[cfg(feature = "uniffi")]
            type #error_binding_type = ::std::sync::Arc<::transit_core::client::RouteError>;
            #[cfg(target_family = "wasm")]
            type #error_binding_type = ::std::cell::RefCell<Option<::transit_core::client::RouteErrorWrapped>>;

            #[cfg(any(feature = "uniffi", target_family = "wasm"))]
            #[derive(Debug)]
            #[cfg_attr(feature = "uniffi", derive(::uniffi::Error))]
            #[cfg_attr(target_family = "wasm", derive(::serde::Serialize))]
            pub enum #error_type {
                Protocol(#response_e),
                Route(
                    #[cfg_attr(target_family = "wasm", serde(serialize_with = "::transit_core::wbg_util::serialize_wrapper"))]
                    #error_binding_type
                ),
            }

            #[cfg(feature = "uniffi")]
            impl ::std::fmt::Display for #error_type {
                fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                    match self {
                        Self::Protocol(e) => ::std::fmt::Display::fmt(e, f),
                        Self::Route(e) => ::std::fmt::Display::fmt(e, f),
                    }
                }
            }

            #[cfg(feature = "uniffi")]
            impl ::std::error::Error for #error_type {
                fn source(&self) -> Option<&(dyn ::std::error::Error + 'static)> {
                    match self {
                        Self::Protocol(e) => Some(e),
                        Self::Route(e) => Some(e),
                    }
                }
            }

            #[cfg(feature = "uniffi")]
            #[::uniffi::export]
            pub async fn #function(t: &::transit_core::client::Transit, req: #request) -> Result<#response_t, #error_type>
            {
                t.route::<#name>(req).await
                    .map_err(|e| #error_type::Route(::std::sync::Arc::new(e)))
                    .and_then(|r| r.map_err(|e| #error_type::Protocol(e)))
            }

            #[cfg(target_family = "wasm")]
            #[::tsify::declare]
            pub type #result_type = Result<#response_t, #error_type>;

            #[cfg(target_family = "wasm")]
            #[::wasm_bindgen::prelude::wasm_bindgen(unchecked_return_type = #js_return_type)]
            pub async fn #function(t: &::transit_core::client::Transit, req: ::tsify::Ts<#request>) -> Result<::wasm_bindgen::JsValue, ::serde_wasm_bindgen::Error>
            {
                let result = match req.to_rust() {
                    Ok(req) => t.route::<#name>(req).await,
                    Err(error) => Err(::transit_core::client::RouteError::InvalidRequest {
                        message: error.to_string(),
                    }),
                };

                let result = result
                    .map_err(|e| #error_type::Route(::std::cell::RefCell::new(Some(::transit_core::client::RouteErrorWrapped::wrap(e)))))
                    .and_then(|r| r.map_err(|e| #error_type::Protocol(e)));

                ::serde_wasm_bindgen::to_value(&result)
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
    let item = syn::parse_macro_input!(item1 as syn::ItemEnum);
    let ident = &item.ident;
    let ident_tag = quote::format_ident!("{ident}Tag");
    let ident_wrap = quote::format_ident!("{ident}Wrapped");

    quote! {
        #[derive(Debug, ::snafu::Snafu, ::strum::EnumDiscriminants)]
        #[cfg_attr(feature = "uniffi", derive(::uniffi::Object))]
        #[cfg_attr(target_family = "wasm", strum_discriminants(::wasm_bindgen::prelude::wasm_bindgen))]
        #[cfg_attr(feature = "uniffi", strum_discriminants(derive(::uniffi::Object)))]
        #[strum_discriminants(name(#ident_tag))]
        #item

        #[cfg(feature = "uniffi")]
        #[::uniffi::export]
        impl #ident {
            pub fn tag(&self) -> #ident_tag {
                ::strum::IntoDiscriminant::discriminant(self)
            }

            pub fn report(&self) -> String {
                ::snafu::Report::from_error(&self).to_string()
            }
        }

        #[cfg(target_family = "wasm")]
        #[derive(Debug)]
        #[::wasm_bindgen::prelude::wasm_bindgen]
        pub struct #ident_wrap {
            error: #ident,
            #[wasm_bindgen(getter)]
            tag: #ident_tag,
        }

        #[cfg(target_family = "wasm")]
        impl #ident_wrap {
            pub fn wrap(error: #ident) -> Self {
                Self {
                    tag: ::strum::IntoDiscriminant::discriminant(&error),
                    error,
                }
            }
        }

        #[cfg(target_family = "wasm")]
        #[::wasm_bindgen::prelude::wasm_bindgen]
        impl #ident_wrap {
            #[::wasm_bindgen::prelude::wasm_bindgen(getter)]
            pub fn report(&self) -> String {
                ::snafu::Report::from_error(&self.error).to_string()
            }

            #[::wasm_bindgen::prelude::wasm_bindgen(getter)]
            pub fn message(&self) -> String {
                format!("{}", self.error)
            }

            #[::wasm_bindgen::prelude::wasm_bindgen(unchecked_return_type = "never")]
            pub fn raise(&self) -> JsValue {
                ::wasm_bindgen::throw_str(&::snafu::Report::from_error(&self.error).to_string());
            }
        }
    }.into()
}
