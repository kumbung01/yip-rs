use proc_macro::TokenStream;
use quote::quote;
use syn::{ItemFn, parse_macro_input, parse_quote};

/// Derives the crate's fixed-size, little-endian `crate::payload::Payload` codec.
/// Struct fields must implement Payload. Enums must be fieldless and repr(u8).
#[proc_macro_derive(Payload)]
pub fn derive_payload(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as syn::DeriveInput);
    expand_payload(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn expand_payload(input: syn::DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let name = &input.ident;
    let mut generics = input.generics.clone();
    let (size, encode, decode) = match &input.data {
        syn::Data::Struct(data) => {
            let mut sizes = Vec::new();
            let mut encoders = Vec::new();
            let mut decoders = Vec::new();
            for (index, field) in data.fields.iter().enumerate() {
                let ty = &field.ty;
                let member = field
                    .ident
                    .clone()
                    .map(syn::Member::Named)
                    .unwrap_or_else(|| syn::Member::Unnamed(index.into()));
                generics
                    .make_where_clause()
                    .predicates
                    .push(parse_quote!(#ty: crate::payload::Payload));
                sizes.push(quote!(<#ty as crate::payload::Payload>::SIZE));
                encoders.push(quote! {
                    let end = offset + <#ty as crate::payload::Payload>::SIZE;
                    <#ty as crate::payload::Payload>::encode(self.#member, &mut dst[offset..end]);
                    offset = end;
                });
                decoders.push(quote! {
                    #member: {
                        let end = offset + <#ty as crate::payload::Payload>::SIZE;
                        let value = <#ty as crate::payload::Payload>::decode(&src[offset..end])?;
                        offset = end;
                        value
                    }
                });
            }
            (
                quote!(0 #(+ #sizes)*),
                quote! {
                    let mut offset = 0;
                    #(#encoders)*
                    let _ = offset;
                },
                quote! {
                    let mut offset = 0;
                    let value = Self { #(#decoders),* };
                    let _ = offset;
                    Ok(value)
                },
            )
        }
        syn::Data::Enum(data) => {
            let mut repr_u8 = false;
            for attr in &input.attrs {
                if attr.path().is_ident("repr") {
                    let repr = attr.parse_args_with(
                        syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated,
                    )?;
                    repr_u8 |= repr.iter().any(|path| path.is_ident("u8"));
                }
            }
            if !repr_u8
                || data
                    .variants
                    .iter()
                    .any(|v| !matches!(v.fields, syn::Fields::Unit))
            {
                return Err(syn::Error::new_spanned(
                    name,
                    "Payload enums must be fieldless and #[repr(u8)]",
                ));
            }
            let variants = data.variants.iter().map(|v| &v.ident);
            (
                quote!(1),
                quote!(dst[0] = self as u8;),
                quote! {
                    match src[0] {
                        #(value if value == Self::#variants as u8 => Ok(Self::#variants),)*
                        _ => Err(()),
                    }
                },
            )
        }
        syn::Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                name,
                "Payload does not support unions",
            ));
        }
    };
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    Ok(quote! {
        impl #impl_generics crate::payload::Payload for #name #ty_generics #where_clause {
            const SIZE: usize = #size;

            fn encode(self, dst: &mut [u8]) {
                let dst = &mut dst[..<Self as crate::payload::Payload>::SIZE];
                #encode
            }

            fn decode(src: &[u8]) -> Result<Self, ()> {
                if src.len() != <Self as crate::payload::Payload>::SIZE {
                    return Err(());
                }
                #decode
            }
        }
    })
}

#[proc_macro_attribute]
pub fn measure_time(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut function = parse_macro_input!(item as ItemFn);

    if function.sig.asyncness.is_some() || function.sig.constness.is_some() {
        return syn::Error::new_spanned(
            &function.sig,
            "measure_time supports synchronous, non-const functions only",
        )
        .to_compile_error()
        .into();
    }

    let name = function.sig.ident.to_string();

    function.block.stmts.insert(
        0,
        parse_quote! {
            #[cfg(feature = "defmt")]
            let _yip_measure_guard = crate::cyccnt::MeasureGuard::new(#name);
        },
    );

    quote!(#function).into()
}

#[cfg(test)]
mod tests {
    use super::expand_payload;

    #[test]
    fn rejects_unsupported_enum_shapes_and_unions() {
        for input in [
            syn::parse_quote!(
                enum MissingRepr {
                    A,
                }
            ),
            syn::parse_quote!(
                #[repr(u16)]
                enum WrongRepr {
                    A,
                }
            ),
            syn::parse_quote!(
                #[repr(u8)]
                enum WithData {
                    A(u8),
                }
            ),
            syn::parse_quote!(union Unsupported { a: u32, b: u8 }),
        ] {
            assert!(expand_payload(input).is_err());
        }
    }
}
