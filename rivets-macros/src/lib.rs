#![feature(proc_macro_diagnostic)]

use anyhow::{bail, Result};
use proc_macro::{Diagnostic, Level, Span, TokenStream};
use quote::quote;
use syn::{parse_macro_input, Abi, FnArg, ItemFn};

fn failure(callback: proc_macro2::TokenStream, error_message: &str) -> TokenStream {
    Diagnostic::spanned(Span::call_site(), Level::Error, error_message).emit();
    callback.into()
}

fn determine_calling_convention(input: &ItemFn, unmangled_name: &str) -> Result<Abi> {
    if let Some(abi) = &input.sig.abi {
        let abi = quote! { #abi };
        bail!("Detour functions cannot specify an ABI. The ABI is automatically specified by rivets. You specifed: {abi}");
    } else {
        let abi = rivets_shared::get_calling_convention(unmangled_name);
        if let Some(abi) = abi {
            Ok(abi)
        } else {
            bail!("Failed to determine calling convention for {unmangled_name}. Please report this issue to the rivets developers.");
        }
    }
}

#[proc_macro_attribute]
pub fn detour(attr: TokenStream, item: TokenStream) -> TokenStream {
    let mangled_name = attr.to_string();
    let unmangled_name =
        rivets_shared::demangle(&mangled_name).unwrap_or_else(|| mangled_name.clone());

    let input = parse_macro_input!(item as ItemFn);
    let callback = quote! { #input };
    let name = &input.sig.ident;
    let return_type = &input.sig.output;

    let arguments: Vec<proc_macro2::TokenStream> = input
        .sig
        .inputs
        .iter()
        .map(|arg| match arg {
            FnArg::Receiver(_) => {
                quote! {compile_error!("Detour functions cannot use the self parameter.")}
            }
            FnArg::Typed(pat) => {
                let attrs = &pat.attrs;
                let ty = &pat.ty;
                quote! { #(#attrs)* #ty }
            }
        })
        .collect();
    let arguments = quote! { #( #arguments ),* };

    let calling_convention = match determine_calling_convention(&input, &unmangled_name) {
        Ok(abi) => abi,
        Err(e) => return failure(callback, &e.to_string()),
    };

    let cpp_function_header = quote! {
        unsafe #calling_convention fn(#arguments) #return_type
    };

    let result = quote! {
        #[doc = #unmangled_name]
        #callback

        unsafe fn hook(address: u64) -> anyhow::Result<()> {
            retour::static_detour! {
                static Detour: #cpp_function_header;
            }

            let compiled_function: #cpp_function_header = std::mem::transmute(address);
            Detour.initialize(compiled_function, #name)?.enable()?;
            Ok(())
        }

        #[ctor::ctor]
        fn ctor() {
            rivets::start_stream();
            if let Err(e) = rivets::inject(#mangled_name, hook) {
                tracing::error!("{e}");
            }
        }
    };

    Diagnostic::spanned(Span::call_site(), Level::Note, unmangled_name.clone()).emit();

    result.into()
}
