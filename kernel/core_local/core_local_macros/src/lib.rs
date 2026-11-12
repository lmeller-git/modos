use proc_macro::{Span, TokenStream};
use quote::quote;
use syn::{
    Attribute,
    Expr,
    Ident,
    Token,
    Type,
    Visibility,
    parse::{Parse, ParseStream},
    parse_macro_input,
};

struct CpuLocal {
    attributes: Vec<Attribute>,
    visibility: Visibility,
    name: Ident,
    ty: Type,
    init: Expr,
}

impl Parse for CpuLocal {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let attributes = input.call(Attribute::parse_outer)?;
        let visibility: Visibility = input.parse()?;
        input.parse::<Token![static]>()?;
        let name: Ident = input.parse()?;
        input.parse::<Token![:]>()?;
        let ty: Type = input.parse()?;
        input.parse::<Token![=]>()?;
        let init: Expr = input.parse()?;
        input.parse::<Token![;]>()?;
        Ok(CpuLocal {
            attributes,
            visibility,
            name,
            ty,
            init,
        })
    }
}

#[proc_macro_attribute]
pub fn core_local(args: TokenStream, input: TokenStream) -> TokenStream {
    let CpuLocal {
        attributes,
        visibility,
        name,
        ty,
        init,
    } = parse_macro_input!(input as CpuLocal);

    let attributes = attributes.iter().map(|attribute| quote! { #attribute });

    let struct_name = Ident::new(&format!("CPU_LOCAL_{name}"), Span::call_site().into());

    // TODO
    //
    // the static gets converted to some struct with accessor methods for the raw value. At init, the actual data behind the static gets copied to each cpu local storage section per core.
    // The one global static which is used to access the data sill not be copied, it should calcualte the correct offset from my_cpu() of the underlying core local data anr return a ptr to it.
    // again the considerations in core_local/lib.rs apply here.
    //
    // core idea:
    //
    // we store the actual static payload in a private static in the cpu_local link_section. At core_local init this will be copied to each core_local storage section.
    // We also store a static CoreLocal<T> var with the original name, which is used to access the statics copied to the core_local sections.
    // For this the CoreLocal stored in the static needs to compute the offset of the static payload data relative to the link section start. This can be computed from the addr of the private static raw data.

    quote! {

        #[repr(transparent)]
        #[doc(hidden)]
        struct #struct_name {
            __inner: #ty,
        }

        impl #struct_name {}

        #(#attributes)*
        #[unsafe(link_section = ".cpu_local")]
        #[used]
        static #name: #struct_name = #struct_name {
            __inner: #init
        };
    }
    .into()
}
