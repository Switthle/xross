use proc_macro::TokenStream;
use quote::{quote,format_ident};
use syn::{parse_macro_input, LitInt, LitStr, ItemStruct, Type};

struct FieldAttr {
    name: syn::Ident,
    ty: syn::Type,
    addr: String
}

impl FieldAttr {
    fn to_field(&self) -> proc_macro2::TokenStream {
        let name = &self.name;
        let ty = &self.ty;
        let addr = &self.addr;
        quote!{
            pub #name: ::xrossd_core::field::Field<#addr,#ty>
        }
    }

    fn to_proto_field(&self) -> proc_macro2::TokenStream {
        let name = &self.name;
        let ty = &self.ty;
        let addr = &self.addr;
        quote!{
            pub #name: ::xrossd_core::field::ProtoField<#addr,#ty>
        }
    }

    fn field_function(&self) -> proc_macro2::TokenStream {
        let name = &self.name;
        let ty = &self.ty;

        match ty {
            Type::Path(tp) if tp.path.is_ident("bool") => {
                let set_fn = format_ident!("{}{}", "set_", name);
                let toggle_fn = format_ident!("{}{}", "toggle_", name);
                quote!{
                    pub async fn #set_fn(&self, socket: &::tokio::net::UdpSocket, val: bool) -> ::anyhow::Result<()> {
                        self.#name.send(socket, val, "").await
                    }

                    pub async fn #toggle_fn(&self, socket: &::tokio::net::UdpSocket) -> ::anyhow::Result<()> {
                        let val = !self.#name.val;
                        self.#name.send(socket, val, "").await
                    }
                }
            },
            Type::Path(tp) if tp.path.is_ident("f32") => {
                let set_fn = format_ident!("{}{}", "set_", name);
                let inc_fn = format_ident!("{}{}", "inc_", name);
                quote!{
                    pub async fn #set_fn(&self, socket: &::tokio::net::UdpSocket, val: f32) -> ::anyhow::Result<()> {
                        self.#name.send(socket, val, "").await
                    }

                    pub async fn #inc_fn(&self, socket: &::tokio::net::UdpSocket, inc: f32) -> ::anyhow::Result<()> {
                        let val = self.#name.val + inc;
                        self.#name.send(socket, val, "").await
                    }
                }
            },
            _ => quote! { },
        }
    }

    fn chan_function(&self) -> proc_macro2::TokenStream {
        let name = &self.name;
        let ty = &self.ty;

        match ty {
            Type::Path(tp) if tp.path.is_ident("bool") => {
                let set_fn = format_ident!("{}{}", "set_chan_", name);
                let toggle_fn = format_ident!("{}{}", "toggle_chan_", name);
                quote!{
                    pub async fn #set_fn(&self, socket: &::tokio::net::UdpSocket, chan: usize, val: bool) -> ::anyhow::Result<()> {
                        let prefix = format!("/ch/{:02}", chan);
                        let channel = 
                            ::anyhow::Context::context(
                                self.channels.get(chan),
                                "Wrong channel"
                            )?;
                        channel.#name.send(socket, val, &prefix).await
                    }

                    pub async fn #toggle_fn(&self, socket: &::tokio::net::UdpSocket, chan: usize) -> ::anyhow::Result<()> {
                        let prefix = format!("/ch/{:02}", chan);
                        let channel = 
                            ::anyhow::Context::context(
                                self.channels.get(chan),
                                "Wrong channel"
                            )?;
                        let val = !channel.#name.val;
                        channel.#name.send(socket, val, &prefix).await
                    }
                }
            },
            Type::Path(tp) if tp.path.is_ident("f32") => {
                let set_fn = format_ident!("{}{}", "set_chan_", name);
                let inc_fn = format_ident!("{}{}", "inc_chan_", name);
                quote!{
                    pub async fn #set_fn(&self, socket: &::tokio::net::UdpSocket, chan: usize, val: f32) -> ::anyhow::Result<()> {
                        let prefix = format!("/ch/{:02}", chan);
                        let channel = 
                            ::anyhow::Context::context(
                                self.channels.get(chan),
                                "Wrong channel"
                            )?;
                        channel.#name.send(socket, val, &prefix).await
                    }
                    pub async fn #inc_fn(&self, socket: &::tokio::net::UdpSocket, chan: usize, inc: f32) -> ::anyhow::Result<()> {
                        let prefix = format!("/ch/{:02}", chan);
                        let channel = 
                            ::anyhow::Context::context(
                                self.channels.get(chan),
                                "Wrong channel"
                            )?;
                        let val = channel.#name.val + inc;
                        channel.#name.send(socket, val, &prefix).await
                    }
                }
            },
            _ => quote! { },
        }
    }

    fn to_match_arm(&self, var: &str, method: &str) -> proc_macro2::TokenStream {
        let name = &self.name;
        let addr = &self.addr;
        let var = format_ident!("{}", var);
        let method = format_ident!("{}", method);
        quote!{
            #addr => #var.#name.#method(val)
        }
    }

    fn init_field(&self) -> proc_macro2::TokenStream {
        let addr = &self.addr;
        let name = &self.name;
        quote!{
            if self.#name.val == None {
                init = false;
                let _ = ::xrossd_core::field::ask_and_wait(socket, #addr, Some(duration)).await;
            }
        }
    }

    fn init_chan_field(&self, num_chans :usize,  prefix: &str) ->
        proc_macro2::TokenStream {
        let mut exprs = Vec::with_capacity(num_chans);
        for chan in 0..num_chans {
            let addr = format!("{}/{:02}{}", prefix, chan+1, &self.addr);
            let name = &self.name;
            exprs.push(quote! {
                if self.channels[#chan].#name.val == None {
                    init = false;
                    let _ = ::xrossd_core::field::ask_and_wait(socket, #addr, Some(duration)).await;
                }
            })
        }
        quote!{
            #(#exprs;)*
        }
    }
}

