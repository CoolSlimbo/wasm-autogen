use anyhow::{Context, Result};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use swc_ecma_ast::*;

pub(crate) fn map_class(class: &ClassDecl) -> Result<Option<TokenStream>> {
    let name = format_ident!("{}", class.ident.sym.as_str());
    tracing::trace!("Mapping class: {:?}", &name);

    // We use qoute! to generate the code for this
    // First, we create the type
    let exported_type = quote! {
        #[wasm_bindgen]
        pub type #name;
    };

    // Then, we have to handle the body part to it
    let mut methods = Vec::new();

    // We handle all the members in the class
    for members in &class.class.body {
        methods.push(match members {
            ClassMember::Constructor(constructor) => {
                let params = &constructor
                    .params
                    .iter()
                    .map(|param| match param {
                        ParamOrTsParamProp::Param(param) => match &param.pat {
                            Pat::Ident(BindingIdent { id, type_ann }) => {
                                let type_ann =
                                    type_ann.clone().context("Missing type annotation").unwrap();
                                let type_ann = &type_ann.type_ann;
                                let id = &id.sym.as_str();
                                Some((format_ident!("{}", *id), type_ann.clone()))
                            }
                            _ => None,
                        },
                        ParamOrTsParamProp::TsParamProp(_param) => None,
                    })
                    .collect::<Vec<_>>();

                // I early exited the match hell, but am gonna remap it again, just because it was getting to deep
                let params = params
                    .iter()
                    .map(|val| {
                        if let Some((id, type_ann)) = val {
                            match type_ann.as_ref() {
                                TsType::TsKeywordType(TsKeywordType { span: _, kind }) => {
                                    // match kind {
                                    //     TsKeywordTypeKind::TsNumberKeyword => {
                                    //         quote! { #id: f64 }
                                    //     }
                                    //     _ => TokenStream::new(),
                                    // }
                                    let kind = map_types(*kind);
                                    quote! { #id: #kind }
                                }
                                _ => TokenStream::new(),
                            }
                        } else {
                            TokenStream::new()
                        }
                    })
                    .collect::<Vec<_>>();

                // We build the full method now
                quote! {
                    #[wasm_bindgen(constructor)]
                    pub fn new(#(#params),*) -> #name;
                }
            }
            ClassMember::ClassProp(class_prop) => {
                let key = &class_prop.key;
                let key = match key {
                    PropName::Ident(Ident {
                        span,
                        sym,
                        optional,
                    }) => {
                        let sym = sym.as_str();
                        format_ident!("{}", sym)
                    }
                    _ => format_ident!(""),
                };

                let value = &class_prop.value;

                TokenStream::new()
            }
            _ => {
                //todo!("Class member")
                TokenStream::new()
            }
        });
    }

    // return Ok(Some(TokenStream::new()));

    Ok(Some(quote! {
        #exported_type
        #(#methods)*
    }))
}

fn map_types(tstype: TsKeywordTypeKind) -> TokenStream {
    match tstype {
        TsKeywordTypeKind::TsNumberKeyword => quote! { f64 },
        TsKeywordTypeKind::TsStringKeyword => quote! { String },
        TsKeywordTypeKind::TsBooleanKeyword => quote! { bool },
        TsKeywordTypeKind::TsVoidKeyword => quote! { () },
        _ => quote! { JsValue },
    }
}
