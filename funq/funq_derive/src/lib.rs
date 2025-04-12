use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{ToTokens, quote};
use syn::parse::{self, Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{
    Error, FnArg, GenericParam, Ident, ItemTrait, Pat, PatIdent, Path, Token, TraitItem,
    TraitItemFn, parse_macro_input,
};

/// Proc-macro arguments.
struct Args {
    args: Vec<Path>,
}

impl Args {
    /// Get n-th argument.
    fn get(&self, n: usize) -> Option<&Path> {
        self.args.get(n)
    }
}

impl Parse for Args {
    fn parse(input: ParseStream<'_>) -> parse::Result<Self> {
        let args = Punctuated::<Path, Token![,]>::parse_terminated(input)?;
        Ok(Self { args: args.into_iter().collect() })
    }
}

#[proc_macro_attribute]
pub fn callbacks(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as Args);
    let input = parse_macro_input!(item as ItemTrait);

    // Get ident of callback state.
    let state_path = match args.get(0) {
        Some(arg) => arg,
        None => panic!("missing state parameter\n\nUsage: #[funq_derive::callbacks(State)]"),
    };
    let state_path: Vec<_> = state_path.segments.iter().map(|segment| &segment.ident).collect();

    // Check if trait is thread-safe.
    let thread_safe = args
        .get(1)
        .and_then(|arg| arg.segments.first())
        .is_none_or(|segment| segment.ident != "thread_local");

    // Always generate thread-local bindings.
    let st_trait_impl = match trait_impl_tokens(&input, &state_path, false) {
        Ok(trait_impl) => trait_impl,
        Err(err) => return err.to_compile_error().into(),
    };

    let mut tokens = quote!(
        #input
        #st_trait_impl
    );

    // Only generate thread-safe bindings without `thread_local` attribute.
    if thread_safe {
        let mt_trait_impl = match trait_impl_tokens(&input, &state_path, true) {
            Ok(trait_impl) => trait_impl,
            Err(err) => return err.to_compile_error().into(),
        };
        tokens.extend(mt_trait_impl);
    };

    tokens.into()
}

/// Generate `impl X for State` tokens.
fn trait_impl_tokens(
    input: &ItemTrait,
    state_path: &[&Ident],
    thread_safe: bool,
) -> Result<TokenStream2, Error> {
    let state_ident = state_path[0];
    let trait_ident = &input.ident;
    let trait_where = &input.generics.where_clause;
    let trait_generics = &input.generics.params;

    // Extract idents from all the generic/lifetime/const params.
    let mut trait_generic_idents = TokenStream2::default();
    for param in &input.generics.params {
        let param_ident = match param {
            GenericParam::Lifetime(param) => param.lifetime.to_token_stream(),
            GenericParam::Const(param) => param.ident.to_token_stream(),
            GenericParam::Type(param) => param.ident.to_token_stream(),
        };
        trait_generic_idents.extend(quote!(#param_ident ,));
    }

    // Pick different queue handle for thread-local callbacks.
    let handle_tokens =
        if thread_safe { quote!(funq::MtQueueHandle) } else { quote!(funq::StQueueHandle) };

    let trait_fns = trait_fns_tokens(&input.items, state_path, trait_ident, thread_safe)?;

    Ok(quote!(
        impl<#trait_generics> #trait_ident<#trait_generic_idents> for #handle_tokens<#state_ident>
            #trait_where
        {
            #trait_fns
        }
    ))
}

/// Generate tokens for trait's functions.
fn trait_fns_tokens(
    items: &Vec<TraitItem>,
    state_path: &[&Ident],
    trait_ident: &Ident,
    thread_safe: bool,
) -> Result<TokenStream2, Error> {
    let mut trait_fns = TokenStream2::default();

    for item in items {
        let fun = match item {
            TraitItem::Fn(fun) => fun,
            _ => return Err(Error::new(item.span(), "only functions are supported in traits")),
        };

        trait_fns.extend(trait_fn_tokens(fun, state_path, trait_ident, thread_safe));
    }

    Ok(trait_fns)
}

/// Generate tokens for a function of the trait.
fn trait_fn_tokens(
    fun: &TraitItemFn,
    state_path: &[&Ident],
    trait_ident: &Ident,
    thread_safe: bool,
) -> Result<TokenStream2, Error> {
    // Ignore functions with default impls.
    if fun.default.is_some() {
        return Ok(TokenStream2::default());
    }

    // We need to define our own unique variable names, since traits are allowed to
    // have argument names which would shadow each other.
    let mut fun = fun.clone();
    let mut arg_idents = Vec::new();
    for (i, arg) in fun.sig.inputs.iter_mut().enumerate() {
        let arg = match arg {
            FnArg::Typed(arg) => arg,
            // Ignore `self` argument.
            FnArg::Receiver(_) => continue,
        };

        // Create unique argument name.
        let arg_name = format!("_arg{i}");
        let arg_ident = Ident::new(&arg_name, Span::call_site());

        arg_idents.push(arg_ident.clone());

        // Replace existing function argument.
        arg.pat = Box::new(Pat::Ident(PatIdent {
            ident: arg_ident,
            attrs: Vec::new(),
            mutability: None,
            subpat: None,
            by_ref: None,
        }));
    }

    // Calculate access path for sub-states.
    let mut state_access = if state_path.len() == 1 { quote!(state) } else { quote!(&mut state) };
    for ident in &state_path[1..] {
        state_access.extend(quote!(.#ident));
    }

    // Pick non-sync implementation for thread-local callbacks.
    let (any_tokens, cb_tokens) = if thread_safe {
        (quote!(dyn std::any::Any + Send + Sync), quote!(funq::MtFun))
    } else {
        (quote!(dyn std::any::Any), quote!(funq::StFun))
    };

    let state_ident = state_path[0];
    let fn_ident = &fun.sig.ident;
    let attrs = &fun.attrs;
    let sig = &fun.sig;

    // NOTE: We rely on type inference for downcasting the arguments at the moment.
    // This also means that generics are not supported. If this should be required
    // in the future, sending the type ID together with each argument might allow
    // for proper type reconstruction.
    Ok(quote!(
        #(#attrs)*
        #sig {
            let args: Vec<Box<#any_tokens>> = vec![ #( Box::new(#arg_idents) ),* ];

            let trampoline = |state: &mut #state_ident, _args: Vec<Box<#any_tokens>>| {
                let mut _args = _args.into_iter();
                #( let #arg_idents = _args.next().unwrap().downcast().unwrap();)*
                #trait_ident::#fn_ident(#state_access, #(*#arg_idents),*);
            };
            let fun = Box::new(trampoline);

            let event = #cb_tokens { fun, args };
            let _ = self.send(event);
        }
    ))
}