#[proc_macro_attribute]
pub fn osc_state(attr: TokenStream, input: TokenStream) -> TokenStream {
    let mut num_chans: Option<usize> = None;
    let mut chan_prefix: Option<String> = None;

    // Use syn's built-in parser for "key = value" or "key(value)" pairs
    let attr_parser = syn::meta::parser(|meta: syn::meta::ParseNestedMeta| {
        if meta.path.is_ident("num_chans") {
            let value = meta.value()?; 
            let lit: LitInt = value.parse()?;
            num_chans = Some(lit.base10_parse()?);
            Ok(())
        } else if meta.path.is_ident("chan_prefix") {
            let value = meta.value()?;
            let lit: LitStr = value.parse()?;
            chan_prefix = Some(lit.value());
            Ok(())
        } else {
            Err(meta.error("unsupported attribute argument"))
        }
    });

    parse_macro_input!(attr with attr_parser);
    let num_chans = num_chans.unwrap();
    let chan_prefix = chan_prefix.unwrap();

    let mut item_struct = parse_macro_input!(input as ItemStruct);
    let struct_name = &item_struct.ident;
    let mut field_attrs = Vec::new();
    let mut chan_field_attrs = Vec::new();

    if let syn::Fields::Named(fields) = &mut item_struct.fields {
        for field in &mut fields.named {
            let name = field.ident.clone().unwrap();
            let ty = field.ty.clone();

            // 1. Find and extract the #[address("...")] string
            let mut addr = String::new();
            let mut per_chan = false;
            field.attrs.retain(|attr| {
                if attr.path().is_ident("address") {
                    let lit: LitStr = attr.parse_args().expect("address must be a string");
                    addr = lit.value();
                    false // Remove this attribute from the final struct
                } else if attr.path().is_ident("per_chan") {
                    per_chan = true;
                    false
                } else {
                    true
                }
            });

            if per_chan {
                chan_field_attrs.push(FieldAttr{ name, ty, addr });
            } else {
                field_attrs.push(FieldAttr{ name,  ty, addr });
            }
        }
    }

    let fields: Vec<_> = field_attrs.iter().map(|f| f.to_field()).collect();
    let protofields: Vec<_> = field_attrs.iter().map(|f| f.to_proto_field()).collect();
    let chan_fields: Vec<_> = chan_field_attrs.iter().map(|f| f.to_field()).collect();
    let chan_protofields: Vec<_> = chan_field_attrs.iter().map(|f| f.to_proto_field()).collect();
    let ready_name = format_ident!("{}Ready", struct_name);
    let init_name = format_ident!("{}Init", struct_name);
    let channel_ready_name = format_ident!("{}ChannelReady", struct_name);
    let channel_init_name = format_ident!("{}ChannelInit", struct_name);
    let match_arms_set:Vec<_> =
        field_attrs.iter().map(|f| f.to_match_arm("self", "set_osc")).collect();
    let match_arms_chan_set:Vec<_> =
        chan_field_attrs.iter().map(|f| f.to_match_arm("chan", "set_osc")).collect();
    let match_arms_update:Vec<_> =
        field_attrs.iter().map(|f| f.to_match_arm("self", "update_osc")).collect();
    let match_arms_chan_update:Vec<_> =
        chan_field_attrs.iter().map(|f| f.to_match_arm("chan", "update_osc")).collect();

    let field_names: Vec<_> =
        field_attrs.iter().map(|f| f.name.clone()).collect();
    let field_chan_names: Vec<_> =
        chan_field_attrs.iter().map(|f| f.name.clone()).collect();


    let try_inits: Vec<_> = 
        field_attrs.iter()
        .map(|f| f.init_field())
        .collect();

    let try_chan_inits: Vec<_> = 
        chan_field_attrs.iter()
        .map(|f| f.init_chan_field(num_chans, &chan_prefix))
        .collect();

    let field_funcs: Vec<_> =
        field_attrs.iter()
        .map(|f| f.field_function())
        .collect();

    let chan_funcs: Vec<_> =
        chan_field_attrs.iter()
        .map(|f| f.chan_function())
        .collect();

    quote! {
        #[derive(Debug)]
        struct #channel_ready_name {
            #(#chan_fields,)*
        }
        #[derive(Debug,Default)]
        struct #channel_init_name {
            #(#chan_protofields,)*
        }
        impl #channel_init_name {
            pub fn try_to_channel(self) -> Option<#channel_ready_name> {
                Some(
                    #channel_ready_name {
                        #(#field_chan_names: self.#field_chan_names.try_to_field().ok()?,)*
                    }
                )
            }
        }

        #[derive(Debug)]
        pub struct #ready_name {
            #(#fields,)*
            channels: [#channel_ready_name;#num_chans]
        }
        #[derive(Debug,Default)]
        pub struct #init_name {
            #(#protofields,)*
            channels: [#channel_init_name;#num_chans]
        }
        #[derive(Debug)]
        pub enum #struct_name {
            Disconnected,
            Initializing(#init_name),
            Ready(#ready_name),
        }

        impl #ready_name {
            #(#field_funcs)*
            #(#chan_funcs)*

            pub fn update_osc(&mut self, addr: &str, val: ::rosc::OscType) -> ::anyhow::Result<()> {
                match addr {
                    s if s.starts_with(#chan_prefix) => {
                        let tail = s.strip_prefix(#chan_prefix).unwrap();
                        let (chan_no, end_addr) =
                            ::anyhow::Context::context(
                                ::sscanf::sscanf!(tail, "/{usize}{String}"),
                                "failed to parse address"
                            )?;
                        let mut chan = 
                            ::anyhow::Context::context(
                                self.channels.get_mut(chan_no-1),
                                "wrong channel number in update_osc"
                            )?;
                        match end_addr.as_str() {
                            #(#match_arms_chan_update,)*
                            _ => Ok(())
                        }
                    },
                    #(#match_arms_update,)*
                    _ => Ok(())
                }
            }
        }

        impl #init_name {
            pub async fn try_init(self, socket: &::tokio::net::UdpSocket, duration: ::tokio::time::Duration) -> Result<#ready_name,#init_name> {
                let mut init = true;
                #(#try_inits;)*
                #(#try_chan_inits;)*
                if init {
                    let channels =
                        self.channels.try_map(|x| x.try_to_channel()).unwrap();
                    Ok(#ready_name {
                        channels,
                        #(#field_names: self.#field_names.try_to_field().unwrap(),)*
                    })
                } else {
                    Err(self)
                }
            }

            pub fn set_osc(&mut self, addr: &str, val: ::rosc::OscType) -> ::anyhow::Result<()> {
                match addr {
                    s if s.starts_with(#chan_prefix) => {
                        let tail = s.strip_prefix(#chan_prefix).unwrap();
                        let (chan_no, end_addr) = ::sscanf::sscanf!(tail, "/{usize}{String}").unwrap();
                        let mut chan =
                            ::anyhow::Context::context(
                                self.channels.get_mut(chan_no-1),
                                "wrong channel number in update_osc"
                            )?;
                        match end_addr.as_str() {
                            #(#match_arms_chan_set,)*
                            _ => Ok(())
                        }
                    },
                    #(#match_arms_set,)*
                    _ => Ok(())
                }
            }
        }
    }.into()
}
